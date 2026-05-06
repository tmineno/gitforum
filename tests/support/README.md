# Test Infrastructure

This directory contains shared support code for integration tests.

## Directory Layout

Files under `tests/` follow the categories defined in
`doc/spec/TEST-POLICY.md`. Names mirror their owner module in
`src/internal/` so a contributor can predict where a new test
belongs.

```
tests/
  # Module integration (category 1) — drives the library API directly.
  init_test.rs                     init + commit identity
  event_storage_test.rs            write_event / read_event / load_thread_events
  thread_test.rs                   replay, list, create, resolve, timestamp
  doctor_test.rs                   doctor checks + linked-thread advisory
  index_test.rs                    index db + reindex + search + tui startup +
                                   reverse-link queries
  id_alloc_test.rs                 thread ID allocation + validation
  ls_test.rs                       ls render + kind filters
  show_test.rs                     show render + nodes + tree advisory + DEC/TASK
  node_test.rs                     say / objection / resolve / retract / revise /
                                   find_node / node-id resolution
  state_change_test.rs             transitions + guards + fast_track + approvals +
                                   RFC deprecation + DEC/TASK lifecycle
  verify_test.rs                   verify_thread guard reports + linked-thread
                                   advisory
  policy_test.rs                   policy.toml load/lint + facet-scoped guards
  evidence_test.rs                 add_evidence + commit-evidence + show section
  thread_link_test.rs              add_thread_link + show + node_show
  brief_test.rs                    brief render + JSON schema

  # CLI surface (category 2) — spawns the git-forum binary.
  cli_*_test.rs

  # Cross-module behavior (category 3).
  operation_check_test.rs          operation-check rule tables (cross-module)
  migrate_test.rs                  1.x → 2.0 storage rewrite
  hook_test.rs                     git commit-msg hook
  purge_test.rs                    purge subcommand
  github_test.rs                   github import/export

  # Output goldens (category 4).
  snapshot_test.rs

  # Shared support (category 5).
  support/
    mod.rs                         Module declarations
    repo.rs                        TestRepo: isolated temp Git repos
    env.rs                         Environment isolation helpers
    clock.rs                       FixedClock / StepClock for tests
    ids.rs                         SequentialIdGenerator for tests
    forum.rs                       Forum-aware fixture helpers (setup, fixed_clock, ...)
    cli.rs                         git-forum subprocess helpers
                                   (bin, run, run_ok, extract_created_id,
                                    fresh_repo, make_thread_via_cli)
    git.rs                         Raw git + tree/blob helpers
                                   (git, create_real_branch,
                                    list_tree_paths, read_blob,
                                    ls_thread_tip, read_thread_file)
```

## Support Modules

### repo.rs

`TestRepo` creates a fresh `git init` in a temp directory with isolated config
(`GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`). Dropped on scope exit.

### clock.rs / ids.rs

Injectable `Clock` and `IdGenerator` implementations for deterministic tests.
`StepClock` increments by a fixed duration on each call. `SequentialIdGenerator`
produces predictable IDs like `human-alice-0001`.

### env.rs

`isolated_env()` builds an env map that isolates Git from the host.

### forum.rs

Forum-aware fixture helpers consolidated from per-module test files:
`setup`, `fixed_clock`, `make_thread`, `link_thread`, `build_index`, `open_index`.

### cli.rs

`git-forum` subprocess scaffolding shared by `tests/cli_*_test.rs` and
`tests/storage_v3_test.rs`:

- `bin()` — path to `CARGO_BIN_EXE_git-forum`.
- `run(repo_path, args)` / `run_ok(repo_path, args)` — invoke the
  binary in a repo path; `run_ok` asserts success and surfaces full
  stdout/stderr on failure.
- `extract_created_id(out)` — parse the thread id from a
  `Created <id> ...` stdout line.
- `fresh_repo()` — `TestRepo::new()` plus `init::init_forum`. Use
  for CLI tests that drive subcommands and assert on stdout/state.
- `make_thread_via_cli(repo_path, kind, title, body)` — shorthand
  for `git-forum new <kind> <title> --body <body>`; returns the new
  thread id.

### git.rs

Raw `git` and tree/blob helpers shared by branch-scope and
storage-shape tests:

- `git(repo_path, args)` — run a raw `git` command in `repo_path`,
  scrubbing host `GIT_DIR`/`GIT_WORK_TREE`/`GIT_INDEX_FILE`, asserts
  success.
- `create_real_branch(repo_path, branch)` — empty initial commit +
  branch pointing at HEAD (used by `branch bind` tests).
- `list_tree_paths(git_ops, refname)` — sorted
  `ls-tree -r --name-only` of the tip of `refname`.
- `read_blob(git_ops, refname, path)` — `cat-file -p` of `path` at
  the tip of `refname`.
- `ls_thread_tip(git_ops, id)` — convenience over `list_tree_paths`
  scoped to `refs/forum/threads/<id>`.
- `read_thread_file(git_ops, id, path)` — convenience over
  `read_blob` scoped to `refs/forum/threads/<id>`.

Helpers that are unique to a single test file (mock editor scripts in
`cli_edit_test.rs`, `legacy_blob_sha` in `snapshot_store_test.rs`) stay
local with a comment noting why.

## Rules

- Do not depend on global Git config
- Do not use the network
- Do not snapshot raw commit hashes or timestamps
- Integration tests must not share state
- Each test gets a fresh `TestRepo` in a temp directory
