# Roadmap

Last updated: 2026-03-22

## Completed

The following capabilities are implemented and tested:

### Thread model

- Four thread kinds: `rfc`, `issue`, `dec`, `task` with full state machines
- Event-sourced thread replay with append-only Git commits
- State machine validation for all thread kinds (RFC-0021)
- Concurrency safety via atomic ref updates (compare-and-swap)
- Semantic merge for concurrent non-conflicting events (ISSUE-0006, ISSUE-0021, ISSUE-0022)
- Branch bind/clear for implementation issues
- Thread-to-thread links with `--link-to` and `--rel`
- Retroactive thread creation from commits (`--from-commit`) or existing threads
  (`--from-thread`)

### Discussion nodes

- Ten typed nodes: claim, question, objection, evidence, summary, action, risk, review,
  alternative, assumption
- Shorthand CLI commands for seven common node types (claim, question, objection, summary,
  action, risk, review)
- Node lifecycle: revise, retract, resolve, reopen (multi-ID with inline failure reporting)
- Reply chains between nodes
- Thread body revision with `--incorporates`
- Inline node flags on thread creation: `--claim`, `--question`, `--objection`, `--action`,
  `--risk`, `--summary` (ISSUE-0052)

### Evidence and provenance

- Evidence attachment: commit, file, hunk, test, benchmark, doc, thread, external
- Bulk evidence add with multiple `--ref` values (ISSUE-0028)
- Commit OID resolution
- Evidence table in SQLite index for fast import dedup lookups (ISSUE-0101)

### Policy and guards

- Policy guard evaluation on state transitions
- Operation checks: creation rules, node rules, revise rules, evidence rules
- Error/warning severity model with `--force` flag and strict mode (RFC-0018)
- Policy lint: state validation, multi-kind transition notes, invalid transition detection,
  remediation hints (ISSUE-0091), allow-list gap detection per thread kind (ISSUE-0095)
- State transition shorthands: `close`, `pend`, `accept`, `propose`, `reject`, `deprecate`
  (ISSUE-0033)
- `--comment` on state transitions (ISSUE-0066)

### CLI and UX

- Repository init, doctor (refs, templates, index integrity), reindex
- `show` (with compact next-states and state diagram), `node show`, `status`, `verify`, `show --what-next` (with operation checks), `policy show` / `lint` / `check` (ISSUE-0110)
- `--help-llm` at any subcommand level with per-command contextual help (ISSUE-0034, ISSUE-0050)
- Structured `--help` output with grouped categories (RFC-0024)
- `--edit` flag for interactive body composition via `$EDITOR` (ISSUE-0072)
- `revise` defaults to body revision; `revise body` and `revise node` still work (ISSUE-0063)
- Body revision diff: `diff` command with `--rev N` and `--rev N..M` (ISSUE-0094)
- Post-action next-actions hints printed to stderr (ISSUE-0048)
- Advisory commit-msg hook: validates thread ID references, auto-installed on init (RFC-0020)

### GitHub interop

- GitHub issue import via `gh` CLI: `import github-issue` (ISSUE-0099)
- GitHub issue export: `export github-issue` (ISSUE-0008)

### TUI

- List, detail, node detail views with sort, filter, mouse, color coding
- Thread/node/link creation from TUI
- Markdown rendering toggle (`m`)
- Full-screen select mode (`S`) for pane-scoped text selection
- Performance telemetry, replay cache, and incremental refresh (RFC-0017 Phases 0-2)

### Infrastructure

- Lexical search over SQLite index
- Git worktree support (ISSUE-0026)
- Snapshot and integration test infrastructure
- E2E multi-agent test harness with Claude Code adapter and worktree-per-actor setup
  (RFC-0003, ISSUE-0042 through ISSUE-0047)

## Open issues

Active issues awaiting implementation:

### CLI improvements

- ISSUE-0104 — Add `say` as alias for `node add` subcommand
- ISSUE-0102 — Support multiple `--link-to` with per-link `--rel` values
- ISSUE-0108 — Support batch `show` for multiple thread IDs
- ISSUE-0106 — Add `--json` output mode to `show` command
- ISSUE-0109 — Suggest shorthand commands on unrecognized subcommand
- ISSUE-0105 — Suppress init warning when repo is functional
- ISSUE-0107 — Detect non-interactive stdin and reject `--edit` with actionable error

### Policy and verification

- ISSUE-0097 — Kind-scoped guard keys to prevent cross-kind collisions
- ~~ISSUE-0081~~ — Superseded by ISSUE-0110 (implemented)
- ISSUE-0093 — Make `verify` distinguish PASS, BLOCKED, and NOT APPLICABLE

### Display and output

- ~~ISSUE-0070~~ — Superseded by ISSUE-0110 (implemented)
- ISSUE-0096 — Expand structured workflow outputs around status, what-next, and verify

### Templates and scaffolding

- ISSUE-0098 — TASK templates should scaffold body sections automatically
- ISSUE-0103 — Body section linter reports false positive on markdown tables

## Draft RFCs

Design proposals not yet proposed/accepted:

- RFC-0001 — Auto-propagate commit evidence to linked threads
- RFC-0002 — Changelog / release report command
- RFC-0019 — Web UI: embedded HTTP server via `git forum serve`
- RFC-0022 — Advisory workflow features: brief, scope tracking, spec-delta warnings,
  escalation hints

## Future considerations

The following are not yet tracked as issues. They represent directions for exploration.

### Enhanced TUI editing

- Inline state transitions with guard feedback in the TUI
- Thread body editing directly in the TUI
- Richer multiline editor with syntax highlighting

### Richer search

- Faceted search (filter by kind, status, actor, date range)
- Full-text search with ranking
- Embedding-based semantic search (long-term)

### Multi-repo and remote workflows

- Push/fetch of forum refs between clones
- Cross-repository thread references
- Conflict resolution UX for divergent forum histories after fetch

### Cryptographic signing

- Extend the approval mechanism with GPG or SSH signatures
- Verify approval signatures in `verify` and `policy check`

### Notification and subscription

- Watch specific threads or node types
- Post-event shell hooks (beyond the shipped commit-msg hook)

### Advanced policy

- Quorum-based approvals (e.g., 2 of 3 maintainers)
- Time-based guards (e.g., minimum review period)
- Escalation rules for unresolved objections

### Metrics and reporting

- Thread velocity and cycle time
- Objection resolution rate
- Per-actor contribution summaries
