# task `wlqhi8xh`: Stored / Domain Event Two-Layer Split

## Context

The current `Event` struct (`src/internal/event.rs`) is a single bag
holding ~17 `Option<...>` fields plus the common metadata five
fields share. A serialized event populates roughly 20–30% of those
fields; the rest serialize as absent (`#[serde(skip_serializing_if = "Option::is_none")]`).

This shape was chosen deliberately. The struct's docstring records
the trade-off:

> `Default` is implemented so test sites and helpers can construct
> events with `Event { thread_id: "...".into(), event_type: ..., ..Default::default() }`
> rather than enumerating every field. This keeps adding new optional
> fields from cascading edits across the codebase.

The shield works for **storage** and **construction**. It fails for
**domain logic**: every site that consumes an `Event` (replay, render,
diff, doctor, migrate, brief) has to defensively unwrap whichever
fields the event_type implies, and the compiler does not catch a
missed combination. `apply_event` (`src/internal/thread.rs`) is the
canonical example — every `EventType` arm starts with a
`match (event.field_a, &event.field_b) { (Some, Some) => ..., (None, _)
=> issue, ... }` to recover what should already be the variant's
guaranteed payload.

The parent v2.x design RFC (thread `915yuegd`) carved out thread `wlqhi8xh`
to split the bag in two:

- **`StoredEvent`** — the existing serde-friendly bag, kept as-is for
  on-disk shape and forward-compatible reads.
- **`DomainEvent`** — a typed sum type projected from `StoredEvent`,
  consumed by replay / domain logic. Each variant carries only the
  fields its `EventType` actually uses.

Before projection code can be written, this ADR must settle one
SPEC-level question (called out in the ticket pre-conditions):

> **How should the projection layer treat a `StoredEvent` whose
> `event_type` value is not recognised?**

Three options were enumerated in the ticket:

- **(a) Graceful** — `DomainEvent::Unknown(StoredEvent)` carrying the
  raw bag; replay treats as no-op + strict issue. Forward-compat
  preserved.
- **(b) Loud reject** — `ProjectionError::UnknownEventType` aborts
  replay. No forward-compat tolerance.
- **(c) Forced migration** — `git forum migrate` rewrites or drops
  unknown variants pre-replay; replay never sees them.

## Decision

**Adopt option (a) Graceful.** A `DomainEvent::Unknown` variant
exists in the typed enum and carries a reference to (or owned copy
of) the raw `StoredEvent`. Replay treats `DomainEvent::Unknown` as a
no-op and emits a `StrictReplayIssue::UnknownEventType` so doctor /
strict surfaces still flag the situation.

This decision matches three already-established properties of the
codebase:

1. **Storage is forward-compatible by construction.** SPEC-2.0 §10
   already commits to lossless 1.x → 2.0 migration and keeps
   `legacy_subtype` for archival reference. Refusing to load events
   that used to round-trip cleanly would regress that contract.
2. **Replay is graceful by default.** `replay_with_issues_inner`
   silently no-ops on conditions that strict-mode flags as issues —
   missing required fields, invalid state names, second
   `facet_set` lifecycle. An unknown `event_type` is the natural
   extension of that pattern.
3. **Doctor / `--strict` is the surfacing channel for graceful
   degradations.** Adding `UnknownEventType` to `StrictReplayIssue`
   gives operators visibility without blocking everyday read paths.

### Implementation phasing

The ticket allows the rename and projection-introduction to land in
separate PRs. This ADR documents the chosen direction; the actual
forward-compat hook (a serde fallback variant on `EventType`) is
broken out as follows:

- **task `wlqhi8xh` EventMeta/DomainEvent slice** — introduce `EventMeta` + `DomainEvent`,
  `Event::project()`, and convert `apply_event` to consume
  `&DomainEvent`. Reserve `DomainEvent::Unknown { meta, raw: Box<Event> }`
  in the enum but leave it unreachable today: every current
  `EventType` variant has an explicit `project` arm. This task slice is
  purely a refactor — no on-disk shape change.
