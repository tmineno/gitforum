# ADR-001: Git OID Canonical IDs

## Context

`git-forum` stores thread history as append-only Git commits under `refs/forum/threads/<THREAD_ID>`.

The previous MVP direction allowed internal UUID / ULID-style IDs for events and nodes. That creates two problems:

1. the canonical identifier is not the Git object that actually carries the authoritative history
2. users still need to handle long opaque IDs, but those IDs do not buy Git-native auditability

For `git-forum`, the important identity boundary is already the Git object graph.

## Decision

- Thread display IDs remain human-readable sequences such as `RFC-0001` and `ISSUE-0001`.
- The canonical ID of an event is the Git commit OID of the commit that stores that event.
- The canonical ID of a node is the Git commit OID of the `say` event commit that introduced that node.
- `edit`, `retract`, `resolve`, and `reopen` events reference nodes by that canonical node OID.
- CLI input may use either the full canonical OID or a unique prefix.
- Prefix resolution rules:
  - exact match wins first
  - if there is no exact match, at least 8 hex characters are required
  - `node show` resolves against all nodes in the repository
  - thread-scoped node commands resolve only inside the specified thread
  - ambiguous prefixes fail with candidate full IDs
- The serialized `event.json` payload does not need to duplicate its own canonical event OID. Readers derive it from the enclosing commit.

## Consequences

- Event and node identity becomes Git-native and auditable.
- IDs are globally unique under the repository's object format without introducing a second identity scheme.
- Short-ID UX still depends on prefix resolution rather than intrinsically short canonical IDs.
- New events can coexist with older repos that serialized explicit `event_id` fields, as long as readers treat commit OID as authoritative.
- The implementation must derive node IDs for `say` events from commit OIDs rather than from a generated payload field.

## Alternatives

### Proper ULID / UUIDv7

Pros:

- simple to implement
- well-known format
- prefix resolution works well

Cons:

- not Git-native
- duplicates identity outside the Git object graph

### Thread-local sequential node IDs

Pros:

- best CLI ergonomics
- highly readable

Cons:

- weak under branch divergence and merge
- requires aliasing or renumbering logic

### Hybrid canonical opaque ID plus display alias

Pros:

- good balance of auditability and UX

Cons:

- more moving parts for MVP

## Exit criteria

- Spec defines event and node canonical IDs in terms of Git commit OIDs.
- CLI resolution rules are documented.
- Implementation returns and accepts canonical node OIDs.
- Replay and render paths derive canonical IDs from Git commits, not from generated opaque payload IDs.
