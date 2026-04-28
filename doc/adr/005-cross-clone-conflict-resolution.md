# ADR-005: Cross-Clone Conflict Resolution

## Context

git-forum 2.0 introduces several entities that can be written independently on multiple clones
and only collide at push or fetch time:

- **Topic handles** (`!<slug>`) — user-derived names that two clones may both claim before
  either has pushed.
- **Topic attach / detach events** — recorded on the thread ref. Two clones may attach the same
  thread to different topics simultaneously.
- **Topic alias renames** — two clones may rename the same topic to different new handles.
- **Tag mutations** (`facet_set` events on threads) — two clones may add or remove tags
  concurrently.
- **Topic archive vs. attach** — one clone archives a topic while another attaches a thread to
  it before seeing the archive.

Within-clone safety is already provided by Git's atomic ref CAS, inherited from 1.x. Cross-clone
divergence is a new surface that 2.0 must specify.

The design tension:

- **Determinism** is mandatory — given the same set of events, every clone must compute the
  same effective state.
- **Agent-friendliness** matters — automated workflows must not be blocked by routine
  reconciliation. Human intervention should be required only when genuinely necessary.
- **Handle stability** is the entire point of having a human-facing topic handle. A handle that
  a user has written into external notes, RFC bodies, or commit messages must not silently
  start meaning a different topic.
- **History preservation** — the event log is the audit trail; resolution must not discard
  intent.

These four objectives sometimes conflict (e.g., agent-friendliness suggests auto-rename, handle
stability forbids it). The resolution rules in spec §8.2 / §8.4 are the negotiated outcome.

## Decision

### 1. Wall-clock LWW with deterministic tiebreaks for non-handle events

Topic attach / detach (§8.2.2) and tag mutation (§8.2.4) conflicts are resolved by Last-Writer-
Wins ordered by `(event.timestamp, actor_id, event_oid)`. The tiebreaker chain is fully
deterministic and clone-independent. All competing events are preserved on the event chain;
only the *effective* state is selected by LWW.

Wall-clock dependency is acknowledged in spec §8.2 and tracked as forward-compatible migration
to Hybrid Logical Clocks (F-W5) if dogfood evidence shows clock skew producing user-surprising
outcomes.

### 2. Explicit error for cross-clone handle conflicts (no auto-rename)

When two clones independently claim the same topic handle and both attempt to push, the alias-
ref CAS fails on the second pusher. The losing client surfaces `HandleConflictOnPush` as an
**error** (non-zero exit) and requires the user to issue an explicit `topic rename` before
re-pushing. There is no silent auto-rename across clones.

The same rule applies to handle alias divergence (rename ⊕ create, §8.2.3): the second pusher
fails with `HandleConflictOnPush` and must rename.

### 3. Within-clone handle collisions get automatic petname recovery

When a topic handle is taken locally at allocation time, the system appends a deterministic
petname suffix (`!payment-rewrite-quick-fox`) and notifies the user. This is symmetric in
mechanism but asymmetric in semantics: within-clone, no other actor's claim is being silently
overridden, so automatic recovery is safe.

### 4. Atomic push for ref groups that span workflow + alias

Topic create and topic rename touch two refs (the workflow event chain and the alias ref) that
must succeed or fail together. Clients use Git's `push --atomic` option (or refuse the operation
if their transport cannot guarantee atomicity). Spec §8.4.1 defines the atomic groups
explicitly. This prevents the failure mode where a workflow exists on the remote without a
visible handle, or where an alias points to a workflow that does not yet exist.

### 5. Display-only short index `/N`

Topic-scoped short references (`!foo/3`) are derived from the local view of attach events and
may differ across clones until sync. They are accepted only at read-only or
canonical-resolution-then-discard CLI positions, and rejected with `ShortIndexInPersistedRef`
in any persisted context (commit messages, evidence refs, link targets, attach arguments). The
rejection message includes the canonical thread ID.

### 6. Archived-topic attach: within-clone block, cross-clone tolerate

Within a clone, attaching a thread to an archived topic is rejected with
`AttachToArchivedTopic` unless `--force` is passed (so newly captured work cannot disappear
into a hidden context). Cross-clone, both events succeed at the ref layer; the resulting
inconsistency is reported by `doctor`. The asymmetry reflects the cost gradient: a within-clone
gate is cheap and prevents the most common UX failure; cross-clone arbitration is unavoidable
without breaking the local-first protocol.

## Consequences

