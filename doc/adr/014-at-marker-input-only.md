# ADR-014: `@<id>` is accepted on input and never printed

Status: Accepted
Date: 2026-09-23
Thread: `cxpv15ny`

## Context

SPEC-3.0 §6 as first written said two things: human-facing output shows
bare thread IDs, and CLI input MUST reject `@<id>`. Neither held on
3.2.0:

| Surface | Behavior | Where |
|---|---|---|
| CLI input | `@<id>` accepted; `@` stripped | `thread::resolve_from_list` (still cited SPEC-2.0 §6.1) |
| TUI display | IDs rendered as `@<id>` in the list, detail header, link popups | `id::display_thread_id`, 7 call sites |
| TUI yank | clipboard gets `@<id>` for threads and nodes | `tui/input.rs`, 3 sites |
| `supersede` default comment | `Superseded by @<new>` | `commands/supersede.rs` |
| `show` node-redirect hint | `it lives in thread @<parent>` | `commands/show.rs` |
| MANUAL / README examples | `@<id>` on 70 / 11 lines | docs |

Rejecting `@` on input, as §6 required, would break the path the TUI
itself creates (yank an ID, paste it into a command) along with every
script and agent habit built on the MANUAL's examples. ADR-013 had
already allowed `@` on a commit's `Refs:` trailer for the same reason.

## Decision

1. CLI input accepts a thread ID with one leading `@` and treats it as
   the bare ID. SPEC-3.0 §6 is amended to require this. No code change:
   the resolver already strips it.
2. Output never adds `@`. `display_thread_id` returns the bare ID, so
   the TUI list, header and popups show `fg61bcmp`. Yanked text,
   the `supersede` comment and the `show` hint are bare too.
3. The MANUAL and README examples use bare IDs. The MANUAL keeps one
   sentence saying that `@<id>` is accepted.

`@` never appears in a thread ID (§6 forbids it in the ref path
component), so stripping one leading `@` cannot turn one valid ID into
another.

## Consequences

- Scripts and agents that pass `@<id>` keep working.
- The TUI looks different: the ID column loses its `@`. Text yanked
  from the TUI now pastes cleanly into places that do not strip `@`,
  such as `git log --grep` or another tool's search box.
- Thread bodies written earlier with `Superseded by @<id>` stay as they
  are; only new comments change.
- The publish lint still recognizes `@<id>` in bodies (SPEC-3.0 §5.5),
  because existing bodies contain it.

## Alternatives

- **Reject `@` on input, as §6 said.** Rejected: it breaks the TUI's own
  yank-then-paste path and existing scripts, for no gain in safety.
- **Make `@<id>` the official display form.** Rejected: it reverses the
  3.0 decision that output is bare, and every new surface (JSON,
  trailers, ref names) would need a rule for when the `@` appears.

## Exit criteria

- A test pins that `show @<id>` resolves (`tests/cli_at_marker_test.rs`).
- Tests pin bare output for the TUI list row, `display_thread_id` (which
  the yank text goes through), the `supersede` comment, and the `show`
  node-redirect hint.
- `grep -nE '@[0-9a-z]{8}|@<' doc/MANUAL.md README.md` shows only lines
  that say `@` is accepted on input or list it as a lint target.
