# git-kamino

This program helps manage a bunch of repo clones, like when you are working on many repos synced between multiple
machines. It checks for repos that need to be committed (changes in working copy), pushed (local commits not
on the remote) or pulled (remote is ahead of local). It doesn't actually resolve any of those things for you -
just tell you that something needs to be done.
