# CLI Regression Coverage Audit (v2.x)

Source of truth for the v2.x CLI surface freeze that gates Phase 2.
Bound by task `4w8hm98j` (RFC `7ymtc4b2` Phase 0).

The Phase 2 cutover replaces the v2 event-chain storage with the v3
snapshot tree per command. Tests that assert *user-observable* CLI
behavior (stdout, stderr, exit code, `git forum show` output) MUST stay
green across the cutover; tests that assert the underlying storage
shape MUST NOT — they are versioned per phase.

This audit:

1. maps every `internal::commands::*` module to its CLI-output coverage,
2. identifies gaps where the only existing coverage reaches into the
   library (`thread::replay_thread`, `read_event`, etc.),
3. fixes the gaps by adding `tests/cli_regression_test.rs` — a single
   file with one CLI-output-only test per module,
4. introduces a versioned storage-shape test convention
   (`tests/storage_v{2,3}_test.rs`) and seeds v2 baselines.

## Why CLI-output tests are storage-shape independent

A test like

```rust
let out = run_cli(repo, &["new", "issue", "Title"]);
let id = extract_id(&out);
let show = run_cli(repo, &["show", &id]);
assert!(show.stdout.contains("Title"));
```

depends on (1) the CLI argument grammar and (2) the rendered
`show` output. Neither changes during the v2→v3 storage cutover —
SPEC-3.0 §1.2.5 requires CLI/TUI to share one application command
layer with surface-stable output. The same test passes against the
event-chain implementation today and against the snapshot
implementation post-cutover.

A test like

```rust
let state = thread::replay_thread(&git, &id).unwrap();
assert_eq!(state.events.len(), 1);
```

reaches into a v2-specific concept (`events`). Once Phase 1 replaces
the event chain with `ThreadSnapshot`, this assertion fails by design.
Such tests live under the `tests/storage_v{2,3}_*` family and turn
over per-command in their Phase 2 cutover commit.

## Per-`commands::*` coverage map

| Module | CLI surface | CLI-output coverage (v2.x) | Storage-shape-coupled coverage | Verdict |
|---|---|---|---|---|
| `commands::bulk` | `state bulk` | `cli_state_bulk_test.rs` (4 tests, assert stdout summary line, exit code; some replay) | `cli_state_bulk_test.rs` partial (replay_thread for action-resolution check) | Pure-CLI test added at `cli_regression_test::state_bulk_summary_visible` |
| `commands::node_bulk` | `retract`, `resolve`, `reopen <node_ids>` | `cli_bulk_node_test.rs` (7 tests; mostly stdout + show) | `cli_bulk_node_test.rs` partial (replay for retract markers) | Pure-CLI test added at `cli_regression_test::node_bulk_resolve_visible_in_show` |
| `commands::repair_workflow` | `repair --workflow-violations` | none | none | DELETE per ADR-011 / Phase 4. Gap intentionally left open — see Gaps below. |
| `commands::revise` | `revise`, `revise body`, `revise node` | `cli_revise_test.rs` (3 tests, assert stdout); `cli_diff_test.rs` (covers downstream visibility) | `cli_revise_test.rs` partial (replay for body equality) | Pure-CLI test added at `cli_regression_test::revise_body_visible_in_show` |
| `commands::shorthand_say` | `comment`, `objection`, `action`, `claim`*, `question`*, `summary`*, `risk`*, `review`* (deprecated), `node add` | `cli_canonical_equiv_test.rs` (8 tests; replay-coupled); `cli_shorthand_test.rs` (1) | Most existing tests use `replay` to compare equivalence | Pure-CLI test added at `cli_regression_test::comment_visible_in_show` |
| `commands::state` | `close`, `pend`, `accept`, `propose`, `deprecate`, `reject`, `withdraw`, `state`, `reopen` (no-nodes) | `cli_canonical_equiv_test.rs` (5 state shorthands; replay-coupled); `cli_state_bulk_test.rs` self-loop coverage | All canonical-equiv tests reach into replay | Pure-CLI test added at `cli_regression_test::close_visible_in_show` |
| `commands::thread_new` | `new`, `thread new` | `cli_thread_new_test.rs` (13 tests; replay-coupled) | All replay-coupled | Pure-CLI test added at `cli_regression_test::thread_new_visible_in_show` |
| `commands::shared` | n/a (helpers) | covered transitively via every command | n/a | No new test required — exercised by every other test |

