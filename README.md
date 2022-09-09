[![Crates.io](https://img.shields.io/crates/v/kamino)](https://crates.io/crates/kamino)
[![docs.rs](https://img.shields.io/docsrs/kamino)](https://docs.rs/kamino)
[![CI](https://github.com/schteve/kamino/actions/workflows/ci.yml/badge.svg)](https://github.com/schteve/kamino/actions/workflows/ci.yml)

# kamino

`kamino` helps manage a bunch of git repo clones. Specifically, it helps when you are working on repos on multiple
machines which which you keep in sync with the same remote server.

It tells you which repos aren't in sync with the remote:
- If there are uncommitted changes, in the working copy or the index (maybe you forgot to commit?)
- If there are stashed changes (maybe you wanted to apply them?)
- If there are local commits not on the remote (maybe you forgot to push?)
- If the remote is ahead of local (maybe you forgot to pull?)
- If the git hooks in `.githooks` (if present) match the ones in `.git/hooks` (maybe you forgot to install / update a hook? maybe you have an active hook that should go into the repo?). This only checks the working copy and ignores `.sample` files.

# Binary

The binary program doesn't actually fix any of the above conditions, because it doesn't know what you want to do about it. It just tells you in case you want to do something. Note that to check local vs remote it performs a fetch.

`kamino` scans for git repos within the directory you provide. Currently, this is a shallow scan that only looks one layer deep.

# License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

# Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
