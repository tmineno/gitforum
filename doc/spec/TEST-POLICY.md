# Spec: Integration test file policy

Version 1.0 — 2026-05-02

Governs the contents and naming of files under `tests/`. Unit tests inside
`src/` follow normal Rust conventions and are out of scope.

## Goal

A new contributor (human or agent) can predict where a test belongs from its
subject alone, and an existing test's location explains what it covers without
reading the file.

## Non-goals

- Naming policy for individual `#[test]` functions inside a file.
- Reorganizing `src/`-level unit tests.
- Defining how `tests/support/` helpers are partitioned.

## Test file categories

Every file directly under `tests/` belongs to exactly one of:

1. **Module integration** — `<module>_test.rs`, mirrors a module in
   `src/internal/<module>.rs`. Drives the library API directly (no subprocess
   spawn). Examples: `init_test.rs`, `doctor_test.rs`, `state_change_test.rs`,
   `evidence_test.rs`, `index_test.rs`.

2. **CLI surface** — `cli_<topic>_test.rs`. Spawns the `git-forum` binary via
   `Command`. Topic = command name, flag, or behavior cluster
   (`cli_thread_new_test.rs`, `cli_edit_test.rs`, `cli_branch_scope_test.rs`).

3. **Cross-module behavior** — `<concept>_test.rs` for tests that legitimately
   cut across modules and have no single owner module. Reserved for genuine
   cross-cutting concerns; not a dumping ground. Current members:
   `operation_check_test.rs`, `migrate_test.rs`, `hook_test.rs`,
   `purge_test.rs`, `github_test.rs`.

4. **Output goldens** — `snapshot_test.rs` (singular). Fixtures live in
   `tests/snapshots/`.

5. **Shared support** — `tests/support/` only.

6. **Versioned storage-shape** — `storage_v{N}_*_test.rs`. Tests that
   intentionally couple to the on-disk shape at
   `refs/forum/threads/<id>` and therefore CANNOT stay green across a
   storage-shape cutover. Added by task `4w8hm98j` for the v2.x → v3.0
   migration. Per-command rows turn over in their Phase 2 cutover
   commit: the `v2_*` assertion is removed and the `v3_*` counterpart
   is unblocked (`#[ignore]` removed) in the same commit. CLI-output
   regression tests remain in category 2 — they assert the
   user-observable contract that DOES stay stable across the cutover.

## Naming rules

- Lowercase `snake_case`, suffix `_test.rs`.
- **No transient identifiers.** A test file name must not encode milestone
  numbers (`m1`), POC tracks (`track_g`), feature flags, sprints, branch
  numbers, or person names. These names lose meaning the moment the
  initiative ends.
- For category 1, the file name SHOULD equal the `src/internal/` module name.
  It MAY differ when the module name is too implementation-flavored to be
  recognizable from a test perspective (e.g., `write_ops` → folded into
  `event_storage_test`); when it differs, the file's module-doc comment must
  state the mapping.
- For category 3, the concept name MUST match user-facing or spec-level
  terminology (`migrate`, `operation_check`), not internal helpers.

## When to split

There is no test-count or line-count cap. Split a file only when its scope
no longer fits in a single sentence describing the module or concept it
covers. Example: if `state_change_test.rs` starts to cover both lifecycle
gating and event-emission shape — two sentences — split along that
conceptual boundary (e.g., `state_change_test.rs` for transition gating,
`state_change_emission_test.rs` for the emitted event shape). Never split
by alphabet, count, or chronology.

If no clean sub-concept split exists when a file feels unwieldy, the file
itself is probably the wrong abstraction — rename, don't shard.

## Failure modes

- A new test added to a milestone-style or otherwise transient-named file →
  reviewer rejects, asks for module/topic name.
- Two files claim the same concept → consolidate; the owner-module name
  wins. Cross-module tests move into the responsible single module's file
  unless they are genuinely cross-cutting per category 3.
- A category 3 file accumulating tests that belong to a single module → split
  out and move them to the module file.

## Acceptance tests

- `tests/m?_test.rs` and `tests/track_g_test.rs` no longer exist.
- Every remaining `tests/*.rs` matches one of the six categories above.
- `cargo test --all-targets` is green and the total `#[test]` count after the
  refactor matches the count before (refactor is move-only).
- `tests/support/README.md` reflects the new layout (no stale references to
  `m1_test.rs … m5_test.rs`).
