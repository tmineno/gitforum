# git-forum Core Value

Last updated: 2026-04-30
Status: **Authoritative**. Upstream of `SPEC.md`, `SPEC-2.0.md`, and all ADRs.
Conflict resolution: when a spec or implementation decision conflicts with this
document, the spec/implementation must change — not this document.

---

## Statement

> **git-forum is a tool for humans and AI coding agents to deliberate, agree,
> and implement as peers, inside the same Git repository as the code they
> are changing.**
>
> **Problem it solves.** Design intent and implementation code drift apart when
> they live in separate tools, separate ID spaces, or separate comment streams.
> Over time, the answer to "why is this code written this way?" is lost.
>
> **How it solves it.** Discussion data is stored as Git refs. The discussion
> UI (CLI, TUI) is consolidated in `git-forum`. **Data transport is delegated
> to standard `git push` / `git fetch`** — git-forum does not introduce its
> own distribution protocol. This, together with the constraint that humans
> and AI use the same CLI surface, are the two structural commitments that
> support the core.
>
> **Agents are participants — not coordination targets, not automation
> drivers.**

---

## Scope: what the tool may do

Inside the non-goal boundary above, the tool implements two categories
of behavior. The boundary between them is load-bearing — confusing the
two is how feature creep starts.

### Guards (single-thread, blocking)

Rules that *block* an operation. To stay inside the core value, a
guard MUST evaluate by reading only the events of the thread being
modified. A guard whose evaluator needs to read another thread
crosses into cross-thread workflow enforcement (non-goal §1) and is
therefore not a guard — it is either an advisory (below) or
out-of-scope.

Examples:

- "Cannot transition to `done` while open objections exist on this
  thread." (`RFC-0018` operation checks.)
- "Cannot add an `evidence` event when the thread is in a terminal
  state."
- "Required body sections must be present at thread creation."

### Advisories (display / observation, never blocking)

Surfaces that *inform* without gating any operation. An advisory MAY
read across threads — answering questions like "what is the state of
the parent RFC?" or "which children of this thread are still open?"
— but it never blocks an operation. The user (or agent) can always
proceed; the advisory just makes the relevant cross-thread context
visible.

Examples:

- `git forum show RFC-X` lists threads that link to RFC-X with
  `--rel implements` and their current state.
- `git forum verify TASK-Y` reports "linked RFC-X is not yet
  `done`" *without* preventing TASK-Y's transition.
- Post-action stderr hints suggesting the next plausible command.
- `doctor`'s "untriaged" count.

This is how the tool surfaces cross-thread *information* without
taking on cross-thread *enforcement*. If a future feature looks like
it should block based on another thread's state, the answer is
"reframe it as an advisory, or it's out-of-scope" — not "loosen the
guard definition".

### Connection to code (always in scope)

- `branch bind`: a thread can name a Git branch.
- `commit-msg` hook: validates that referenced thread IDs exist.
- `evidence add`: pointers from threads to commits, files, hunks,
  tests.

These keep discussion adjacent to the code it concerns. They do not
introduce cross-thread enforcement.

---

## What we explicitly do not do

These are non-goals at the **core value** level — not just at a release
level. RFCs and features that fall into these categories are rejected by
this document, regardless of how cleanly they are designed.

1. **Cross-thread workflow enforcement.** No thread-to-thread state
   coupling, no automatic transitions triggered by other threads, no
   policy predicates that dispatch on the state of another thread.
   *(rejects RFC-0027, RFC-ij6g130o, the spec-driven-workflow direction
   in RFC-0022)*

2. **Agent dispatch / coordination.** No leases, no claims, no
   assignment scheduling, no automatic agent invocation from forum
   events.
   *(rejects RFC-rwi8spmf, the dispatch portion of RFC-6m4kap23)*

