use clap::Parser;
use git2::{Repository, StatusOptions};
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

    // Check if there are any uncommitted local changes
    for dir in dirs {
        if let Ok(repo) = Repository::open(&dir) {
            if check_uncommitted(&repo) {
                println!("\tRepo in {dir:?} has uncommitted changes");
            }
        }
    }

    println!("Kamino scans complete!");
}

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
