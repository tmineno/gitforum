# Roadmap

Last updated: 2026-05-02

The authoritative target model is [doc/spec/SPEC-2.0.md](./spec/SPEC-2.0.md). This document
groups shipped capability against the 2.0 surface, lists open work, and records exploratory
directions that are not yet tracked as issues.

## Completed

The following capabilities are implemented and tested in the 2.0 surface (or carried over from
1.x where SPEC-2.0 inherits them unchanged).

### Thread model

- Threads with `lifecycle` (`proposal` / `execution` / `record`) and free-form `tags`,
  replacing the four 1.x kinds with one entity (SPEC-2.0 §2.2 / §2.3)
- Stable kind presets `new rfc` / `new dec` / `new task` / `new issue` / `new bug` mapped onto
  the conventional (lifecycle, tags) pairs (SPEC-2.0 §9.1)
- Canonical scriptable form `git forum thread new --lifecycle <L> --tag <T>...`
- `@XXXXXXXX` display form / bare 8-char base36 storage form for thread IDs (SPEC-2.0 §6)
- Type-marker omission at CLI input — every position accepts the bare token without `@`
  (SPEC-2.0 §6.1.1)
- One-shot 1.x → 2.0 migration via `git forum migrate` (rewrites refs to bare-token form,
  emits `facet_set` events, remaps states, preserves links, rewrites legacy node types to
  `comment` with `legacy_subtype`); legacy IDs and kind-keyed policy keys auto-resolve
- Unified state machine — `draft`, `open`, `working`, `review`, `done`, `rejected`,
  `withdrawn`, `deprecated` — gated per-lifecycle (SPEC-2.0 §3.1)
- Event-sourced thread replay with append-only Git commits
- Concurrency safety via atomic ref updates within a clone; cross-clone divergence reported
  informationally and reconciled with plain `git push` / `git fetch` on `refs/forum/*`
  (SPEC-2.0 §8.2)
- Branch bind / clear for execution threads
- Thread-to-thread links with `--link-to` and `--rel`; multiple `--link-to` with per-link
  `--rel` values
- Retroactive thread creation from commits (`--from-commit`) or existing threads
  (`--from-thread`)
- Case-insensitive thread lookup and repair command
- Advisory one-hop `--rel implements` view via `git forum show <ID> --tree` (SPEC-2.0 §2.1)
- Read-only single-thread digest for AI agents and scripts: `git forum brief <ID> [--json]`
  with a stable v1 schema (RFC `5wf2v8hv`)

### Discussion nodes

- Four canonical typed nodes — `comment`, `approval`, `objection`, `action` — chosen by
  protocol effect (SPEC-2.0 §2.5)
- Shorthand CLI commands for `comment`, `objection`, `action`; `approval` is appended via the
  `--approve <ACTOR>` flag on state-change commands
- Legacy 1.x types (`claim`, `question`, `summary`, `risk`, `review`, `alternative`,
  `assumption`) accepted as deprecated aliases for `comment` for one minor release; migrated
  events preserve the original type in `legacy_subtype`
- Node lifecycle: revise, retract, resolve, reopen (multi-ID with inline failure reporting)
- Reply chains between nodes
- Thread body revision with `--incorporates`
- Inline node flags on thread creation: `--objection`, `--action` (canonical) plus
  `--claim` / `--question` / `--summary` / `--risk` (deprecated aliases that write `comment`
  nodes with `legacy_subtype`)
- In-place node type change via `retype` command with operation policy checks
- Lower minimum node ID prefix from 8 to 4 characters

### Evidence and provenance

- Evidence attachment: commit, file, hunk, test, benchmark, doc, thread, external
- Bulk evidence add with multiple `--ref` values
- Commit OID resolution
- Evidence table in SQLite index for fast import dedup lookups

### Policy and guards

- **Lifecycle / tag-scoped guards** with boolean facet expressions (e.g.
  `lifecycle=proposal AND tag=cross-cutting : review->done`); 1.x kind-keyed guards
  auto-rewrite to lifecycle keys at config-load time with a deprecation warning
  (SPEC-2.0 §7.1 / §10.4)
- Operation checks: `creation_rules.<lifecycle>[.tag.<name>]`, node rules, revise rules,
  evidence rules with most-specific-match resolution (SPEC-2.0 §7.2)
- Guard predicates currently understood: `no_open_objections`, `no_open_actions`,
  `one_human_approval`, `has_commit_evidence` (the 1.x `at_least_one_summary` predicate is
  no longer shipped — `summary` is no longer a node type)
- Error / warning severity model with `--force` flag and strict mode
- Policy lint: state validation, multi-lifecycle transition notes, invalid transition
  detection, remediation hints, allow-list gap detection
- State transition shorthands: `close`, `pend`, `accept`, `propose`, `reject`, `withdraw`,
  `deprecate` — each lifecycle-aware (SPEC-2.0 §9.3)
- `--comment` on state transitions
- `verify` distinguishes PASS, BLOCKED, and NOT APPLICABLE; reframed as a single-thread
  preflight (SPEC-2.0 §9.4)
- `verify` and `doctor` surface cross-thread advisories without gating any operation
  (CORE-VALUE.md "Advisories")
- Discoverable policy and state transitions in `show` and `policy` commands
- Structured workflow outputs for status, what-next, and verify

### CLI and UX

- Repository init with default actor prompt and configurable commit identity; suppressed init
  warning when refs exist
- Doctor (refs, templates, index integrity, fetch refspec, observed remote divergence) with
  collapsed replays, summary, `--verbose`; auto-configures forum fetch refspec on init
- `show` (with compact next-states, state diagram, copy-pastable follow-up commands for open
  objections / actions / conversations), batch `show` for multiple IDs
