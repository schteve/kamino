#![deny(unsafe_code)]

use clap::Parser;
use git2::{
    BranchType, Config, Cred, CredentialType, Error, FetchOptions, Oid, RemoteCallbacks,
    Repository, StatusOptions,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    sync::Once,
};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)] // Read from `Cargo.toml`
struct Args {
    dir: PathBuf,
}

fn main() {
    let args = Args::parse();

    println!(
        "Kamino scanning repos in {:?}",
        args.dir
            .canonicalize()
            .unwrap_or_else(|_| panic!("Failed to canonicalize {:?}", args.dir)),
    );

    // Get all dir entries in given dir
    let dirs: Vec<PathBuf> = fs::read_dir(&args.dir)
        .unwrap_or_else(|_| panic!("Given path is not a directory: {}", args.dir.display()))
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then_some(path)
        })
        .collect();

    for dir in dirs {
        if let Ok(repo) = Repository::open(&dir) {
            let once = Once::new();
            let print_header_once =
                move || once.call_once(|| println!("{}:", dir.to_string_lossy()));

            if check_uncommitted(&repo) {
                print_header_once();
                println!("    Has uncommitted changes");
            }

            let repo = {
                // Unfortunately checking the stash takes a mut ref to the repository although
                // it doesn't seem to actually modify anything. Since none of this program wants
                // to modify the repo we scope the mut ref.
                let mut repo = repo;
                let stashed = check_stashed(&mut repo);
                if stashed > 0 {
                    print_header_once();
                    println!("    Has {stashed} stashed changes");
                }
                repo
            };

            for ab in check_ahead_behind(&repo) {
                if let Some(ahead) = ab.ahead {
                    if ahead > 0 {
                        print_header_once();
                        println!(
                            "    Branch {} is ahead of {} by {} commits",
                            ab.branch_name.as_deref().unwrap_or("(unnamed??)"),
                            ab.upstream_name.as_deref().unwrap_or("upstream"),
                            ahead,
                        );
                    }
                }

                if let Some(behind) = ab.behind {
                    if behind > 0 {
                        print_header_once();
                        println!(
                            "    Branch {} is behind {} by {} commits",
                            ab.branch_name.as_deref().unwrap_or("(unnamed??)"),
                            ab.upstream_name.as_deref().unwrap_or("upstream"),
                            behind,
                        );
                    }
                }
            }

            for hook in check_hooks(&repo) {
                match hook.state {
                    HookState::ActiveOnly => {
                        print_header_once();
                        println!("    Hook {:?} only appears in .git/hooks", hook.name);
                    }
                    HookState::InRepoOnly => {
                        print_header_once();
                        println!("    Hook {:?} only appears in .githooks", hook.name);
                    }
                    HookState::Mismatch => {
                        print_header_once();
                        println!(
                            "    Hook {:?} is different in .git/hooks and .githooks",
                            hook.name
                        );
                    }
                    HookState::Good => (),
                }
            }
        }
    }

    println!("Kamino scans complete!");
}

// Check if there are any uncommitted local changes
fn check_uncommitted(repo: &Repository) -> bool {
    let mut status_opts = StatusOptions::new();
    status_opts.include_ignored(false).include_untracked(true);

    let statuses = repo
        .statuses(Some(&mut status_opts))
        .unwrap_or_else(|_| panic!("Error getting repo status for {:?}", repo.path()));
    !statuses.is_empty()
}

// Check if there are any stashed changes.
fn check_stashed(repo: &mut Repository) -> u32 {
    let mut stash_count = 0;

    let cb = |_index: usize, _msg: &str, _id: &Oid| -> bool {
        stash_count += 1;
        true
    };
    repo.stash_foreach(cb).expect("Checking the stash failed");

    stash_count
}

struct AheadBehind {
    ahead: Option<usize>,
    behind: Option<usize>,
    branch_name: Option<String>,
    upstream_name: Option<String>,
}

