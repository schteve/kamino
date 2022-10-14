# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## v1.1.1 - 2022-Oct-13

- Add tests
- Allow path argument to be optional, defaulting to '.'

## v1.1.0 - 2022-Sep-09

- Split binary and library
- Address various pedantic clippy lints
- Return iterators in a few interfaces in case the user doesn't want to allocate
- Return errors rather than panicking
- Allow user to specify remote name rather than assuming "origin"

## v1.0.0 - 2022-Aug-22

### Add
- Check for uncommitted changes
- Check for stashed changes
- Fetch remote and check whether local is ahead / behind
- Check if hooks in `.githooks` match those in `.git/hooks`
- Initial release 🎉
