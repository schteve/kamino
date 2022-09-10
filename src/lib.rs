#![doc = include_str!("../README.md")]
#![deny(missing_docs)]
#![deny(unsafe_code)]

use git2::{
    Branch, BranchType, Config, Cred, CredentialType, FetchOptions, Oid, RemoteCallbacks,
    Repository, StatusOptions,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
};

/// Error type for `check_uncommitted()`
#[derive(thiserror::Error, Debug)]
#[error("failed getting repo status for {path}")]
pub struct UncommittedError {
    /// Path to the repo
    path: PathBuf,
    /// Underlying error
    source: git2::Error,
}

/// Check if there are any uncommitted local changes
///
/// # Errors
///
/// See `UncommittedError`
pub fn check_uncommitted(repo: &Repository) -> Result<bool, UncommittedError> {
    let mut status_opts = StatusOptions::new();
    status_opts.include_ignored(false).include_untracked(true);

    let statuses = repo
        .statuses(Some(&mut status_opts))
        .map_err(|e| UncommittedError {
            path: repo.path().to_owned(),
            source: e,
        })?;
    Ok(!statuses.is_empty())
}

/// Error type for `check_stashed()`
#[derive(thiserror::Error, Debug)]
#[error("failed to check the stash")]
pub struct StashedError(#[source] git2::Error);

/// Check if there are any stashed changes.
///
/// # Errors
///
/// Return `Error::Stash` if any of the stash queries fail.
pub fn check_stashed(repo: &mut Repository) -> Result<u32, StashedError> {
    let mut stash_count = 0;

    let cb = |_index: usize, _msg: &str, _id: &Oid| -> bool {
        stash_count += 1;
        true
    };
    repo.stash_foreach(cb).map_err(StashedError)?;

    Ok(stash_count)
}

/// Contains details about the state of a branch relative to the remote server.
pub struct AheadBehind {
    /// The number of commits this branch is ahead of the remote server, or None if no upstream branch is detected.
    pub ahead: Option<usize>,
    /// The number of commits this branch is behind the remote server, or None if no upstream branch is detected.
    pub behind: Option<usize>,
    /// The name of this branch, or None if the name would not be a valid String (not UTF-8 for example).
    pub branch_name: Option<String>,
    /// The name of the upstream branch, or None if it is not detected.
    pub upstream_name: Option<String>,
}

/// Error type for `check_ahead_behind()`
#[derive(thiserror::Error, Debug)]
#[error("failed to fetch origin")]
pub struct AheadBehindError(#[source] git2::Error);

/// Error type for the iterator returned from `check_ahead_behind()`
#[derive(thiserror::Error, Debug)]
pub enum AheadBehindIterError {
    /// Failed to get OID of a branch
    #[error("failed to get OID of branch {0}")]
    Oid(String),

    /// Failed to check the commit graph
    #[error("Error while checking graph ahead/behind")]
    CommitGraph(#[source] git2::Error),
}

/// Check if local is ahead or behind remote
/// Fetch from origin first to make sure upstream is accurate.
///
/// # Errors
///
/// See `AheadBehindError`
pub fn check_ahead_behind<'a>(
    repo: &'a Repository,
    remote: &str,
) -> Result<impl Iterator<Item = Result<AheadBehind, AheadBehindIterError>> + 'a, AheadBehindError>
{
    if let Ok(mut remote) = repo.find_remote(remote) {
        let refspecs: &[&str] = &[]; // Use base refspecs, which I assume means all local branches
        let mut cbs = RemoteCallbacks::new();
        cbs.credentials(git_cred_check);
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(cbs);
        remote
            .fetch(refspecs, Some(&mut opts), None)
            .map_err(AheadBehindError)?;
    }

    Ok(repo
        .branches(Some(BranchType::Local))
        .expect("Failed to get list of local branches")
        .flatten()
        .map(|(local, _)| -> Result<AheadBehind, AheadBehindIterError> {
            if let Ok(upstream) = local.upstream() {
                // We have an upstream, so check the graph difference between it and the local
                let local_oid = local.get().target().ok_or_else(|| {
                    AheadBehindIterError::Oid(
                        branch_to_string(&local).unwrap_or_else(|| String::from("(unnamed??)")),
                    )
                })?;
                let upstream_oid = upstream.get().target().ok_or_else(|| {
                    AheadBehindIterError::Oid(
                        branch_to_string(&upstream).unwrap_or_else(|| String::from("(unnamed??)")),
                    )
                })?;
                let (ahead, behind) = repo
                    .graph_ahead_behind(local_oid, upstream_oid)
                    .map_err(AheadBehindIterError::CommitGraph)?;
                Ok(AheadBehind {
                    ahead: Some(ahead),
                    behind: Some(behind),
                    branch_name: branch_to_string(&local),
                    upstream_name: branch_to_string(&upstream),
                })
            } else {
                Ok(AheadBehind {
                    ahead: None,
                    behind: None,
                    branch_name: branch_to_string(&local),
                    upstream_name: None,
                })
            }
        }))
}

// Helper function to get the branch name as a string, or None if not found.
fn branch_to_string(branch: &Branch) -> Option<String> {
    branch.name().ok().flatten().map(ToOwned::to_owned)
}

// Credential check callback for providing credentials when working with an authenticated remote
// There was an earlier implementation for git_cred_check() which uses commands to access the credential
// manager. It worked, but was pretty verbose. Check the repo history if you need it.
fn git_cred_check(
    url: &str,
    username: Option<&str>,
    allowed_types: CredentialType,
) -> Result<Cred, git2::Error> {
    assert_eq!(allowed_types, CredentialType::USER_PASS_PLAINTEXT);

    let config = Config::open_default().expect("Couldn't find default git configuration");
    Cred::credential_helper(&config, url, username)
}

/// Indicates the state of a single git hook
pub enum HookState {
    /// Only in .git/hooks
    ActiveOnly,
    /// Only in .githooks
    InRepoOnly,
    /// In both locations but file contents don't match
    Mismatch,
    /// In both locations and file contents match
    Good,
}

/// Contains the name and state of a single git hook.
pub struct Hook {
    /// The filename of the git hook (the same name in .git/hooks and .githooks)
    pub name: OsString,
    /// The state of the git hook
    pub state: HookState,
}

/// Error type for `check_hooks()`
#[derive(thiserror::Error, Debug)]
#[error("File IO failed on \"{filename}\"")]
pub struct HooksError {
    /// Filename that op failed on
    filename: PathBuf,
    /// Underlying error
    source: io::Error,
}

/// Check whether git hooks match up in .githooks and .git/hooks
/// Ignore files that end with `.sample`.
/// For each hook found, give the filename and state of it.
///
/// # Errors
///
/// Return `Error::Io` if any file operation fails.
pub fn check_hooks(repo: &Repository) -> Result<Vec<Hook>, HooksError> {
    // Note that repo.path() points to the .git directory
    let active_dir = repo.path().join("hooks/");
    let active_hooks: HashSet<_> = hook_filenames_in_dir(&active_dir).collect();

    let in_repo_dir = repo.path().join("../.githooks/");
    let in_repo_hooks: HashSet<_> = hook_filenames_in_dir(&in_repo_dir).collect();

    let mut output = Vec::new();

    // Hooks in both - compare file contents
    let in_both: HashSet<_> = active_hooks.intersection(&in_repo_hooks).cloned().collect();
    for path in &in_both {
        let active_path = repo.path().join("hooks/").join(path);
        let active_bytes = fs::read(&active_path).map_err(|e| HooksError {
            filename: active_path,
            source: e,
        })?;
        let active_hash = Sha256::digest(active_bytes);

        let in_repo_path = repo.path().join("../.githooks/").join(path);
        let in_repo_bytes = fs::read(&in_repo_path).map_err(|e| HooksError {
            filename: in_repo_path,
            source: e,
        })?;
        let in_repo_hash = Sha256::digest(in_repo_bytes);

        let state = if active_hash == in_repo_hash {
            HookState::Good
        } else {
            HookState::Mismatch
        };
        output.push(Hook {
            name: path.clone(),
            state,
        });
    }

    // Hooks just in active dir
    for path in active_hooks.difference(&in_both) {
        output.push(Hook {
            name: path.clone(),
            state: HookState::ActiveOnly,
        });
    }

    // Hooks just in repo
    for path in in_repo_hooks.difference(&in_both) {
        output.push(Hook {
            name: path.clone(),
            state: HookState::InRepoOnly,
        });
    }

    Ok(output)
}

// Get a list of git hook filenames in the given directory.
// Ignores .sample files.
// If directory isn't present just report that it has no files
fn hook_filenames_in_dir(dir: &Path) -> impl Iterator<Item = OsString> + '_ {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| path.extension() != Some(OsStr::new("sample")))
        .filter_map(|path| path.file_name().map(ToOwned::to_owned))
}