// Check if local is ahead or behind remote
fn check_ahead_behind(repo: &Repository) -> Vec<AheadBehind> {
    // Fetch from origin first to make sure upstream is accurate.
    // If your remote isn't origin then tough luck.
    if let Ok(mut remote) = repo.find_remote("origin") {
        let refspecs: &[&str] = &[]; // Use base refspecs, which I assume means all local branches
        let mut cbs = RemoteCallbacks::new();
        cbs.credentials(git_cred_check);
        let mut opts = FetchOptions::new();
        opts.remote_callbacks(cbs);
        remote
            .fetch(refspecs, Some(&mut opts), None)
            .expect("Fetch on origin failed");
    }

    repo.branches(Some(BranchType::Local))
        .expect("Failed to get list of local branches")
        .flatten()
        .map(|(local, _)| {
            if let Ok(upstream) = local.upstream() {
                // We have an upstream, so check the graph difference between it and the local
                let local_oid = local
                    .get()
                    .target()
                    .expect("Failed to get OID of local branch");
                let upstream_oid = upstream
                    .get()
                    .target()
                    .expect("Failed to get OID of upstream branch");
                let (ahead, behind) = repo
                    .graph_ahead_behind(local_oid, upstream_oid)
                    .expect("Error while checking graph ahead/behind");
                AheadBehind {
                    ahead: Some(ahead),
                    behind: Some(behind),
                    branch_name: local.name().ok().flatten().map(|x| x.to_owned()),
                    upstream_name: upstream.name().ok().flatten().map(|x| x.to_owned()),
                }
            } else {
                AheadBehind {
                    ahead: None,
                    behind: None,
                    branch_name: local.name().ok().flatten().map(|x| x.to_owned()),
                    upstream_name: None,
                }
            }
        })
        .collect()
}

// Credential check callback for providing credentials when working with an authenticated remote
fn git_cred_check(
    url: &str,
    username: Option<&str>,
    allowed_types: CredentialType,
) -> Result<Cred, Error> {
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

enum HookState {
    ActiveOnly, // Only in .git/hooks
    InRepoOnly, // Only in .githooks
    Mismatch,   // In both locations but file contents don't match
    Good,       // In both locations and file contents match
}

struct Hook {
    name: OsString,
    state: HookState,
}

// Check whether git hooks match up in .githooks and .git/hooks
// Note that repo.path() points to the .git directory
fn check_hooks(repo: &Repository) -> Vec<Hook> {
    let active_dir = repo.path().join("hooks/");
    let active_hooks = hook_filenames_in_dir(&active_dir);

    let in_repo_dir = repo.path().join("../.githooks/");
    let in_repo_hooks = hook_filenames_in_dir(&in_repo_dir);

    let mut output = Vec::new();

    // Hooks in both - compare file contents
    let in_both: HashSet<_> = active_hooks.intersection(&in_repo_hooks).cloned().collect();
    for path in &in_both {
        let active_path = repo.path().join("hooks/").join(path);
        let active_bytes =
            fs::read(&active_path).unwrap_or_else(|_| panic!("Couldn't open file {active_path:?}"));
        let active_hash = Sha256::digest(active_bytes);

        let in_repo_path = repo.path().join("../.githooks/").join(path);
        let in_repo_bytes = fs::read(&in_repo_path)
            .unwrap_or_else(|_| panic!("Couldn't open file {in_repo_path:?}"));
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

    output
}

// Get a list of git hook filenames in the given directory.
// Ignores .sample files.
fn hook_filenames_in_dir(dir: &Path) -> HashSet<OsString> {
    if let Ok(contents) = fs::read_dir(dir) {
        contents
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| path.extension() != Some(OsStr::new("sample")))
            .filter_map(|path| path.file_name().map(|s| s.to_owned()))
            .collect()
    } else {
        // If directory isn't present just report that it has no files
        HashSet::new()
    }
}