- `node show`, `status`, `verify`, `show --what-next` (with operation checks),
  `policy show` / `lint` / `check`
- `log` command for history-oriented thread view
- `shortlog` command for release-note summaries
- `purge` command for hard-delete of event content with `--node` shorthand
- `--help-llm` at any subcommand level with per-command contextual help; two-tier
  `--help-llm` / `--help-llm full`
- Structured `--help` output with grouped categories
- `--edit` flag with non-interactive stdin detection and actionable error
- `--status` filter on `ls` subcommand; column width clamping and title truncation
- `--compact` slimmed to triage-oriented view
- `revise` defaults to body revision; `revise body` and `revise node` still work
- Body revision diff: `diff` command with `--rev N` and `--rev N..M`
- `--body -` rejects empty stdin with actionable error
- Suggest shorthand commands on unrecognized subcommand
- Post-action next-actions hints printed to stderr
- Advisory commit-msg hook: validates thread ID references (both `@`-form and legacy
  `KIND-…`), auto-installed on init
- Post-checkout hook: worktree auto-init and index blob repair, auto-installed on init
- Consistent 16-char OID truncation across all CLI output
- Trust model documented in init and help surfaces; command-role guide
- Retract documented as soft-delete with stderr warning

### GitHub interop

- GitHub issue import via `gh` CLI: `import github-issue` with GitHub usernames stored as
  actor references
- GitHub issue export: `export github-issue`

### Search

- Lexical search over a SQLite index with a `lifecycle` column and `tags` join table
- Legacy `kind:<name>` predicates auto-translate to `lifecycle:` / `tag:` form for one minor
  release with a deprecation warning (SPEC-2.0 §12)

### Security and privacy hardening

- Exhaustive match in `apply_event` replacing catch-all
- Event field size validation to prevent DoS
- Deduplicate approval actors to prevent forged duplicates
- Actor impersonation trust model documented
- `--raw-field` for `gh api` body updates to prevent injection
- Descriptive expect replacing stdin unwrap
- Configurable commit identity for forum commits
- SQLite index permissions set to 0o600 on Unix
- Thread IDs hashed in perf logs to prevent access pattern leakage
- Init prints directory name instead of absolute path

### TUI

- List, detail, node detail views with sort, filter (by lifecycle, tag, status), mouse, and
  color coding (lifecycle / status / node type)
- Thread / node / link creation from the TUI; create-thread form takes `lifecycle` plus
  comma-separated `tags` (validated against the SPEC-2.0 §2.3.5 grammar at submit time)
- Markdown rendering toggle (`m`) with fixed table, link, image, strikethrough rendering
- Full-screen select mode (`S`) for pane-scoped text selection
- `t` key toggles horizontal/vertical split
- PageUp/PageDown/Home/End key support
- Yank/confirm-discard support
- In-app error catching with flash and CLI next-step hint
- Performance telemetry, replay cache, and incremental refresh
- Linked-children advisory panel on thread detail (one-hop incoming `implements`)

### Infrastructure

- Git worktree support with auto-init via post-checkout hook
- Snapshot and integration test infrastructure

## Open issues

Active issues awaiting implementation:

### CLI improvements

- thread `g0nh5fjf` — Add `--json` output mode to `show` command

## Draft RFCs

Design proposals not yet open / done:

- thread `sb7fmsjj` — Auto-propagate commit evidence to linked threads (open)
- thread `fq5xcnr8` — Web UI: embedded HTTP server via `git forum serve`
- thread `bg7tojsh` — Advisory workflow features: brief, scope tracking,
  spec-delta warnings, escalation hints
- thread `6m4kap23` — Spawn Claude Code to fix selected issue (`git forum fix`)

## Future considerations

The following are not yet tracked as issues. They represent directions for exploration; some
will need an ADR / RFC before any implementation.

### Enhanced TUI editing

- Inline state transitions with guard feedback in the TUI
- Thread body editing directly in the TUI
- Richer multiline editor with syntax highlighting

### Richer search

- Full-text search with ranking
- Faceted search (filter by lifecycle, tag, status, actor, date range)
- Embedding-based semantic search (long-term)

### Cross-clone observability

git-forum delegates distribution to plain Git on `refs/forum/*` by design (SPEC-2.0 §8.2;
CORE-VALUE.md non-goal §3) — there is no plan to ship a `git forum push` or `git forum fetch`
command or a bespoke conflict-resolution protocol. The remaining open question is what
read-only **observability** the tool should provide on top of the Git workflow:

- Better `doctor` advisories on observed divergence between local and remote `refs/forum/*`
- Read-only views (`ls --remote`, `log --remote`, `diff` against a fetched ref) over forum
  data living in another remote without mutating it
- UX hints that point users at the standard Git resolution flow when push / fetch reports
  a non-fast-forward

### Cryptographic signing

- Extend the approval mechanism with GPG or SSH signatures
- Verify approval signatures in `verify` and `policy check`

### Notification and subscription

- Watch specific threads or node types
- Post-event shell hooks (beyond the shipped commit-msg hook)

### Advanced policy

- Quorum-based approvals (e.g., 2 of 3 maintainers)
- Time-based guards (e.g., minimum review period)
- Multi-tag combiners for operation checks (field-level union with explicit `OR` / `MAX`
  semantics) — deferred per SPEC-2.0 §7.2 until dogfood evidence shows the simple
  most-specific-match resolution is insufficient
- Tag-vocabulary discipline (registry, conventional list, deprecation, lint) — deferred
  per SPEC-2.0 §2.3.5 / Appendix A.3 until language drift is observed

### Metrics and reporting

- Thread velocity and cycle time
- Objection resolution rate
- Per-actor contribution summaries
