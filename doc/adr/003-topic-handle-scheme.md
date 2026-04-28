# ADR-003: Topic Handle Scheme

## Context

git-forum 2.0 introduces `topic` as a **named context** that groups related threads under a
handle humans can remember, point at, and reference in conversation (spec §2.1). Threads
themselves remain the primary unit of work and keep opaque content-addressed IDs (display form
`@XXXXXXXX`) — those are receipts, not handles. Topics are an optional grouping layer; standalone
threads are first-class (spec §2.2.4).

This shifts the "ID readability" problem from threads (where it failed in 1.x — `RFC-6m4kap23`
is unmemorable) to topics. Topics need handles that are:

- **Memorable** — pronounceable, ideally derived from the title.
- **Local-first** — allocatable on any clone without coordination.
- **Conflict-resolvable** — collisions across clones must have a deterministic, data-preserving
  resolution path. Within-clone collisions are recovered automatically via petname suffix.
  Cross-clone handle conflicts surface at push time as an explicit error and require an explicit
  rename — silent auto-rename was rejected because it would break handle stability for the
  losing clone (see "Alternatives" below).
- **Stable enough** — links and references should not break gratuitously, but renames must be
  possible.

git-forum's design constraints (no central server, push/fetch over Git refs) preclude any scheme
that requires a coordinator at allocation time.

## Decision

Adopt a **two-layer identity** for topics, mirroring Git's "SHA + ref" model:

- **Internal opaque ID**: `<topic-id>` — 8 base36 chars, generated from
  `sha256(actor || timestamp || title || nonce)` (same algorithm as thread IDs, RFC-0030).
  Stored at `refs/forum/topics/<topic-id>`. Never collides across clones.
- **User-facing handle**: display form `!<slug>` where `<slug>` is derived from the title
  (`[a-z0-9-]+`, 3–48 chars). On collision within a clone, append a deterministic petname
  suffix (e.g., display `!payment-rewrite-quick-fox`) computed from `sha256(topic_id)`.
  Stored at `refs/forum/aliases/<slug>` (the bare slug, no `!`; the marker is display-only
  per spec §6.0) as a symref or note pointing to the topic ID.

Handles are **mutable**. Renames preserve all old handles as permanent aliases via
`topic_alias` events. Cross-clone handle conflicts (two clones independently claiming the
same handle) surface at push time as an **explicit error** (`HandleConflictOnPush`); the user
must rename their topic before re-pushing. Within-clone collisions, by contrast, are
recovered automatically via a deterministic petname suffix.

Within a known topic, child threads may be referenced by short index (`!foo/3`). This is a
**session-local convenience**, not a stable identifier (see spec §8.3). The `/` separator is
the spec §6.0 short-index marker.

## Consequences

- Handles are pronounceable and grep-friendly without losing the conflict-free property of
  content-addressed IDs.
- The handle alias ref tree becomes the lookup index — handle resolution is one ref-read.
- Topic rename is cheap (alias write) and never breaks references.
- **Handle stability is preserved across clones**: a handle a user has written into external
  notes, RFC bodies, or commit messages keeps meaning the same topic forever, because no
  silent reassignment occurs. Cross-clone push conflicts require explicit user resolution.
- This costs CI ergonomics: `git forum push` (or wrapping push) can fail with
  `HandleConflictOnPush`, requiring a manual `topic rename` step. Topic creation is rare
  enough that this is acceptable; for high-volume thread creation, no analogous conflict exists
  (thread IDs are content-addressed).
- The petname dictionary (~2,048 adjectives × ~2,048 nouns) is a build artifact that must ship
  with the binary, used for within-clone collision recovery only.
- `/N` short references in any persisted context (commit messages parsed by the `commit-msg`
  hook, evidence refs, link targets) are an error in 2.0 (`ShortIndexInPersistedRef`), not a
  warning. They are display-only.

## Alternatives

### Sequential numeric handles (`!1`, `!2`, ...)

Pros: short, readable, conventional (matches Linear/Jira).

Cons: requires a central allocator, or post-merge renumbering. Either violates local-first or
breaks references on merge. Same flaw as 1.x sequential thread IDs.

### Pure opaque handles (`!x9k2m4p7` — no slug)

Pros: no collision logic needed, deterministic.

Cons: defeats the entire purpose of handles. Same readability problem as 1.x opaque thread IDs.

### Pure petname (`!quick-fox`, no slug derivation)

Pros: always pronounceable, always unique with high probability.

Cons: no semantic anchor — `!quick-fox` doesn't tell you what it's about. Title becomes the
only mnemonic, and titles are not addressable directly.

### User-mandatory handle on creation

Pros: user always controls naming.

Cons: friction in agent / scripted contexts. Most topic names are obvious from title; forcing
the user to type them is busywork. Petname auto-append still needed for collision recovery
either way.

### Single-layer ID (handle is the ID, no opaque internal ID)

Pros: one ref tree instead of two.

Cons: rename becomes destructive (changes the canonical ID). Cross-clone conflict resolution
becomes destructive (changing the ID after collision invalidates references). The two-layer
model isolates the volatile name from the stable identity.

### Silent auto-rename on cross-clone handle conflict (rejected after review)

An earlier draft proposed automatically appending a petname suffix when a push failed due to
alias-ref CAS conflict — symmetric with the within-clone resolution. This was rejected because
the loser's handle would change without their explicit consent; any external reference to that
handle (RFC body, commit message, external document) would silently start resolving to a
different topic. The whole point of having a stable human-facing handle is that it can be
written down and pointed at later. Auto-rename undermines that guarantee for whichever clone
loses the push race. Explicit failure with a manual rename step is safer; the friction is
acceptable because topic creation is rare.

## Exit criteria

- Spec §6.1 defines handle generation, petname collision resolution (within-clone), and rename
  semantics.
- Spec §8.2.1 defines cross-clone handle conflict as an explicit error requiring user rename.
- `refs/forum/topics/` and `refs/forum/aliases/` ref trees implemented; create / rename use
  atomic push (spec §8.4.1).
- `git forum topic rename` records `topic_alias` events; old handles continue to resolve.
- Petname dictionary bundled with the binary (within-clone collision recovery only).
- Test: two simulated clones independently allocating the same handle — the second push fails
  with `HandleConflictOnPush`; after explicit `topic rename` on the loser, both topics
  exist with distinct handles.
- Test: `/N` short reference rejected with `ShortIndexInPersistedRef` when used as evidence ref,
  link target, or in `commit-msg` hook input.
