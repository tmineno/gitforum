# Roadmap

Last updated: 2026-03-31

## Completed

The following capabilities are implemented and tested:

### Thread model

- Four thread kinds: `rfc`, `ask` (issue), `dec`, `job` (task) with full state machines
- 3-letter kind prefixes for consistent ID width: RFC, ASK, DEC, JOB (RFC-0031)
- Opaque content-addressed thread IDs via sha256 for conflict-free allocation (RFC-0030)
- Event-sourced thread replay with append-only Git commits
- State machine validation for all thread kinds (RFC-0021)
- Concurrency safety via atomic ref updates (compare-and-swap)
- Semantic merge for concurrent non-conflicting events (ISSUE-0006, ISSUE-0021, ISSUE-0022)
- Branch bind/clear for implementation issues
- Thread-to-thread links with `--link-to` and `--rel`
- Multiple `--link-to` with per-link `--rel` values (ISSUE-0102)
- Retroactive thread creation from commits (`--from-commit`) or existing threads
  (`--from-thread`)
- Case-insensitive thread lookup and repair command (ISSUE-0143, ISSUE-0142)

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
- In-place node type change via `retype` command with operation policy checks (ISSUE-0152)
- Lower minimum node ID prefix from 8 to 4 characters (ISSUE-0149)

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
- Kind-scoped guard keys to prevent cross-kind collisions (ISSUE-0097)
- State transition shorthands: `close`, `pend`, `accept`, `propose`, `reject`, `deprecate`
  (ISSUE-0033)
- `--comment` on state transitions (ISSUE-0066)
- `verify` distinguishes PASS, BLOCKED, and NOT APPLICABLE; reframed as preflight check
  (ISSUE-0093, ISSUE-0138)
- Discoverable policy and state transitions in `show` and `policy` commands (ISSUE-0110)
- Structured workflow outputs for status, what-next, and verify (ISSUE-0096)

### CLI and UX

- Repository init with default actor prompt and configurable commit identity
  (ISSUE-0130, ISSUE-0127); suppressed init warning when refs exist (ISSUE-0105)
- Doctor (refs, templates, index integrity) with collapsed replays, summary, `--verbose`;
  auto-configures forum fetch refspec on init
- `show` (with compact next-states, state diagram, copy-pastable follow-up commands for open
  objections/actions/conversations (ISSUE-0146)), batch `show` for multiple IDs (ISSUE-0108)
- `node show`, `status`, `verify`, `show --what-next` (with operation checks),
  `policy show` / `lint` / `check` (ISSUE-0110)
- `log` command for history-oriented thread view (ISSUE-0145)
- `shortlog` command for release-note summaries (RFC-0002)
- `purge` command for hard-delete of event content with `--node` shorthand
  (ISSUE-0132, ISSUE-0137)
- `--help-llm` at any subcommand level with per-command contextual help (ISSUE-0034,
  ISSUE-0050); two-tier `--help-llm` / `--help-llm full` (RFC-0025)
- Structured `--help` output with grouped categories (RFC-0024)
- `--edit` flag with non-interactive stdin detection and actionable error (ISSUE-0072,
  ISSUE-0107)
- `--status` filter on `ls` subcommand; column width clamping and title truncation
  (ISSUE-0150)
- `--compact` slimmed to triage-oriented view (ISSUE-0147)
- `revise` defaults to body revision; `revise body` and `revise node` still work (ISSUE-0063)
- Body revision diff: `diff` command with `--rev N` and `--rev N..M` (ISSUE-0094)
- `--body -` rejects empty stdin with actionable error (ISSUE-0144)
- Suggest shorthand commands on unrecognized subcommand (ISSUE-0109)
- Post-action next-actions hints printed to stderr (ISSUE-0048)
- Advisory commit-msg hook: validates thread ID references, auto-installed on init (RFC-0020)
- Trust model documented in init and help surfaces; command-role guide (ISSUE-0139, ISSUE-0140)
- Retract documented as soft-delete with stderr warning (ISSUE-0129)

### GitHub interop

- GitHub issue import via `gh` CLI: `import github-issue` with GitHub usernames stored as
  actor references (ISSUE-0099, ISSUE-0128)
- GitHub issue export: `export github-issue` (ISSUE-0008)

### Security and privacy hardening

- Exhaustive match in `apply_event` replacing catch-all (ISSUE-0120)
- Event field size validation to prevent DoS (ISSUE-0121)
- Deduplicate approval actors to prevent forged duplicates (ISSUE-0119)
- Actor impersonation trust model documented (ISSUE-0118)
- `--raw-field` for `gh api` body updates to prevent injection (ISSUE-0125)
- Descriptive expect replacing stdin unwrap (ISSUE-0126)
- Configurable commit identity for forum commits (ISSUE-0127)
- SQLite index permissions set to 0o600 on Unix (ISSUE-0131)
- Thread IDs hashed in perf logs to prevent access pattern leakage (ISSUE-0133)
- Init prints directory name instead of absolute path (ISSUE-0134)

### TUI

- List, detail, node detail views with sort, filter (all 4 thread kinds), mouse, color coding
- Thread/node/link creation from TUI
- Markdown rendering toggle (`m`) with fixed table, link, image, strikethrough rendering
- Full-screen select mode (`S`) for pane-scoped text selection
- `t` key toggles horizontal/vertical split (ISSUE-0151)
- PageUp/PageDown/Home/End key support (ISSUE-0117)
- Yank/confirm-discard support
- In-app error catching with flash and CLI next-step hint (ISSUE-0141)
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

- ISSUE-0106 — Add `--json` output mode to `show` command

## Draft RFCs

Design proposals not yet proposed/accepted:

- RFC-0001 — Auto-propagate commit evidence to linked threads (proposed)
- RFC-0019 — Web UI: embedded HTTP server via `git forum serve`
- RFC-0022 — Advisory workflow features: brief, scope tracking, spec-delta warnings,
  escalation hints
- RFC-6m4kap23 — Spawn Claude Code to fix selected issue (`git forum fix`)

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
