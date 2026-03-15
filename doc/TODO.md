# MVP TODO

This document reorganizes [doc/spec/MVP_SPEC.md](./spec/MVP_SPEC.md) into milestones.
When spec and implementation diverge, the spec wins.

## Finish line

- [x] `git forum init` works in an empty Git repository
- [x] `issue` and `rfc` can be created
- [x] typed discussion nodes can be added
- [x] first-batch shorthand discussion commands exist
- [x] policy-driven state validation works
- [x] evidence and links can be attached
- [x] branch binding for issues works
- [x] `git forum show` displays open objections / open actions / latest summary / timeline
- [x] `git forum verify` evaluates the minimum required guards
- [x] `git forum reindex` rebuilds the local index
- [x] `git forum tui` supports list/detail/basic create flows
- [ ] minimal semantic merge is implemented
- [x] the project builds and tests on stable Rust
- [x] legacy `decision` and `run` surfaces are removed or clearly demoted from the preferred UX

## Test harness baseline

Across all milestones:

- [x] unit tests cover replay / state machine / policy / guard / merge / index / search
- [x] integration tests use temporary Git repositories
- [x] clock and ID generation are replaceable
- [ ] stable snapshot coverage is limited to `show`, `verify`, export, and TUI render
- [x] CI baseline is `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test`

Planned layout:

- [x] `tests/support/` for temp repo, env isolation, Git helpers, clock/id helpers, CLI helpers, TUI helpers
- [ ] `tests/fixtures/` for import/export and merge inputs
- [ ] `tests/snapshots/` for stable output checks

## Milestone 0: Rust bootstrap

Goal:
Keep a stable Rust foundation for continued work.

Done when:

- [x] `Cargo.toml` and `src/` exist
- [x] single-binary CLI entrypoint exists
- [x] fmt / clippy / test pass
- [x] base error/config/CLI skeleton exists
- [x] test helper scaffolding exists

Verification:

