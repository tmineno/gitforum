# git-forum Core Value

Last updated: 2026-05-03
Status: **Authoritative**. Upstream of specs, ADRs, and implementation
decisions.

This document describes product values and boundaries. It does not define
release-specific storage layout, command syntax, state machines, compatibility
formats, or migration algorithms.

---

## Core Statement

**git-forum keeps repository deliberation, decisions, and implementation context
together inside Git so humans and AI coding agents can work as peers on the same
codebase.**

Design intent and implementation code drift apart when they live in separate
tools, separate ID spaces, or separate comment streams. git-forum exists so a
future contributor can answer "why is this code written this way?" from
artifacts that travel with the repository.

The durable commitments are:

1. forum state belongs to the repository;
2. Git owns storage, history, reachability, and transport;
3. humans and AI agents use the same product semantics;
4. discussion stays connected to the code it concerns.

---

## Principles

### Repository-Local Continuity

Forum state is part of the repository's long-lived context, not part of the
currently checked-out source branch, an external service, an editor session, or
an agent run.

Switching branches or linked worktrees should not split the local forum record.
Clone-to-clone sharing should use ordinary Git object and ref transport.

### Code-Adjacent Deliberation

Discussion must remain close enough to the code that it can explain and justify
changes. Threads should be able to refer to commits, files, tests, documents,
implementation branches, and other relevant repository artifacts.

Those links are explanatory context. They must not turn git-forum into a
cross-thread workflow engine.

### Human-Agent Parity

Humans and AI coding agents are peer participants. The same concepts, policy
semantics, mutation paths, and audit trail must apply to both.

git-forum should not grow AI-only commands, hidden agent coordination channels,
or privileged automation semantics. An agent may use the tool quickly or
programmatically, but it is still acting through the same forum model as a
human.

### Git-Native History And Distribution

git-forum should adapt its data model to Git rather than reinventing Git around
the forum. Git is responsible for object storage, history, reachability,
branching, fetching, pushing, and low-level conflict visibility.

When forum distribution or history feels hard, the preferred answer is to
simplify the data layout so normal Git behavior is sufficient, not to add a
separate transport protocol or live database.

### Current-State Clarity

Common reads should answer "what is true now?" directly. History is still useful
and should remain inspectable through Git, but the live product model should not
be organized around preserving every past internal representation forever.

Backward compatibility is a transition concern. The product should preserve
human-meaningful content and relationships; it does not need to preserve old
runtime machinery when that machinery no longer serves the product.

### Bounded Policy

Blocking policy should be local to the thread being changed: its current
classification, status, content, and direct discussion state.

Cross-thread information may be displayed as context, but it must not block an
operation. If a proposed feature needs another thread's state to decide whether
the current thread may change, it has crossed the core boundary.

### Small Core, Rich Clients

Interactive, terminal, scripted, and integration clients may present different
workflows, but they should share the same product semantics and mutation layer.

Clients can be richer than the core model. They must not become alternate
sources of truth, alternate policy engines, or compatibility paths that keep
obsolete models alive in normal use.

---

## Boundaries

The following are in scope when they serve the principles above:

- repository-local discussion threads;
- current thread state and direct discussion artifacts;
- code linkage through commits, files, tests, documents, branches, and related
  repository artifacts;
- local policy that blocks only by inspecting the thread being changed;
- transition from older forum records into the current product model;
- human and AI participation through shared semantics.

The following are outside the core product boundary:

- cross-thread workflow enforcement;
- automatic transitions triggered by other threads;
- agent dispatch, leases, scheduling, or resource allocation;
- project-management dashboards such as velocity, burndown, SLA, and capacity
  tracking;
- custom clone-to-clone distribution or merge protocols above ordinary Git;
- primary proprietary or web-only interaction surfaces;
- separate human-only and AI-only command sets;
- exact runtime compatibility with obsolete internal models after transition.

---

## Empirical Basis

This repository's own use has shaped these boundaries:

- `rfc` has behaved like a protocol-shaped discussion form, while issue, task,
  bug, and decision labels have been less consistently separated.
- Accepted product decisions have been made by both human and AI actors, so
  human-agent parity is an observed operating model rather than an aspiration.
- Several proposals attempted cross-thread workflow enforcement, agent
  coordination, or project-management expansion; they repeatedly failed to
  become durable product direction.

The lesson is not a specific 3.0 mechanism. The lesson is that git-forum works
best when it stays repository-local, Git-native, code-adjacent, and small enough
that humans and AI agents can share one coherent model.

---

## How To Use This Document

When writing a spec, ADR, or implementation plan:

1. Start from the principles, then choose mechanisms.
2. If a mechanism violates a boundary, redesign it as advisory context or remove
   it from core.
3. If a spec conflicts with this document, revise the spec.
4. Change this document only when there is evidence that the product purpose or
   boundary has changed.