- Cross-clone divergence has a fully specified resolution path for every scenario in §8.2;
  `doctor` is the single surface for reporting unresolved or surprising outcomes.
- Handle stability is preserved: a handle written in external prose continues to mean the same
  topic forever, because no silent reassignment occurs.
- CI ergonomics take a small hit: `HandleConflictOnPush` produces a non-zero exit that
  pipelines must handle by triggering a human-mediated rename. Topic creation is rare enough
  that this cost is acceptable.
- Atomic push is required for two operations. Some Git transports (older protocols, certain
  proxies) do not support `--atomic`. Clients on those transports cannot create or rename
  topics; this is documented as a transport-level constraint rather than a workaround being
  built into the spec.
- LWW for tags can produce theoretical "tag flicker" across clones with skewed clocks. The
  flicker is bounded — the next explicit `tag add`/`tag rm` always wins — and tags are
  advisory rather than gating, so the practical impact is low. F-W6 covers a future move to
  CRDT-based tag merging if needed.
- The asymmetry between within-clone (auto petname, attach-to-archived rejection) and cross-
  clone (explicit user action) is documented. Reviewers may find it surprising at first; the
  rationale is that within-clone resolution sees full local state and can reason about it,
  while cross-clone resolution must work without coordination and must not silently override
  remote intent.

## Alternatives

### LWW for handle conflict (silent auto-rename) — rejected

Symmetric with within-clone resolution, but undermines handle stability. A handle a user has
written into external prose can silently come to mean a different topic on the remote. The
agent-friendliness argument (no blocking error) does not outweigh the lost guarantee. Rejected
during review.

### First-Writer-Wins (FWW) for attach conflict

Initial intent preserved by attach timestamp; later attaches require explicit detach first.

Pros: matches a "stake your claim" mental model.

Cons: cannot fix attach mistakes by re-attaching elsewhere later — a user who attaches to the
wrong topic must remember the order of operations to undo it. LWW (with reversibility via
re-attach) is more forgiving and matches the "most recent intent" model that users naturally
expect.

### Explicit conflict marker for attach (require manual resolution)

Treat divergent attach as a true conflict; mark the thread "in conflict"; refuse subsequent
operations until the user acknowledges.

Pros: zero auto-resolution surprise; user always sees and decides.

Cons: blocks agent workflows, which is contrary to the agent-friendliness goal. Friction is
high enough that for routine cases (which dominate) it would be a net loss.

### CRDT (observed-remove set) for tags

Eliminates wall-clock dependency entirely; tag merge is provably correct without timestamp
ordering.

Pros: most robust against clock skew; no theoretical flicker.

Cons: implementation complexity; event payload format may need extension to carry causal
metadata. Deferred to F-W6 with a forward-compat contract that allows migration without
breaking 2.0 clients.

### HLC (Hybrid Logical Clocks) for all events

Replaces wall-clock with HLC across the board, removing clock dependency for attach/tag/state
ordering.

Pros: eliminates clock skew as a class of issues.

Cons: implementation cost; serialization format extension; tooling that doesn't compute HLC
must fall back gracefully. Deferred to F-W5.

### Eventually-consistent reconciliation by `doctor` (no atomic push)

Allow workflow ref and alias ref to be pushed independently; have `doctor` detect and repair
inconsistencies (orphan workflow without handle, dangling alias).

Pros: tolerates non-atomic-push transports.

Cons: there is a window where other clients observe the inconsistent state and may make
decisions on it (e.g., a client that fetches between the two pushes sees a workflow without a
handle and may treat it as "deleted"). Atomic push closes the window structurally; `doctor`
becomes a sanity-check, not a correctness component.

## Exit criteria

- Spec §8.1, §8.2, §8.3, §8.4 specify all six rules above with worked scenarios.
- Spec §13 enumerates `HandleConflictOnPush`, `AttachConflictResolved`, `AttachToArchivedTopic`,
  `ShortIndexInPersistedRef`, `AmbiguousReferenceWithoutMarker` with severity and trigger.
- Spec §14 testing strategy covers each cross-clone scenario with simulated multi-clone tests.
- F-W5 (HLC) and F-W6 (CRDT tags) are recorded in Appendix A.3 with forward-compat contracts.
- `git forum doctor` reports unresolved cross-clone conflicts and recommends remediation.
- Atomic push is implemented for `topic_create` and `topic_rename` ref groups; transports that
  cannot guarantee atomicity refuse the operation rather than proceed.