3. **Reinventing distribution.** No `git forum push` / `git forum
   fetch`, no cross-clone conflict-resolution protocol, no
   git-forum-specific merge logic above what plain Git provides on
   refs. If standard Git push/fetch on `refs/forum/*` is not enough,
   the answer is "fix the data layout," not "build a protocol."
   *(rejects ADR-005's 5-scenario protocol; cancels SPEC-2.0 Phase 4)*

4. **Project management / dashboards.** Not a replacement for external
   PM tools. No Gantt, no velocity, no burndown, no SLA tracking, no
   resource allocation.
   *(rejects RFC-0022's dashboard direction)*

5. **Other surfaces and modes.** No real-time collaborative editing.
   No proprietary Web UI as a primary interface. No AI-only command
   set parallel to the human one.
   *(rejects RFC-0019)*

---

## Empirical basis

Survey conducted 2026-04-30 against the v2.0.2 event-chain dataset.
Subsequent surveys SHOULD use the migration archive at
`legacy/events.ndjson` (SPEC-3.0 §8.2) so empirical comparisons remain
reproducible across the 3.0 collapse.

This document is grounded in observed usage of the tool inside its own
repository:

- **Thread distribution (258 threads):** `issue` 197, `rfc` 43, `task`
  16, `dec` 0. The `dec` kind has zero uses; the `task`/`issue`
  boundary is inconsistent. `rfc` is the only kind whose identity
  comes from its protocol shape (proposal → review → accept), not
  from its label.
- **Authorship of accepted RFCs:** Among the design RFCs that shaped
  the product (≈14 sampled), authorship splits roughly evenly between
  `human/*` and `ai/*` actors. The "human-agent parity" claim in the
  README is not aspirational — it describes how this repo already
  operates.
- **Workflow-orchestration attempts:** Six separate RFCs proposed
  some form of cross-thread workflow enforcement, automatic agent
  coordination, or executable workflow policy
  (RFC-0021 deprecated, RFC-0027 withdrawn, RFC-34fbx905 withdrawn,
  RFC-0022 draft 1+ month, RFC-rwi8spmf draft, RFC-ij6g130o draft
  2026-04-30). None reached `accepted`. The recurrence of this
  direction is itself the strongest signal that it must be *named
  and rejected* at the core-value layer, not re-evaluated each
  time it resurfaces.

---

## Litmus test

The five non-goals above are the active gate. The table below shows
worked examples of how the guard/advisory split and the non-goals
apply in practice. It is illustrative, not exhaustive — full
per-feature verdicts live in the current spec (SPEC-3.0 and superseded
ADRs).

| Direction | Verdict | Why |
|---|---|---|
| Single-thread state guards (operation checks) | **Keep** (guard) | Reads only the thread being modified. |
| Display linked-thread state in `show` / `verify` (no blocking) | **Keep** (advisory) | Cross-thread *information* without cross-thread *enforcement*. |
| Cross-thread workflow policy | **Reject** | Cross-thread orchestration is non-goal #1. |
| Lease / claim / agent dispatch | **Reject** | Non-goal #2. Agents are peers, not coordination targets. |
| Dashboard / scope tracking as core feature | **Reject** | Non-goal #4. *(A read-only `brief` command is acceptable iff it derives strictly from current thread state and adds no enforcement.)* |
| AI-specific commands (`git forum fix`, agent-only flags) | **Reject** | Same CLI for humans and agents (non-goal #5). |

**Outstanding deferral.** Tag-registry vocabulary discipline (SPEC-2.0
§2.3.5) — no language-drift evidence yet; revisit if drift becomes
measurable.

---

## How to use this document

1. **When proposing a new RFC:** check it against the litmus test and
   the five non-goals above. If it falls into a non-goal, do not draft
   it as a feature RFC; instead, open an `issue` describing the
   underlying pain and let the discussion find a core-aligned path.
2. **When a spec section conflicts with this document:** the spec is
   wrong. Open an issue/RFC to revise the spec.
3. **When this document needs to change:** propose the change as an
   RFC against this file specifically, with the empirical evidence
   that justifies the shift. The bar for changing the core value is
   higher than the bar for changing any spec.
