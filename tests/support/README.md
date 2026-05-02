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

## Rules

- Do not depend on global Git config
- Do not use the network
- Do not snapshot raw commit hashes or timestamps
- Integration tests must not share state
- Each test gets a fresh `TestRepo` in a temp directory
