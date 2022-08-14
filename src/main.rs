use clap::Parser;
use git2::{
    BranchType, Config, Cred, CredentialType, Error, FetchOptions, Oid, RemoteCallbacks,
    Repository, StatusOptions,
};
use std::{fs::read_dir, path::PathBuf};

#[derive(Parser)]
struct Args {
    dir: PathBuf,
}

struct DoOnce<F> {
    done: bool,
    f: F,
}

impl<F> DoOnce<F>
where
    F: FnMut(),
{
    fn new(f: F) -> Self {
        Self { done: false, f }
    }

    fn do_once(&mut self) {
        if !self.done {
            self.done = true;
            (self.f)();
        }
    }
}

fn main() {
    let args = Args::parse();

    println!(
        "Kamino scanning repos in {:?}",
        args.dir.canonicalize().unwrap()
    );

    // Get all dir entries in given dir
    let dirs: Vec<PathBuf> = read_dir(&args.dir)
        .unwrap_or_else(|_| panic!("Given path is not a directory: {}", args.dir.display()))
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            path.is_dir().then_some(path)
        })
        .collect();

    for dir in dirs {
        if let Ok(repo) = Repository::open(&dir) {
            let mut print_header =
                DoOnce::new(|| println!("{}:", dir.file_name().unwrap().to_string_lossy()));

            if check_uncommitted(&repo) {
                print_header.do_once();
                println!("    Has uncommitted changes");
            }

            let repo = {
                // Unfortunately checking the stash takes a mut ref to the repository although
                // it doesn't seem to actually modify anything. Since none of this program wants
                // to modify the repo we scope the mut ref.
                let mut repo = repo;
                let stashed = check_stashed(&mut repo);
                if stashed > 0 {
                    print_header.do_once();
                    println!("    Has {stashed} stashed changes");
                }
                repo
            };

            let ahead_behinds = check_ahead_behind(&repo);
            for ab in ahead_behinds {
                if let Some(ahead) = ab.ahead {
                    if ahead > 0 {
                        print_header.do_once();
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
                        print_header.do_once();
                        println!(
                            "    Branch {} is behind {} by {} commits",
                            ab.branch_name.as_deref().unwrap_or("(unnamed??)"),
                            ab.upstream_name.as_deref().unwrap_or("upstream"),
                            behind,
                        );
                    }
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
    /*if !statuses.is_empty() {
        for s in statuses.iter() {
            println!("    {}", s.path().unwrap()); // TODO: use logging
        }
    }*/
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
        .unwrap()
        .flatten()
        .map(|(local, _)| {
            if let Ok(upstream) = local.upstream() {
                // We have an upstream, so check the graph difference between it and the local
                let local_oid = local.get().target().unwrap();
                let upstream_oid = upstream.get().target().unwrap();
                let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid).unwrap();
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

    /*
    Attempt to make a credential reader before I realized credential_helper() was a thing. Keeping till I'm sure it's not needed.
    let url = Url::parse(url).unwrap_or_else(|_| panic!("Couldn't parse url \"{url}\""));
    let protocol = url.scheme();
    let host = url.host_str().unwrap_or_else(|| panic!("Couldn't find host name in url \"{url}\""));
    let protocol_str = format!("protocol={}", protocol);
    let host_str = format!("host={}", host);
    let fill_str = [protocol_str, host_str].join("\n");
    dbg!(&fill_str);

    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("Couldn't spawn git process");

    let child_stdin = child.stdin.as_mut().expect("Couldn't get stdin on child process");
    child_stdin.write_all(fill_str.as_bytes()).expect("Write to stdin failed");

    let output = child.wait_with_output().expect("Process execution / wait failed");
    let output_str = String::from_utf8(output.stdout).expect("Process output is not utf8");
    let mut password = None;
    for line in output_str.lines() {
        let (key, value) = line.split_once('=').unwrap_or_else(|| panic!("Couldn't split line {line}"));
        if matches!(key, "password") {
            password = Some(value);
        }
    }
    dbg!(output_str);

    todo!()*/

    let config_path = Config::find_global().expect("Couldn't find global git configuration");
    let config = Config::open(&config_path)
        .unwrap_or_else(|_| panic!("Couldn't open git config file {config_path:?}"));
    Cred::credential_helper(&config, url, username)
}