After the additions in `tests/cli_regression_test.rs`, every module
listed (except the DELETE row) has at least one regression test that
asserts only user-observable CLI output.

## Gaps

### `commands::repair_workflow` — deferred deletion

`repair --workflow-violations` is on the Phase 4 DELETE list per
ADR-011 and `doc/internal/3.0-removal-plan.md` (the file
`commands/repair_workflow.rs` is classified `DELETE`; its peer
`internal/repair_workflow.rs` is also `DELETE`). Adding a regression
test for code we are about to delete would be busywork.

The Phase 4 sweep removes the `Commands::Repair` arm together with the
two `repair*` modules; there is no surviving CLI surface to lock.
Documenting this gap explicitly here so the absence is intentional, not
oversight.

### Existing tests that mix CLI output with library replay

Pre-existing `cli_*_test.rs` files (e.g. `cli_thread_new_test.rs`,
`cli_canonical_equiv_test.rs`) call `thread::replay_thread` to verify
state. They are *not* removed:

- They remain useful regression coverage for the v2 implementation.
- During each command's Phase 2 cutover commit, the in-test `replay`
  call is replaced with a snapshot read OR the assertion is moved into
  the matching `tests/storage_v3_<cmd>_test.rs` file.
- Until then, the new `tests/cli_regression_test.rs` provides the
  cutover-stable contract. The old tests act as additional safety net
  against silent regressions in v2.x while the work is in flight.

### TUI

Out of scope per the task body. TUI regression coverage is a separate
Phase 2 concern (RFC `7ymtc4b2` Phase 2 mutation cutover).

## Versioned storage-shape tests

The task specifies "tests/storage/v2_*.rs (pre) and tests/storage/v3_*.rs
(post)". Cargo's default integration-test layout treats every file
directly under `tests/` as a separate test crate; files in
`tests/<subdir>/` are not picked up unless wired via `#[path]` or a
sibling entrypoint. To stay within Cargo conventions while keeping
the v2/v3 grouping the issue asks for, this branch uses **flat naming
with a `storage_` prefix**:

- `tests/storage_v2_test.rs` — v2.x event-chain storage-shape baselines.
- `tests/storage_v3_test.rs` — v3.0 snapshot-tree storage-shape tests
  (initially `#[ignore]`-gated stubs; per-command tests turn on as their
  Phase 2 cutover lands).

The prefix-grouping is documented in `doc/spec/TEST-POLICY.md` as a
new test category; the cutover discipline is the same as the issue's
original wording (`v2_*` removed in the same commit that turns on the
matching `v3_*`).

## Cutover discipline (per Phase 2 commit)

For each command in slot 1-10 of the cutover order
(`doc/internal/main-rs-audit.md`):

1. The cutover commit rewires the command from `event_chain → snapshot`.
2. The same commit removes the corresponding test in
   `tests/storage_v2_test.rs` (or whichever per-command v2 storage file
   has been split out by then) and adds the v3 counterpart in
   `tests/storage_v3_test.rs`.
3. `tests/cli_regression_test.rs` is untouched — its assertions still
   pass because they assert user-observable behavior, not storage
   shape.
4. Pre-existing `cli_*_test.rs` `replay`-coupled tests are migrated:
   either rewritten to assert via `git forum show` output, or moved
   into the matching `tests/storage_v3_<cmd>_test.rs`.

## Acceptance check (this task)

```
$ cargo test --test cli_regression_test
running 6 tests
... (all pass)

$ cargo test --test storage_v2_test
running 1 test
... (passes against v2.x tip)

$ cargo test --test storage_v3_test
running 1 test
test v3_thread_create_snapshot ... ignored
```

`cargo test` overall stays green; no existing test regressed.