- **task `wlqhi8xh` unknown-event fallback slice** — add `EventType::Other(String)` with
  `#[serde(other)]` so a future writer's unknown event_type
  deserialises into the fallback variant; wire `project` so
  `EventType::Other` produces `DomainEvent::Unknown`. This task slice
  changes deserialiser behaviour and rolls through the ~80 existing
  `match event.event_type` sites; it is tractable but not worth
  bundling into the projection-introduction PR.
- **task `wlqhi8xh` StoredEvent rename slice** — rename `Event` → `StoredEvent`. Pure
  identifier shuffle; deferred per the ticket's exception clause to
  keep task `wlqhi8xh` EventMeta/DomainEvent slice reviewable.

The decision recorded here applies once task `wlqhi8xh` unknown-event fallback slice lands. Until then,
the practical behaviour for a corrupt or future-shape event is the
existing one: serde rejects deserialisation up-front. That is
strictly tighter than option (a) and does not violate it.

## Consequences

- `StrictReplayIssue` gains an `UnknownEventType { event_id, raw_type:
  String }` variant. `doctor --strict` learns to surface it; doctor's
  exit code policy is unchanged (strict issues are advisories, not
  failures, unless the operator opted into strict).
- `apply_event` gains an explicit `DomainEvent::Unknown => no-op`
  arm, preserving the property that no replay path silently drops a
  stored event.
- A future binary that introduces a new `EventType` variant gains
  forward compat *for older readers* the day task `wlqhi8xh` unknown-event fallback slice lands. Until
  then, older readers fail to deserialise — the same as today.
- The "cascading edit" shield documented on the existing `Event`
  struct moves up a layer: adding an optional storage field still
  costs zero edits across the codebase (task `wlqhi8xh` EventMeta/DomainEvent slice preserves
  `Event::Default`), but adding a new **domain-meaningful** event
  variant now costs four edits — `EventType`, `DomainEvent`, the
  `project` arm, and the `apply_event` arm — and the compiler
  enforces all four. This is the intended trade.

## Alternatives

### (b) Loud reject — `ProjectionError::UnknownEventType` aborts replay

Pros: simpler implementation; no `Unknown` variant to plumb through.

Cons: regresses the existing graceful-reads property. Any operator
running an older binary against newer data would see hard failures
instead of advisory issues. The 1.x → 2.0 migration story is built
around graceful reads (see SPEC-2.0 §10.1); reversing that for
hypothetical 3.0 would be inconsistent.

### (c) Forced migration before replay

Pros: replay logic stays minimal — every event reaching apply_event
is well-formed.

Cons: requires every read path (including `git forum log`,
`git forum show`, doctor, the TUI) to gate on a migration step. The
operational surface is much larger than option (a); the migration
tool grows a "rewrite unknown variants" branch for which there is
no current use case.

### Defer the unknown-variant question until task `wlqhi8xh` unknown-event fallback slice

Pros: smaller blast radius for the projection-introduction PR.

Cons: the ticket pre-condition requires the decision *before* the
projection code is written, because the `DomainEvent` enum's shape
depends on it (does `DomainEvent::Unknown` exist? does it own the
`StoredEvent` or borrow it? does `project` return `Result` or
`(DomainEvent, Vec<Issue>)`?). Delaying the decision means
re-shaping the enum mid-implementation. The phasing above lets the
decision land here while the deserialiser change stays out of
scope.

## Exit criteria

- task `wlqhi8xh` EventMeta/DomainEvent slice landed: `DomainEvent` enum exists with an `Unknown`
  variant, `Event::project()` returns either a known variant or a
  projection error tied to a specific missing/invalid field, and
  `apply_event(state, &DomainEvent, issues)` is the replay
  signature.
- task `wlqhi8xh` unknown-event fallback slice landed: `EventType::Other(String)` is serde-fallback;
  `project` maps it to `DomainEvent::Unknown`; `apply_event` no-ops
  on `Unknown` and emits `StrictReplayIssue::UnknownEventType`.
- This ADR is referenced from the SPEC-2.0 §10 forward-compat
  section once task `wlqhi8xh` unknown-event fallback slice ships.