```bash
cargo build
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Milestone 1: Repository and event foundation

Goal:
Store forum data in Git and reconstruct state from append-only events.

Includes:

- [x] `.forum/` and `.git/forum/` initialization
- [x] `git forum init`
- [x] thread and index ref namespaces
- [x] event persistence as Git commits
- [x] thread replay
- [x] `git forum doctor`
- [x] `git forum reindex`
- [x] isolated temporary Git repo helpers

Exit criteria:

- [x] empty repo init works
- [x] latest thread state can be replayed from refs
- [x] local index can be rebuilt from Git

Verification:

```bash
cargo run -- init
cargo run -- doctor
cargo run -- reindex
```

## Milestone 2: RFC and issue lifecycle

Goal:
Make `rfc` and `issue` the only preferred thread kinds.

Includes:

- [x] issue / rfc state machines
- [x] human-readable IDs
- [x] `git forum issue new`
- [x] `git forum rfc new`
- [x] thread body on create
- [x] `git forum ls`, `issue ls`, `rfc ls`
- [x] `git forum show`
- [x] remove or demote legacy `decision` thread kind from CLI, templates, docs, and tests
- [x] remove `DEC-*` assumptions from examples and fixtures

Exit criteria:

- [x] issue and rfc can be created and displayed
- [x] `show` output matches replay state
- [x] preferred docs no longer depend on `decision`

Verification:

```bash
cargo run -- issue new "First issue" --body "Problem statement"
cargo run -- rfc new "First RFC" --body-file ./tmp/rfc-body.md
cargo run -- ls
cargo run -- show RFC-0001
```

## Milestone 3: Structured discussion and approvals

Goal:
Make typed discussion the main work surface.

Includes:

- [x] typed nodes for claim / question / objection / alternative / evidence / summary / action / risk / assumption
- [x] `git forum say`
- [x] `git forum revise`
- [x] `git forum retract`
- [x] `git forum resolve`
- [x] `git forum reopen`
- [x] open objection / open action tracking
- [x] `.forum/policy.toml` parser
- [x] `one_human_approval`, `at_least_one_summary`, `no_open_objections`, `no_open_actions`
- [x] `git forum state`
- [x] `git forum state --resolve-open-actions`
- [x] `git forum state bulk`
- [x] `git forum verify`
- [x] `git forum policy lint`
- [x] `git forum policy check`
- [x] first-batch shorthand commands:
  - [x] `git forum claim`
  - [x] `git forum question`
  - [x] `git forum objection`
  - [x] `git forum summary`
  - [x] `git forum action`
  - [x] `git forum risk`
- [ ] decide whether `alternative`, `evidence`, and `assumption` also deserve shorthand commands
- [ ] fuller role-based enforcement for `can_say` and `can_transition`

Exit criteria:

- [x] typed discussion can be added
- [x] objections and actions can be resolved / reopened
- [x] issue close can be blocked by open actions
- [x] RFC acceptance can require summary and human approval
- [x] preferred UX does not require `say --type` for common node types

Verification:

```bash
cargo run -- claim RFC-0001 "Needed for compatibility."
cargo run -- objection RFC-0001 "Benchmarks are missing."
cargo run -- policy check RFC-0001 --transition under-review->accepted
cargo run -- verify RFC-0001
```

## Milestone 4: Evidence, links, and branch-oriented implementation

Goal:
Connect accepted RFCs to implementation issues and code evidence.

Includes:

- [x] `git forum evidence add`
- [x] commit / file / hunk / test / benchmark / doc / thread / external evidence
- [x] `git forum link`
- [x] timeline and detail rendering for evidence / relation
- [x] `git forum branch bind`
- [x] `git forum branch clear`
- [x] branch column and branch filtering in lists
- [ ] tighten the RFC -> issue implementation workflow in examples and docs
- [x] remove or demote legacy `run` / provenance UX from docs and preferred examples

Exit criteria:

- [x] evidence can be attached
- [x] thread links can be followed from detail views
- [x] issues can bind to branches
- [ ] preferred workflow reads as "accepted RFC -> linked issue -> branch-bound implementation"

Verification:

```bash
cargo run -- link ISSUE-0001 RFC-0001 --rel implements
cargo run -- branch bind ISSUE-0001 feat/parser-rewrite
cargo run -- evidence add ISSUE-0001 --kind test --ref tests/parser.rs
cargo run -- show ISSUE-0001
```

## Milestone 5: Index, search, and TUI

Goal:
Provide practical read performance and a lightweight local UI.

Includes:

- [x] SQLite index
- [x] full `reindex`
- [x] auto-create fallback when index is missing
- [x] lexical search over thread title / thread body / current node body / kind / state
- [x] `git forum tui`
- [x] list and detail views
- [x] basic filter
- [x] node create from thread detail and node detail
- [x] thread create from list view
- [x] thread link create from TUI
- [x] basic mouse support:
  - [x] click to open rows
  - [x] wheel scroll
  - [x] click submit rows
- [ ] expanded mouse support:
  - [ ] click dropdown / target list / filter / back actions
  - [ ] decide whether hover / drag / range select are worth supporting
- [x] TUI render tests

Exit criteria:

- [x] index rebuild works
- [x] thread-oriented search shows matching nodes
- [x] TUI supports list/detail/filter/minimum create flows
- [ ] TUI keyboard and mouse flows are coherent enough for everyday use

Verification:

```bash
cargo run -- reindex
cargo run -- tui
cargo run -- tui RFC-0001
cargo test index
```

## Milestone 6: Merge, import/export, and release hardening

Goal:
Close the remaining MVP gaps and align implementation with the new docs.

Includes:

- [ ] auto-merge of concurrent new `say` events
- [ ] auto-merge of evidence additions
- [ ] auto-merge of summary additions
- [ ] conflict detection for concurrent terminal state changes
- [ ] conflict detection for concurrent `resolve` / `reopen`
- [ ] synthetic merge event and unresolved-conflict rendering
- [ ] GitHub issue -> `issue` import
- [ ] markdown -> `rfc` import with manual cleanup
- [ ] `issue` export
- [ ] `rfc` export
- [x] remove or fully demote legacy `decision` CLI surface
- [x] remove or fully demote legacy `run` / provenance CLI surface
- [ ] README / MANUAL / spec / examples stay aligned
- [ ] acceptance criteria final pass

Exit criteria:

- [ ] minimal semantic merge exists
- [ ] import / export minimum requirements are met
- [ ] preferred docs match the shipped workflow
- [ ] finish line items are all complete

Verification:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

## Scope guard

The MVP still excludes:

- Web UI
- central server
- real-time collaboration
- mandatory AI provenance as a gate
- separate high-level agent-only workflows
- rich full-edit TUI
- automatic patch application
