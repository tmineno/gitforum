# ADR-009: Stale-Reference Cleanup Is a Separate Operation From Content Purge

## Context

Phase 1 (commit `6ac6bf4`) split event replay into a lenient default and a
strict mode (`thread::replay_strict` / `git forum doctor --strict`). Running
strict mode against this repo's own forum data immediately surfaced 164
`StrictReplayIssue::UnknownTargetNode` findings across 43 threads:
`edit` / `retract` / `resolve` / `reopen` / `retype` / `revise-body` events
whose `target_node_id` (or `incorporated_node_ids` for `revise-body`)
referenced a node SHA that was no longer present in the thread's chain.
Every distinct orphan target SHA (156 of them) was confirmed gone from the
local Git object store via `git cat-file -e`.

Investigation traced the orphans to two upstream causes:

1. **Manual chain rewriting outside `purge`.** The summary node on thread
   `1ooguji1` documents the user's intent: "The earlier action nodes
   (`690cecad / e841a7ff / ...`) have been resolved with their content
   carried forward to the corresponding task." Action events were dropped
   from the chain by direct Git rewrite; the `resolve` events that pointed
   at them stayed.
2. **`purge_event`'s content-only redaction.** When `purge_event` censors a
   commit's body/title to `[purged]`, the commit's SHA changes (different
   tree → different commit). Any downstream event whose `target_node_id`
   field equals the **old** SHA is no longer adjusted; after `git gc` prunes
   the unreferenced old object, the reference becomes stale. `purge_event`
   does not scan descendants for references to the SHAs it rewrites, by
   design — it is scoped to censoring content, not reshaping the reference
   graph.

Both paths produce the same artefact: events that lenient replay silently
no-ops and strict replay flags. The Phase 1.5 cleanup (commit `47ad593`,
the `git forum prune-stale-events` subcommand) drops 120 such events from
43 threads on this repo and brings strict replay to a clean state.

## Decision

Treat **content redaction** and **reference-graph cleanup** as two distinct
operations with different blast-radius semantics, surfaced by two different
subcommands:

- `git forum purge --event <sha>` — censors the event's body/title to
  `[purged]`. Rewrites the chain because the commit tree changes, but does
  not scan or rewrite descendant references. Audit-trail-preserving by
  design.
- `git forum prune-stale-events` — drops events whose `target_node_id` /
  `incorporated_node_ids` no longer resolve. Rewrites only the suffix from
  the first dropped commit forward; create event SHAs are preserved.
  Default dry-run; `--apply` to execute.

The recommended workflow when redacting an event that other events
reference is `purge --event <sha>` then `prune-stale-events` then
`reindex`. This composition is explicit rather than embedded as a
side-effect of `purge`.

## Consequences

- `purge_event` keeps its narrow scope: redact content, leave structure.
  This makes its behaviour predictable for the original use case
  (accidental secret/PII commit) and avoids cascading rewrites the
  operator did not request.
- Strict mode (`doctor --strict`) is the canonical detector for stale
  references whatever their origin (manual rewrites, past purges, future
  write-side bugs). It is not coupled to the purge code path.
- Operators must remember the two-step composition. The CLI hint emitted
  by `purge --event` in a follow-up change should mention
  `prune-stale-events` to reduce that burden; out of scope for this ADR.
- The 156 originally-purged commits this repo recovered from are gone for
  good. The information they once carried lives only in summary nodes the
  operator wrote at migration time. That is the explicit cost of the
  Phase 1.5 cleanup decision (Option A in the review thread).
- Any future `purge`-class operation that wants atomicity (redact +
  reference-cascade in one step) MUST be a new subcommand or an opt-in
  flag (`purge --cascade`). Silently changing `purge`'s blast radius is
  out of scope.

## Alternatives

- **Make `purge_event` cascade reference updates automatically.** Rejected
  for now. The cascade is heuristic (every `target_node_id` /
  `incorporated_node_ids` field across every descendant event must be
  rewritten if its value matches a `purge`-rewritten SHA), and conflating
  redaction with structural rewrite makes failure modes harder to reason
  about. An opt-in `--cascade` flag is a possible future addition; this
  ADR does not commit to it.
- **Make `purge_event` refuse when descendants reference the target.**
  Rejected — too restrictive for the originating use case (operator just
  wants the secret out of the body). Forces the operator to manually
  drop references first, which is exactly what `prune-stale-events`
  exists to automate.
- **Emit a runtime warning from `purge_event` and stop.** Considered.
  Useful as a UX nudge, but should not gate this ADR. Tracked as
  documentation work.
- **Lenient replay continues to no-op stale references; do not introduce
  strict mode.** Already rejected by Phase 1's design — the silent
  swallowing is exactly the integrity gap that motivated `replay_strict`.

## Exit criteria

- `prune-stale-events` exists as a documented subcommand with a dry-run
  default. (Done in commit `47ad593`.)
- `doctor --strict` reports zero `UnknownTargetNode` findings on this
  repo. (Verified post-`apply`.)
- This ADR documents the design split so a future contributor does not
  silently re-architect `purge_event` into a structural-rewrite tool.
- The recommended `purge → prune-stale-events → reindex` workflow is
  surfaced by `purge`'s CLI help / output. (Out of scope for this ADR;
  tracked as follow-up.)

## References

- Commit `6ac6bf4` — Phase 1: strict replay validator, `doctor --strict`
  opt-in, `prune-orphans`.
- Commit `47ad593` — Phase 1.5: `prune-stale-events` plus the cleanup
  applied to this repo (120 events dropped from 43 threads).
- `src/internal/purge.rs::purge_event` / `rewrite_chain` — current
  content-only redaction implementation.
- `src/internal/prune.rs::scan_stale_events` /
  `apply_stale_event_plans` — the post-rewrite cleanup added in Phase 1.5.
- `src/internal/validate.rs::StrictReplayIssue::UnknownTargetNode` — the
  strict-mode detector.
