use clap::Parser;
use git2::{BranchType, Repository, StatusOptions};
use std::{fs::read_dir, path::PathBuf};

#[derive(Parser)]
struct Args {
    dir: PathBuf,
}

fn main() {
    let args = Args::parse();

    println!(
        "Kamino scanning repos in {:?}",
        args.dir.canonicalize().unwrap()
    );

    // Get all dir entries in given dir
    let mut dirs = Vec::new();
    for entry in read_dir(args.dir)
        .expect("Given path is not a directory: {dir}")
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }

    for dir in dirs {
        if let Ok(repo) = Repository::open(&dir) {
            let mut printed_header = false;

            if check_uncommitted(&repo) {
                if !printed_header {
                    printed_header = true;
                    println!("Repo in {dir:?}:");
                }
                println!("    Has uncommitted changes");
            }

            let ahead_behinds = check_ahead_behind(&repo);
            for ab in ahead_behinds {
                if let Some(ahead) = ab.ahead {
                    if ahead > 0 {
                        if !printed_header {
                            printed_header = true;
                            println!("Repo in {dir:?}:");
                        }
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
                        if !printed_header {
                            printed_header = true;
                            println!("Repo in {dir:?}:");
                        }
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

    // TODO: also check stashes?

    println!("Kamino scans complete!");
}

// Check if there are any uncommitted local changes
fn check_uncommitted(repo: &Repository) -> bool {
    let mut status_opts = StatusOptions::new();
    status_opts.include_ignored(false).include_untracked(true);

    let statuses = repo
        .statuses(Some(&mut status_opts))
        .expect("Error getting repo status");
    !statuses.is_empty()
    /*if !statuses.is_empty() {
        for s in statuses.iter() {
            println!("    {}", s.path().unwrap()); // TODO: use logging
        }
    }*/
}

struct AheadBehind {
    ahead: Option<usize>,
    behind: Option<usize>,
    branch_name: Option<String>,
    upstream_name: Option<String>,
}

// Check if local is ahead or behind remote
fn check_ahead_behind(repo: &Repository) -> Vec<AheadBehind> {
    // TODO: Fetch from origin first to make sure upstream is accurate.
    // If your remote isn't origin then tough luck.

    let mut output = Vec::new();
    for (local, _) in repo.branches(Some(BranchType::Local)).unwrap().flatten() {
        if let Ok(upstream) = local.upstream() {
            // We have an upstream, so check the graph difference between it and the local
            let local_oid = local.get().target().unwrap();
            let upstream_oid = upstream.get().target().unwrap();
            let (ahead, behind) = repo.graph_ahead_behind(local_oid, upstream_oid).unwrap();
            let ab = AheadBehind {
                ahead: Some(ahead),
                behind: Some(behind),
                branch_name: local.name().ok().flatten().map(|x| x.to_owned()),
                upstream_name: upstream.name().ok().flatten().map(|x| x.to_owned()),
            };
            output.push(ab);
        } else {
            let ab = AheadBehind {
                ahead: None,
                behind: None,
                branch_name: local.name().ok().flatten().map(|x| x.to_owned()),
                upstream_name: None,
            };
            output.push(ab);
        }
    }
    output
}
