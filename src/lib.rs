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

/// Various errors that can occur while using this library
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// Failed getting repo status
    #[error("failed getting repo status for {path}")]
    RepoStatus {
        /// Path to the repo
        path: PathBuf,
        /// Underlying error
        source: git2::Error,
    },

    /// Failed accessing the stash
    #[error("failed to check the stash")]
    Stash(#[source] git2::Error),

    /// Failed to fetch origin
    #[error("failed to fetch origin")]
    FetchOrigin(#[source] git2::Error),

    /// Failed to get OID of a branch
    #[error("failed to get OID of branch {0}")]
    Oid(String),

    /// Failed to check the commit graph
    #[error("Error while checking graph ahead/behind")]
    CommitGraph(#[source] git2::Error),

    /// A filesystem IO operation failed
    #[error("File IO failed on {filename}")]
    Io {
        /// Filename that op failed on
        filename: PathBuf,
        /// Underlying error
        source: io::Error,
    },
}

/// Check if there are any uncommitted local changes
///
/// # Errors
///
/// Return `Error::RepoStatus` if the repo status query fails.
pub fn check_uncommitted(repo: &Repository) -> Result<bool, Error> {
    let mut status_opts = StatusOptions::new();
    status_opts.include_ignored(false).include_untracked(true);

    let statuses = repo
        .statuses(Some(&mut status_opts))
        .map_err(|e| Error::RepoStatus {
            path: repo.path().to_owned(),
            source: e,
        })?;
    Ok(!statuses.is_empty())
}

/// Check if there are any stashed changes.
///
/// # Errors
///
/// Return `Error::Stash` if any of the stash queries fail.
pub fn check_stashed(repo: &mut Repository) -> Result<u32, Error> {
    let mut stash_count = 0;

    let cb = |_index: usize, _msg: &str, _id: &Oid| -> bool {
        stash_count += 1;
        true
    };
    repo.stash_foreach(cb).map_err(Error::Stash)?;

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

/// Check if local is ahead or behind remote
/// Fetch from origin first to make sure upstream is accurate.
/// If your remote isn't origin then tough luck.
///
/// # Errors
///
/// Return `Error::FetchOrigin` if the fetching fails.
/// Return `Error::OidLocal` if querying the local branch OID fails.
/// Return `Error::OidUpstream` if querying the upstream branch OID fails.
/// Return `Error::CommitGraph` if querying the commit graph fails.
pub fn check_ahead_behind(
    repo: &Repository,
) -> Result<impl Iterator<Item = Result<AheadBehind, Error>> + '_, Error> {
    if let Ok(mut remote) = repo.find_remote("origin") {
        let refspecs: &[&str] = &[]; // Use base refspecs, which I assume means all local branches
        let mut cbs = RemoteCallbacks::new();
        cbs.credentials(git_cred_check);
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(cbs);
        remote
            .fetch(refspecs, Some(&mut opts), None)
            .map_err(Error::FetchOrigin)?;
    }

    Ok(repo
        .branches(Some(BranchType::Local))
        .expect("Failed to get list of local branches")
        .flatten()
        .map(|(local, _)| -> Result<AheadBehind, Error> {
            if let Ok(upstream) = local.upstream() {
                // We have an upstream, so check the graph difference between it and the local
                let local_oid = local.get().target().ok_or_else(|| {
                    Error::Oid(
                        branch_to_string(&local).unwrap_or_else(|| String::from("(unnamed??)")),
                    )
                })?;
                let upstream_oid = upstream.get().target().ok_or_else(|| {
                    Error::Oid(
                        branch_to_string(&upstream).unwrap_or_else(|| String::from("(unnamed??)")),
                    )
                })?;
                let (ahead, behind) = repo
                    .graph_ahead_behind(local_oid, upstream_oid)
                    .map_err(Error::CommitGraph)?;
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
fn git_cred_check(
    url: &str,
    username: Option<&str>,
    allowed_types: CredentialType,
) -> Result<Cred, git2::Error> {
    assert_eq!(allowed_types, CredentialType::USER_PASS_PLAINTEXT);

    let config = Config::open_default().expect("Couldn't find default git configuration");
    Cred::credential_helper(&config, url, username)
}

/*
    This was an earlier implementation for git_cred_check() which uses commands to access the credential
    manager. I'm keeping the credential_helper() implementation in place but keeping this nearby since
    I suspect it might be needed in the future. If credential_helper() fails try using this.
        - Add url crate

fn git_cred_check(
    url: &str,
    _username: Option<&str>,
    allowed_types: CredentialType,
) -> Result<Cred, Error> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    use url::Url;

    assert_eq!(allowed_types, CredentialType::USER_PASS_PLAINTEXT);

    let url = Url::parse(url).unwrap_or_else(|_| panic!("Couldn't parse url \"{url}\""));
    let protocol = url.scheme();
    let host = url
        .host_str()
        .unwrap_or_else(|| panic!("Couldn't find host name in url \"{url}\""));
    let protocol_str = format!("protocol={}", protocol);
    let host_str = format!("host={}", host);
    let fill_str = [protocol_str, host_str].join("\n");

    // Create a child process where the stdin and stdout are accessible to us
    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Couldn't spawn git process");

    // Write the input to stdin
    let child_stdin = child
        .stdin
        .as_mut()
        .expect("Couldn't get stdin on child process");
    child_stdin
        .write_all(fill_str.as_bytes())
        .expect("Write to stdin failed");

    // Wait for the process to complete
    let output = child
        .wait_with_output()
        .expect("Process execution / wait failed");
    let output_str = String::from_utf8(output.stdout).expect("Process output is not utf8");

    let mut username = None;
    let mut password = None;
    for line in output_str.lines() {
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("Couldn't split line {line}"));
        match key {
            "username" => username = Some(value),
            "password" => password = Some(value),
            _ => (),
        }
    }

    Cred::userpass_plaintext(
        username.expect("Couldn't find username"),
        password.expect("Couldn't find password"),
    )
}
*/

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

/// Check whether git hooks match up in .githooks and .git/hooks
/// Ignore files that end with `.sample`.
/// For each hook found, give the filename and state of it.
///
/// # Errors
///
/// Return `Error::Io` if any file operation fails.
pub fn check_hooks(repo: &Repository) -> Result<Vec<Hook>, Error> {
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
        let active_bytes = fs::read(&active_path).map_err(|e| Error::Io {
            filename: active_path,
            source: e,
        })?;
        let active_hash = Sha256::digest(active_bytes);

        let in_repo_path = repo.path().join("../.githooks/").join(path);
        let in_repo_bytes = fs::read(&in_repo_path).map_err(|e| Error::Io {
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
