use clap::Parser;
use std::{path::PathBuf, fs::read_dir};

#[derive(Parser)]
struct Args {
    dir: PathBuf,
}

fn main() {
    let args = Args::parse();

    for entry in read_dir(args.dir).expect("Given path is not a directory: {dir}") {
        if let Ok(x) = entry {
            let path = x.path();
            if path.is_dir() {
                println!("{}", path.display());
            }
        }
    }
}
