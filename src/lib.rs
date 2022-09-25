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

/// Error type for [`check_uncommitted()`].
#[derive(thiserror::Error, Debug)]
#[error("failed getting repo status for {path}")]
pub struct UncommittedError {
    /// Path to the repo.
    path: PathBuf,
    /// Underlying error.
    source: git2::Error,
}

/// Check if there are any uncommitted local changes.
///
/// # Errors
///
/// See [`UncommittedError`].
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

/// Error type for [`check_stashed()`].
#[derive(thiserror::Error, Debug)]
#[error("failed to check the stash")]
pub struct StashedError(#[source] git2::Error);

/// Check if there are any stashed changes.
///
/// # Errors
///
/// See [`StashedError`].
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
#[derive(Clone, Debug, Eq, PartialEq)]
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

/// Error type for [`check_ahead_behind()`].
#[derive(thiserror::Error, Debug)]
#[error("failed to fetch origin")]
pub struct AheadBehindError(#[source] git2::Error);

/// Error type for the iterator returned from [`check_ahead_behind()`].
#[derive(thiserror::Error, Debug)]
pub enum AheadBehindIterError {
    /// Failed to get OID of a branch.
    #[error("failed to get OID of branch {0}")]
    Oid(String),

    /// Failed to check the commit graph.
    #[error("Error while checking graph ahead/behind")]
    CommitGraph(#[source] git2::Error),
}

/// Check if each local branch is ahead or behind the remote.
/// Fetch from origin first to make sure upstream is accurate.
///
/// # Errors
///
/// See [`AheadBehindError`].
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

// Helper function to get the branch name as a string, or `None` if not found.
fn branch_to_string(branch: &Branch) -> Option<String> {
    branch.name().ok().flatten().map(ToOwned::to_owned)
}

// Credential check callback for providing credentials when working with an authenticated remote.
//
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

/// Indicates the state of a single git hook.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookState {
    /// Only in `.git/hooks`.
    ActiveOnly,
    /// Only in `.githooks`.
    InRepoOnly,
    /// In both locations but file contents don't match.
    Mismatch,
    /// In both locations and file contents match.
    Good,
}

/// Contains the name and state of a single git hook.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hook {
    /// The filename of the git hook (the same name in `.git/hooks` and `.githooks`).
    pub name: OsString,
    /// The state of the git hook.
    pub state: HookState,
}

/// Error type for [`check_hooks()`].
#[derive(thiserror::Error, Debug)]
#[error("File IO failed on \"{filename}\"")]
pub struct HookError {
    /// Filename that op failed on.
    filename: PathBuf,
    /// Underlying error.
    source: io::Error,
}

/// Check whether git hooks match up in `.githooks` and `.git/hooks`.
/// Ignore files that end with `.sample`.
/// For each hook found, give the filename and state of it.
///
/// # Errors
///
/// See [`HookError`].
pub fn check_hooks(repo: &Repository) -> Result<Vec<Hook>, HookError> {
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
        let active_bytes = fs::read(&active_path).map_err(|e| HookError {
            filename: active_path,
            source: e,
        })?;
        let active_hash = Sha256::digest(active_bytes);

        let in_repo_path = repo.path().join("../.githooks/").join(path);
        let in_repo_bytes = fs::read(&in_repo_path).map_err(|e| HookError {
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
// If directory isn't present just report that it has no files.
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

#[cfg(test)]
mod test {
    use super::*;
    use git2::{RepositoryInitOptions, StashFlags};
    use std::{
        fs::{self, File},
        io::Write,
    };
    use tempfile::TempDir;

    // Stolen from git2
    fn repo_init() -> (TempDir, Repository) {
        let td = TempDir::new().unwrap();
        let mut opts = RepositoryInitOptions::new();
        opts.initial_head("main");
        let repo = Repository::init_opts(td.path(), &opts).unwrap();
        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "name").unwrap();
            config.set_str("user.email", "email").unwrap();
            let mut index = repo.index().unwrap();
            let id = index.write_tree().unwrap();

            let tree = repo.find_tree(id).unwrap();
            let sig = repo.signature().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "initial\n\nbody", &tree, &[])
                .unwrap();
        }
        (td, repo)
    }

    fn create_file(dir: &Path, filename: &str) {
        create_file_with_contents(dir, filename, "contents");
    }

    fn create_file_with_contents(dir: &Path, filename: &str, contents: &str) {
        if !dir.exists() {
            fs::create_dir(dir).unwrap();
        }
        File::create(dir.join(filename))
            .unwrap()
            .write_all(contents.as_bytes())
            .unwrap();
    }

    fn remove_file(dir: &Path, filename: &str) {
        fs::remove_file(dir.join(filename)).unwrap();
    }

    fn add_file_to_index(repo: &Repository, filename: &str) {
        let mut index = repo.index().unwrap();
        index.add_path(&Path::new(filename)).unwrap();
    }

    fn commit_index_to_branch(repo: &Repository, branch_name: &str) -> (Oid, Oid) {
        let mut index = repo.index().unwrap();
        let mut branch_ref_name = String::from("refs/heads/");
        branch_ref_name.push_str(branch_name);

        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = repo.signature().unwrap();
        let oid = repo.refname_to_id(branch_ref_name.as_str()).unwrap();
        let parent = repo.find_commit(oid).unwrap();
        let commit = repo
            .commit(
                Some(branch_ref_name.as_str()),
                &sig,
                &sig,
                "commit",
                &tree,
                &[&parent],
            )
            .unwrap();
        (commit, tree_id)
    }

    fn create_branch_at_head(repo: &Repository, name: &str) {
        let head = repo.head().unwrap();
        let target = head.target().unwrap();
        let commit = repo.find_commit(target).unwrap();
        repo.branch(name, &commit, false).unwrap();
    }

    // Note: usually upstream wants to point to a remote, in which case you need to include the remote
    // name in the upstream name e.g. "origin/mybranch"
    fn set_branch_upstream(repo: &Repository, local_name: &str, upstream_name: Option<&str>) {
        repo.find_branch(local_name, BranchType::Local)
            .unwrap()
            .set_upstream(upstream_name)
            .unwrap();
    }

    #[test]
    fn uncommitted() {
        let (dir, repo) = repo_init();
        assert!(!check_uncommitted(&repo).unwrap());

        // Working directory
        create_file(dir.path(), "file");
        assert!(check_uncommitted(&repo).unwrap());

        // Index
        add_file_to_index(&repo, "file");
        assert!(check_uncommitted(&repo).unwrap());
    }

    #[test]
    fn stashed() {
        let (dir, mut repo) = repo_init();
        assert_eq!(check_stashed(&mut repo).unwrap(), 0);

        create_file(dir.path(), "file1");
        repo.stash_save(
            &repo.signature().unwrap(),
            "msg1",
            Some(StashFlags::INCLUDE_UNTRACKED),
        )
        .unwrap();
        assert_eq!(check_stashed(&mut repo).unwrap(), 1);

        create_file(dir.path(), "file2");
        repo.stash_save(
            &repo.signature().unwrap(),
            "msg2",
            Some(StashFlags::INCLUDE_UNTRACKED),
        )
        .unwrap();
        assert_eq!(check_stashed(&mut repo).unwrap(), 2);

        repo.stash_drop(0).unwrap();
        repo.stash_drop(0).unwrap();
        assert_eq!(check_stashed(&mut repo).unwrap(), 0);
    }

    #[test]
    fn ahead_behind() {
        let (upstream_dir, upstream_repo) = repo_init();
        let (local_dir, local_repo) = repo_init();
        local_repo
            .remote("origin", upstream_dir.path().to_str().unwrap())
            .unwrap();

        // Create branches on local and upstream
        create_branch_at_head(&local_repo, "b1");
        create_branch_at_head(&local_repo, "b2");
        create_branch_at_head(&local_repo, "b3");
        create_branch_at_head(&upstream_repo, "b1");
        create_branch_at_head(&upstream_repo, "b2");
        create_branch_at_head(&upstream_repo, "b3");

        // Fetch the newly created branches from upstream. This is required in order to set
        // these as upstream tracking branches for the local ones.
        if let Ok(mut remote) = local_repo.find_remote("origin") {
            let refspecs: &[&str] = &[];
            remote
                .fetch(refspecs, None, None)
                .map_err(AheadBehindError)
                .unwrap();
        }

        // Connect local and upstream branches
        set_branch_upstream(&local_repo, "b1", Some("origin/b1"));
        set_branch_upstream(&local_repo, "b2", Some("origin/b2"));
        set_branch_upstream(&local_repo, "b3", Some("origin/b3"));

        // Make local b1 ahead of upstream
        create_file(local_dir.path(), "file1");
        add_file_to_index(&local_repo, "file1");
        commit_index_to_branch(&local_repo, "b1");

        // Make upstream b2 ahead of local
        create_file(upstream_dir.path(), "file2");
        add_file_to_index(&upstream_repo, "file2");
        commit_index_to_branch(&upstream_repo, "b2");

        // Make b3 ahead and behind
        create_file(local_dir.path(), "file3a");
        add_file_to_index(&local_repo, "file3a");
        commit_index_to_branch(&local_repo, "b3");

        create_file(upstream_dir.path(), "file3b");
        add_file_to_index(&upstream_repo, "file3b");
        commit_index_to_branch(&upstream_repo, "b3");

        let results: Vec<AheadBehind> = check_ahead_behind(&local_repo, "origin")
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(results.len(), 4);

        assert!(results.contains(&AheadBehind {
            ahead: None,
            behind: None,
            branch_name: Some("main".into()),
            upstream_name: None,
        }));
        assert!(results.contains(&AheadBehind {
            ahead: Some(1),
            behind: Some(0),
            branch_name: Some("b1".into()),
            upstream_name: Some("origin/b1".into()),
        }));
        assert!(results.contains(&AheadBehind {
            ahead: Some(0),
            behind: Some(1),
            branch_name: Some("b2".into()),
            upstream_name: Some("origin/b2".into()),
        }));
        assert!(results.contains(&AheadBehind {
            ahead: Some(1),
            behind: Some(1),
            branch_name: Some("b3".into()),
            upstream_name: Some("origin/b3".into()),
        }));
    }

    #[test]
    fn hooks() {
        let (dir, repo) = repo_init();
        let results = check_hooks(&repo).unwrap();
        assert!(results.is_empty());

        let active_dir = dir.path().join(".git/hooks");
        let in_repo_dir = dir.path().join(".githooks");

        // Only in `.git/hooks`.
        create_file(&active_dir, "hook.sample");
        create_file(&active_dir, "hook1");
        let results = check_hooks(&repo).unwrap();
        assert_eq!(
            results,
            vec![Hook {
                name: "hook1".into(),
                state: HookState::ActiveOnly
            }]
        );
        remove_file(&active_dir, "hook.sample");
        remove_file(&active_dir, "hook1");

        // Only in `.githooks`.
        create_file(&in_repo_dir, "hook.sample");
        create_file(&in_repo_dir, "hook1");
        let results = check_hooks(&repo).unwrap();
        assert_eq!(
            results,
            vec![Hook {
                name: "hook1".into(),
                state: HookState::InRepoOnly
            }]
        );
        remove_file(&in_repo_dir, "hook.sample");
        remove_file(&in_repo_dir, "hook1");

        // In both locations but file contents don't match.
        create_file_with_contents(&active_dir, "hook.sample", "a");
        create_file_with_contents(&active_dir, "hook1", "b");
        create_file_with_contents(&in_repo_dir, "hook.sample", "c");
        create_file_with_contents(&in_repo_dir, "hook1", "d");
        let results = check_hooks(&repo).unwrap();
        assert_eq!(
            results,
            vec![Hook {
                name: "hook1".into(),
                state: HookState::Mismatch
            }]
        );
        remove_file(&active_dir, "hook.sample");
        remove_file(&active_dir, "hook1");
        remove_file(&in_repo_dir, "hook.sample");
        remove_file(&in_repo_dir, "hook1");

        // In both locations and file contents match.
        create_file(&active_dir, "hook.sample");
        create_file(&active_dir, "hook1");
        create_file(&in_repo_dir, "hook.sample");
        create_file(&in_repo_dir, "hook1");
        let results = check_hooks(&repo).unwrap();
        assert_eq!(
            results,
            vec![Hook {
                name: "hook1".into(),
                state: HookState::Good
            }]
        );
        remove_file(&active_dir, "hook.sample");
        remove_file(&active_dir, "hook1");
        remove_file(&in_repo_dir, "hook.sample");
        remove_file(&in_repo_dir, "hook1");
    }
}
