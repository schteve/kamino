#![deny(unsafe_code)]

use clap::Parser;
use git2::Repository;
use kamino::HookState;
use std::{error::Error, fs, path::PathBuf, sync::Once};

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
            if let Err(e) = check_repo(repo) {
                eprintln!("Error: {}", e);
                if let Some(source) = e.source() {
                    eprintln!("Caused by: {}", source);
                }
                return;
            }
        }
    }

    println!("Kamino scans complete!");
}

fn check_repo(repo: Repository) -> Result<(), kamino::Error> {
    let print_header_once = {
        let once = Once::new();
        let path = repo.path().display().to_string();
        move || once.call_once(|| println!("{}:", path))
    };

    if kamino::check_uncommitted(&repo)? {
        print_header_once();
        println!("    Has uncommitted changes");
    }

    let repo = {
        // Unfortunately checking the stash takes a mut ref to the repository although
        // it doesn't seem to actually modify anything. Since none of this program wants
        // to modify the repo we scope the mut ref.
        let mut repo = repo;
        let stashed = kamino::check_stashed(&mut repo)?;
        if stashed > 0 {
            print_header_once();
            println!("    Has {stashed} stashed changes");
        }
        repo
    };

    for ab in kamino::check_ahead_behind(&repo)? {
        let ab = ab?;

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

    for hook in kamino::check_hooks(&repo)? {
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

    Ok(())
}
