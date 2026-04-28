# ADR-003: Workflow Handle Scheme

## Context

git-forum 2.0 introduces `workflow` as the primary user-facing unit of work (spec §2.1). Workflows
group related threads under a name humans can remember, point at, and reference in conversation.
Threads themselves keep opaque content-addressed IDs (`t-XXXXXXXX`) — they are receipts, not
handles.

This shifts the "ID readability" problem from threads (where it failed in 1.x — `RFC-6m4kap23`
is unmemorable) to workflows. Workflows need handles that are:

- **Memorable** — pronounceable, ideally derived from the title.
- **Local-first** — allocatable on any clone without coordination.
- **Conflict-tolerant** — collisions across clones must be resolvable without data loss or
  manual intervention.
- **Stable enough** — links and references should not break gratuitously, but renames must be
  possible.

git-forum's design constraints (no central server, push/fetch over Git refs) preclude any scheme
that requires a coordinator at allocation time.

## Decision

Adopt a **two-layer identity** for workflows, mirroring Git's "SHA + ref" model:

- **Internal opaque ID**: `wf-XXXXXXXX` (8 base36 chars), generated from
  `sha256(actor || timestamp || title || nonce)` — same algorithm as thread IDs (RFC-0030).
  Stored at `refs/forum/workflows/<WORKFLOW_ID>`. Never collides across clones.
- **User-facing handle**: `wf-<slug>` where `<slug>` is derived from the title (`[a-z0-9-]+`,
  3–48 chars). On collision within a clone, append a deterministic petname suffix
  (e.g., `wf-payment-rewrite-quick-fox`) computed from `sha256(workflow_id)`. Stored at
  `refs/forum/aliases/<HANDLE>` as a symref or note pointing to the workflow ID.

Handles are **mutable**. Renames preserve all old handles as permanent aliases via
`workflow_alias` events. Cross-clone handle conflicts (two clones independently claiming the
same handle) are resolved at push time by automatic petname rename of the loser's handle (spec
§8.2.1).

Within a known workflow, child threads may be referenced by short index (`wf-foo#3`). This is a
**session-local convenience**, not a stable identifier (see spec §8.3).

## Consequences

- Handles are pronounceable and grep-friendly without losing the conflict-free property of
  content-addressed IDs.
- The handle alias ref tree becomes the lookup index — handle resolution is one ref-read.
- Workflow rename is cheap (alias write) and never breaks references.
- Push-time handle conflicts auto-recover, but the loser's handle changes silently from their
  perspective until the next CLI invocation tells them. Surprise mitigated by clear notification.
- The petname dictionary (~2,048 adjectives × ~2,048 nouns) is a build artifact that must ship
  with the binary.
- `#N` short references in commit messages or evidence refs are a footgun — the spec forbids
  them and tooling warns when they appear in scanned contexts.
- A user pre-setting `--handle wf-pay` and hitting cross-clone collision still gets a petname
  appended (`wf-pay-quick-fox`); explicit user intent does not bypass conflict resolution.

## Alternatives

### Sequential numeric handles (`wf-1`, `wf-2`, ...)

Pros: short, readable, conventional (matches Linear/Jira).

Cons: requires a central allocator, or post-merge renumbering. Either violates local-first or
breaks references on merge. Same flaw as 1.x sequential thread IDs.

### Pure opaque handles (`wf-x9k2m4p7` — no slug)

Pros: no collision logic needed, deterministic.

Cons: defeats the entire purpose of handles. Same readability problem as 1.x opaque thread IDs.

### Pure petname (`wf-quick-fox`, no slug derivation)

Pros: always pronounceable, always unique with high probability.

Cons: no semantic anchor — `wf-quick-fox` doesn't tell you what it's about. Title becomes the
only mnemonic, and titles are not addressable directly.

### User-mandatory handle on creation

Pros: user always controls naming.

Cons: friction in agent / scripted contexts. Most workflow names are obvious from title; forcing
the user to type them is busywork. Petname auto-append still needed for collision recovery
either way.

### Single-layer ID (handle is the ID, no opaque internal ID)

Pros: one ref tree instead of two.

Cons: rename becomes destructive (changes the canonical ID). Cross-clone conflict resolution
becomes destructive (changing the ID after collision invalidates references). The two-layer
model isolates the volatile name from the stable identity.

## Exit criteria

- Spec §6.1 defines handle generation, petname collision resolution, and rename semantics.
- Spec §8.2.1 defines cross-clone handle conflict resolution.
- `refs/forum/workflows/` and `refs/forum/aliases/` ref trees implemented.
- `git forum workflow rename` records `workflow_alias` events; old handles continue to resolve.
- Petname dictionary bundled with the binary.
- Test: two simulated clones independently allocating the same handle converge to two distinct
  workflows accessible by their (post-rename) handles.
