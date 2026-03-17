# Roadmap

Last updated: 2026-03-17

## Completed

The following capabilities are implemented and tested:

- Repository init, doctor, reindex
- RFC and issue thread creation with body, branch binding, and link-at-create
- Event-sourced thread replay
- Typed discussion nodes (claim, question, objection, alternative, evidence, summary, action,
  risk, assumption, review)
- Shorthand CLI commands for common node types (claim, question, objection, summary, action, risk,
  review)
- Node lifecycle: revise, retract, resolve, reopen, reply chains
- Thread body revision with `--incorporates`
- State machine validation for RFC and issue (including `rejected` state for issues)
- Policy guard evaluation on state transitions
- Evidence attachment with commit OID resolution
- Thread-to-thread links
- Branch bind/clear
- `show`, `node show`, `status`, `verify`, `policy lint`, `policy check`
- Lexical search over SQLite index
- TUI: list, detail, node detail, create thread/node/link, sort, filter, mouse, color
- Concurrency safety via atomic ref updates (compare-and-swap)
- Snapshot and integration test infrastructure
- Git worktree support (ISSUE-0026)
- State transition shorthand commands: `issue close`, `issue reopen`, `issue reject`, `rfc propose`,
  `rfc accept` (ISSUE-0033)
- `--link-to` and `--comment` flags on state transitions (ISSUE-0027, ISSUE-0036)
- `--from-commit` flag for retroactive thread creation from commits (ISSUE-0030)
- Bulk evidence add with multiple `--ref` values (ISSUE-0028)
- `list` alias for `ls` (ISSUE-0032)
- `--help-llm` works at any subcommand level (ISSUE-0034)
- Quick-reference cheat sheet in `--help-llm` output (ISSUE-0035)
- Datetime display (HH:MM) in thread listings (ISSUE-0029)
- Titles starting with `--` accepted in thread creation (ISSUE-0031)

## In progress

### Role-based policy enforcement

Guard rules on transitions are enforced, but `can_say` and `can_transition` role restrictions
defined in `.forum/policy.toml` are parsed without being checked at runtime.

- ISSUE-0002 — Enforce role-based can_say and can_transition policy rules
- ISSUE-0023 — Enforce can_say role policy in say and shorthand commands
- ISSUE-0024 — Enforce can_transition role policy in state command

### Semantic merge

Concurrent writes to the same thread currently fail with a compare-and-swap error. Semantic merge
would automatically resolve non-conflicting concurrent writes and surface true conflicts.

- ISSUE-0006 — Implement semantic merge for concurrent events
- ISSUE-0021 — Auto-merge concurrent non-conflicting events (say, evidence, summaries)
- ISSUE-0022 — Detect and surface conflicting concurrent events (state changes, resolve/reopen)

### Remaining shorthand commands

Three node types lack dedicated shorthand commands.

- ISSUE-0001 — Add shorthand commands for alternative, evidence, and assumption node types

### Documentation alignment

- ISSUE-0009 — Align README/MANUAL/spec/examples with shipped workflow

## Planned

### Import / export

Interoperability with external systems.

- ISSUE-0007 — Import: GitHub issue and markdown RFC
- ISSUE-0008 — Export: issue and RFC to markdown or tracker-friendly format

### Completion tracking

- ISSUE-0025 — MVP acceptance criteria completion tracker

### Auto-propagate evidence to linked threads

When adding commit evidence to an issue linked to an RFC, automatically propagate the evidence to
the linked RFC.

- RFC-0001 — Auto-propagate commit evidence to linked threads

### Changelog / release report

Summarize closed/accepted threads since a date or tag for release notes.

- RFC-0002 — Changelog / release report command

## Future considerations

The following items are not yet tracked as issues. They represent directions for exploration after
the current open issues are resolved.

### Enhanced TUI editing

- Inline state transitions with guard feedback in the TUI.
- Thread body editing directly in the TUI.
- Richer multiline editor with syntax highlighting.

### Richer search

- Faceted search (filter by kind, status, actor, date range).
- Full-text search with ranking.
- Embedding-based semantic search (long-term).

### Multi-repo and remote workflows

- Push/fetch of forum refs between clones.
- Cross-repository thread references.
- Conflict resolution UX for divergent forum histories after fetch.

### Cryptographic signing

- Extend the approval mechanism with GPG or SSH signatures.
- Verify approval signatures in `verify` and `policy check`.

### Notification and subscription

- Watch specific threads or node types.
- Hook-based notifications (post-event shell hooks).

### Advanced policy

- Quorum-based approvals (e.g., 2 of 3 maintainers).
- Time-based guards (e.g., minimum review period).
- Escalation rules for unresolved objections.

### Metrics and reporting

- Thread velocity and cycle time.
- Objection resolution rate.
- Per-actor contribution summaries.
