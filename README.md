# kamino

`kamino` helps manage a bunch of git repo clones. Specifically, it helps when you are working on repos on multiple
machines which which you keep in sync with the same remote server.

It tells you which repos aren't in sync with the remote:
- If there are uncommitted changes, in the working copy or the index (maybe you forgot to commit?)
- If there are stashed changes (maybe you wanted to apply them?)
- If there are local commits not on the remote (maybe you forgot to push?)
- If the remote is ahead of local (maybe you forgot to pull?)

This program doesn't actually fix any of the above conditions, because it doesn't know what you want to do about it. It just tells you that you may want to do something.

`kamino` scans for git repos within the directory you provide. Currently, this is a shallow scan that only looks one layer deep.
