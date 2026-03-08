# Test Support Design

This directory contains shared support code for integration tests and TUI tests.
For the MVP, it is the place where we enforce three rules: every test gets a clean Git repo, unstable values are fixed, and external dependencies are replaced with fakes.

## Planned modules

- `repo.rs`
  - create a temporary directory
  - run `git init`
  - set `user.name` and `user.email`
  - help inspect `.forum/` and `.git/forum/`
- `env.rs`
  - isolate `HOME`, `XDG_CONFIG_HOME`, and `GIT_CONFIG_NOSYSTEM=1`
  - inject test-specific environment variables
- `cli.rs`
  - run the `git-forum` binary
  - make stdout / stderr / exit code easy to assert
- `clock.rs`
  - provide a fixed clock or step clock
- `ids.rs`
  - provide a fixed ID generator / predictable sequence
- `git.rs`
  - create commits for tests
  - fix `GIT_AUTHOR_DATE` and `GIT_COMMITTER_DATE`
  - help with branch creation and merge
- `ai.rs`
  - fake provider
  - return fixed run results / tool calls / confidence
- `tui.rs`
  - build the TUI backend
  - build test inputs for list / detail rendering

## Planned sibling directories

- `tests/fixtures/`
  - fixed files for import / export
  - replay / merge reproduction inputs
- `tests/snapshots/`
  - `show`
  - `verify`
  - export
  - TUI render

## Rules

- do not depend on global Git config
- do not use the network
- do not snapshot raw commit hashes or timestamps
- integration tests must not share state
- prefer stable render assertions over full interactive key-sequence automation in TUI tests
