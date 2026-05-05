//! v1/v2 event-chain replay machinery (relocated here in v3.1 step 3j).
//!
//! Until v3.1 step 3j the `replay`/`replay_strict`/`replay_thread_at`
//! family lived in `internal::thread`. They are 2.0-era machinery —
//! every helper consumes [`super::event::Event`] /
//! [`super::event::DomainEvent`] and routes through
//! [`super::workflow::SPEC`] for transition validation. Keeping them
//! in `thread.rs` forced 3.0-native code to share a module with the
//! legacy event surface.
//!
//! 3j moves all of it into `internal::legacy::chain_replay`. The
//! 3.0-native [`super::super::thread::replay_thread`] now reads only
//! the snapshot at the thread ref tip; the legacy event-chain reader
//! lives here and is reachable solely from `commands::migrate`
//! (the only ALLOW-listed consumer of `internal::legacy::*` outside
//! the legacy tree itself).
//!
//! Public surface (post-3j):
//!
//! - [`replay`] / [`replay_strict`] / [`replay_strict_unsuppressed`] —
//!   pure event-list projection. Used by the chain readers below and
//!   by the migrate-internal tests that round-trip event fixtures.
//! - [`replay_chain_at`] / [`replay_chain_strict_at`] — walk the
//!   commit chain at a caller-supplied rev (snapshot-aware: a
//!   snapshot ancestor seeds state, the event tail is folded in via
//!   [`apply_event`]). These are the migrate-only equivalents of
//!   the old `thread::replay_thread_at` / `replay_thread_strict_at`.

use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::node::{Node, NodeKind};
use super::super::thread::{materialize_thread_state_from_snapshot, ThreadState, ThreadStatus};
use super::super::validate::StrictReplayIssue;
use super::event::{
    self, node_type_to_kind_and_subtype, DomainEvent, Event, EventMeta, EventType, Lifecycle,
    LinkPayload, ProjectionError,
};

/// Replay events to reconstruct thread state (lenient).
///
/// Silently no-ops on conditions that strict replay would flag (unknown
/// target node, second `facet_set` lifecycle, etc.). Read-side callers want
/// best-effort; doctor / migration / tests want [`replay_strict`].
///
/// Precondition: `events` is in chronological order; first must be `Create`.
pub fn replay(events: &[Event]) -> ForumResult<ThreadState> {
    let (state, _issues) = replay_with_issues(events)?;
    Ok(state)
}

/// Replay events strictly, returning every silent-no-op as a
/// [`StrictReplayIssue`] alongside the final state.
///
/// The state machine is identical to lenient `replay()` (first-write-wins
/// lifecycle, dedup tags, etc.) — strict mode only **observes** the
/// no-ops; it does not abort on them. A fully clean replay returns an
/// empty issue vector.
pub fn replay_strict(events: &[Event]) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    replay_with_issues(events)
}

/// Like [`replay_strict`] but skips the post-pass that suppresses
/// `InvalidTransition` issues whose chain tail has self-healed.
///
/// Used by the workflow-repair tool (#uu9wxn1d) to recover the offending
/// event id even on chains that the public `replay_strict` would have
/// reported as clean. Read-side callers (doctor, search, display) want
/// the suppressed view; only the repair tool needs the raw stream.
pub fn replay_strict_unsuppressed(
    events: &[Event],
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    replay_with_issues_inner(events, /* suppress_self_healed = */ false)
}

fn replay_with_issues(events: &[Event]) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    replay_with_issues_inner(events, true)
}

fn replay_with_issues_inner(
    events: &[Event],
    suppress_self_healed: bool,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    let first = events
        .first()
        .ok_or_else(|| ForumError::StateMachine("no events to replay".into()))?;

    if first.event_type != EventType::Create {
        return Err(ForumError::StateMachine(
            "first event must be 'create'".into(),
        ));
    }

    // Project the create event up-front: the seed needs `kind` + `title`
    // and there is no graceful "skip Create" path (an empty thread is
    // not representable in 2.0).
    let create = first.project().map_err(|e| match e {
        ProjectionError::MissingRequiredField { field } => {
            ForumError::StateMachine(format!("create event missing '{field}'"))
        }
    })?;
    let DomainEvent::Create {
        kind,
        title,
        body,
        branch,
        ..
    } = create
    else {
        return Err(ForumError::StateMachine(
            "first event must be 'create'".into(),
        ));
    };

    // `kind.initial_status()` returns a hardcoded canonical literal
    // (`"draft"` / `"open"`); parse_lenient is total over this input.
    let initial_status = ThreadStatus::parse_lenient(kind.initial_status())
        .expect("kind.initial_status() always returns a canonical 2.0 status name");
    let mut state = ThreadState {
        id: first.thread_id.clone(),
        kind,
        title,
        body,
        branch,
        status: initial_status,
        created_at: first.created_at,
        created_by: first.actor.clone(),
        updated_at: first.created_at,
        nodes: vec![],
        evidence_items: vec![],
        links: vec![],
        body_revision_count: 0,
        incorporated_node_ids: vec![],
        // Phase 2c: lifecycle is always populated. Default is the §2.3.3
        // kind-derived value (a 1.x compat fallback for chains without
        // an explicit `facet_set`); the first explicit `facet_set` then
        // overrides it and flips `lifecycle_explicit` below.
        lifecycle: super::v1::lifecycle_for_legacy_kind(kind),
        lifecycle_explicit: false,
        tags: Vec::new(),
    };

    let mut issues = Vec::new();
    for ev in &events[1..] {
        // Track the most recent event timestamp regardless of projection
        // outcome — display surfaces want a non-stale "updated" column
        // even when the tail event's payload is malformed.
        if ev.created_at > state.updated_at {
            state.updated_at = ev.created_at;
        }
        match ev.project() {
            Ok(domain) => apply_event(&mut state, &domain, &mut issues)?,
            Err(ProjectionError::MissingRequiredField { field }) => {
                issues.push(StrictReplayIssue::MissingRequiredField {
                    event_id: ev.event_id.clone(),
                    event_type: ev.event_type.to_string(),
                    field,
                });
            }
        }
    }
    if suppress_self_healed {
        suppress_self_healed_invalid_transitions(events, &state, &mut issues);
    }
    Ok((state, issues))
}

/// SPEC-2.0 §3.1 / #uu9wxn1d: drop `InvalidTransition` issues whose offending
/// event has been "self-healed" by a subsequent legal corrective sequence.
///
/// A self-heal is recognised when:
/// 1. The chain's final terminal status equals the issue's `to` (the visible
///    state the operator intended).
/// 2. After the offending event, every subsequent `state` event is on a legal
///    edge for the lifecycle.
/// 3. The running state visits at least one non-`to` state and walks back to
///    `to` via legal edges (i.e. the corrective tail is non-trivial).
///
/// Without (3), a chain that simply stops at the offending event would
/// trivially pass — we want to require an explicit operator-emitted
/// corrective sequence (the pattern `state open` → `state rejected` for the
/// `draft → rejected` case). Threads whose terminal sits on a sink state
/// (`withdrawn` in proposal lifecycle) cannot self-heal via append-only
/// because no legal outgoing edge exists; those issues remain reported.
fn suppress_self_healed_invalid_transitions(
    events: &[Event],
    state: &ThreadState,
    issues: &mut Vec<StrictReplayIssue>,
) {
    issues.retain(|issue| {
        let StrictReplayIssue::InvalidTransition {
            event_id,
            to: target,
            ..
        } = issue
        else {
            return true;
        };
        if state.status.as_str() != target {
            return true;
        }
        let Some(idx) = events.iter().position(|e| &e.event_id == event_id) else {
            return true;
        };
        !is_self_healed_after(&events[idx + 1..], state.lifecycle, target)
    });
}

fn is_self_healed_after(tail: &[Event], lifecycle: Lifecycle, target: &str) -> bool {
    let Some(target_status) = ThreadStatus::parse_lenient(target) else {
        return false;
    };
    let mut running = target_status;
    let mut left_target = false;
    for ev in tail {
        if ev.event_type != EventType::State {
            continue;
        }
        let Some(name) = ev.new_state.as_deref() else {
            continue;
        };
        let Some(parsed) = ThreadStatus::parse_lenient(name) else {
            return false;
        };
        if parsed == running {
            continue;
        }
        if !super::workflow::SPEC.is_valid_transition(lifecycle, running.as_str(), parsed.as_str())
        {
            return false;
        }
        running = parsed;
        if running.as_str() != target {
            left_target = true;
        }
        if left_target && running.as_str() == target {
            return true;
        }
    }
    false
}

fn apply_event(
    state: &mut ThreadState,
    event: &DomainEvent,
    issues: &mut Vec<StrictReplayIssue>,
) -> ForumResult<()> {
    match event {
        DomainEvent::State {
            meta,
            new_state,
            approvals,
        } => {
            match ThreadStatus::parse_lenient(new_state) {
                Some(parsed) => {
                    // SPEC-2.0 §3.1 (P0 #34ith16h): strict mode flags
                    // an illegal `from -> to` for the thread's
                    // lifecycle on the per-lifecycle filtered graph.
                    // Lenient mode applies the new status regardless
                    // so legacy chains keep replaying.
                    let from = state.status;
                    if from != parsed
                        && !super::workflow::SPEC.is_valid_transition(
                            state.lifecycle,
                            from.as_str(),
                            parsed.as_str(),
                        )
                    {
                        issues.push(StrictReplayIssue::InvalidTransition {
                            event_id: meta.event_id.clone(),
                            from: from.as_str().to_string(),
                            to: parsed.as_str().to_string(),
                            lifecycle: state.lifecycle.as_str().to_string(),
                        });
                    }
                    state.status = parsed;
                }
                // Lenient: keep the prior status. Strict mode surfaces
                // the unparseable value below.
                None => issues.push(StrictReplayIssue::InvalidStateValue {
                    event_id: meta.event_id.clone(),
                    value: new_state.clone(),
                }),
            }
            // SPEC-2.0 §2.8: 1.x State events carried approvals as a direct
            // field; 2.0 emits them as Approval-typed Say nodes. Synthesize
            // equivalent nodes here so policy guards see one source of truth.
            for approval in approvals {
                state.nodes.push(Node {
                    node_id: format!("{}#{}", meta.event_id, approval.actor_id),
                    node_type: NodeKind::Approval,
                    body: String::new(),
                    actor: approval.actor_id.clone(),
                    created_at: approval.approved_at,
                    ..Node::default()
                });
            }
        }
        DomainEvent::Scope { branch, .. } => {
            // `branch = None` legitimately clears scope; lenient and strict agree.
            state.branch.clone_from(branch);
        }
        DomainEvent::Say {
            meta,
            node_type,
            body,
            reply_to,
            legacy_subtype,
            target_node_id,
        } => {
            // Fold the v2 12-variant NodeType into the SPEC-3.0
            // 4-variant NodeKind. v1 events with rhetorical types
            // (Question/Claim/etc.) collapse to Comment with the
            // label preserved in legacy_subtype.
            let (kind, derived_subtype) = node_type_to_kind_and_subtype(*node_type);
            state.nodes.push(Node {
                node_id: target_node_id
                    .clone()
                    .unwrap_or_else(|| meta.event_id.clone()),
                node_type: kind,
                body: body.clone(),
                actor: meta.actor.clone(),
                created_at: meta.created_at,
                resolved: false,
                retracted: false,
                incorporated: false,
                reply_to: reply_to.clone(),
                legacy_subtype: legacy_subtype.clone().or(derived_subtype),
            });
        }
        DomainEvent::Edit {
            meta,
            target_node_id,
            body,
        } => {
            if let Some(node) = state
                .nodes
                .iter_mut()
                .find(|n| &n.node_id == target_node_id)
            {
                node.body = body.clone();
            } else {
                issues.push(StrictReplayIssue::UnknownTargetNode {
                    event_id: meta.event_id.clone(),
                    event_type: meta.event_type.to_string(),
                    target_node_id: target_node_id.clone(),
                });
            }
        }
        DomainEvent::Retract {
            meta,
            target_node_id,
        } => apply_node_flag(state, meta, target_node_id, issues, |n| n.retracted = true),
        DomainEvent::Resolve {
            meta,
            target_node_id,
        } => apply_node_flag(state, meta, target_node_id, issues, |n| n.resolved = true),
        DomainEvent::Reopen {
            meta,
            target_node_id,
        } => apply_node_flag(state, meta, target_node_id, issues, |n| {
            n.resolved = false;
            n.retracted = false;
            n.incorporated = false;
        }),
        DomainEvent::Retype {
            meta,
            target_node_id,
            node_type,
            ..
        } => {
            if let Some(node) = state
                .nodes
                .iter_mut()
                .find(|n| &n.node_id == target_node_id)
            {
                let (kind, derived_subtype) = node_type_to_kind_and_subtype(*node_type);
                node.node_type = kind;
                if derived_subtype.is_some() {
                    node.legacy_subtype = derived_subtype;
                }
            } else {
                issues.push(StrictReplayIssue::UnknownTargetNode {
                    event_id: meta.event_id.clone(),
                    event_type: meta.event_type.to_string(),
                    target_node_id: target_node_id.clone(),
                });
            }
        }
        DomainEvent::ReviseBody {
            meta,
            body,
            incorporated_node_ids,
        } => {
            state.body = Some(body.clone());
            state.body_revision_count += 1;
            for node_id in incorporated_node_ids {
                let found = state
                    .nodes
                    .iter_mut()
                    .find(|n| n.node_id == *node_id)
                    .map(|node| node.incorporated = true);
                if found.is_none() {
                    issues.push(StrictReplayIssue::UnknownTargetNode {
                        event_id: meta.event_id.clone(),
                        event_type: meta.event_type.to_string(),
                        target_node_id: node_id.clone(),
                    });
                }
                if !state.incorporated_node_ids.contains(node_id) {
                    state.incorporated_node_ids.push(node_id.clone());
                }
            }
        }
        DomainEvent::Link { meta, payload } => match payload {
            LinkPayload::Evidence(ev_data) => {
                let mut ev = ev_data.clone();
                ev.evidence_id = meta.event_id.clone();
                state.evidence_items.push(ev);
            }
            LinkPayload::Thread {
                target_thread_id,
                link_rel,
            } => {
                state.links.push(super::super::thread::ThreadLink {
                    target_thread_id: target_thread_id.clone(),
                    rel: link_rel.clone(),
                });
            }
        },
        // No-ops during replay:
        DomainEvent::Create { .. } => {} // handled in replay() seed before apply_event loop
        DomainEvent::Verify { .. } | DomainEvent::Merge { .. } => {}
        // ADR-010 option (a): unknown variants no-op + emit a strict
        // issue. Unreachable in Phase A; Phase B wires the
        // `EventType::Other(String)` deserialiser to this arm.
        DomainEvent::Unknown { meta, .. } => {
            issues.push(StrictReplayIssue::MissingRequiredField {
                event_id: meta.event_id.clone(),
                event_type: meta.event_type.to_string(),
                field: "unknown_event_type",
            });
        }
        // SPEC-2.0 §2.4.1: per-event facet mutation, not full-state
        // replacement.
        DomainEvent::FacetSet {
            meta,
            lifecycle,
            tags_add,
            tags_remove,
        } => {
            // First-lifecycle-wins: §7.3 makes lifecycle immutable, so any
            // subsequent facet_set carrying `lifecycle` is silently ignored
            // at replay (write-side rejection with FacetTransitionDisallowed
            // is Track B's responsibility).
            if let Some(lc) = lifecycle {
                let parsed = Lifecycle::parse(lc);
                if parsed.is_none() {
                    issues.push(StrictReplayIssue::InvalidLifecycleValue {
                        event_id: meta.event_id.clone(),
                        value: lc.clone(),
                    });
                }
                if let Some(parsed_lc) = parsed {
                    if !state.lifecycle_explicit {
                        // First explicit facet_set wins. Override the
                        // kind-derived default.
                        state.lifecycle = parsed_lc;
                        state.lifecycle_explicit = true;
                    } else if state.lifecycle != parsed_lc {
                        issues.push(StrictReplayIssue::LifecycleResetAttempted {
                            event_id: meta.event_id.clone(),
                            existing: state.lifecycle.as_str().to_string(),
                            attempted: lc.clone(),
                        });
                    }
                    // else: idempotent re-set with the same value — no-op.
                }
            }
            // Within a single event, tags_add is applied before tags_remove
            // (an event that simultaneously adds and removes the same tag
            // is a removal). Insertion is set-style (no duplicates).
            //
            // Tag-grammar validation happens at the migration boundary
            // (`commands::migrate::project_state_to_doc`), NOT here:
            // legacy display surfaces want to render tags verbatim even
            // when they violate the 3.0 grammar (e.g. a 1-char tag
            // accepted by an earlier loose validator). Migration drops
            // invalid tags and records them as `kind: "tag"` omissions
            // in the report (task `9635buy0` objection `e285682f`).
            for tag in tags_add {
                if !state.tags.iter().any(|t| t == tag) {
                    state.tags.push(tag.clone());
                }
            }
            for tag in tags_remove {
                state.tags.retain(|t| t != tag);
            }
        }
    }
    Ok(())
}

/// Shared helper for `Retract` / `Resolve` / `Reopen`: locate the target
/// node by id, apply `mutate`, or record an
/// [`StrictReplayIssue::UnknownTargetNode`]. Projection has already
/// guaranteed a present `target_node_id` for these variants.
fn apply_node_flag(
    state: &mut ThreadState,
    meta: &EventMeta,
    target_node_id: &str,
    issues: &mut Vec<StrictReplayIssue>,
    mutate: impl FnOnce(&mut Node),
) {
    if let Some(node) = state.nodes.iter_mut().find(|n| n.node_id == target_node_id) {
        mutate(node);
    } else {
        issues.push(StrictReplayIssue::UnknownTargetNode {
            event_id: meta.event_id.clone(),
            event_type: meta.event_type.to_string(),
            target_node_id: target_node_id.to_string(),
        });
    }
}

/// Walk the chain at `start_rev` oldest→newest and project a
/// [`ThreadState`].
///
/// Mirror of the v3.0 `thread::replay_thread_at` reader, relocated
/// here in v3.1 step 3j. Used by `commands::migrate` to project a
/// pinned legacy chain. The 3.0-native [`super::super::thread::replay_thread`]
/// no longer dispatches through this — it consumes the snapshot at
/// the thread ref tip directly.
///
/// Mixed chains (snapshot at the bottom, event tail on top) are
/// folded: the snapshot seeds state, then each tail event is
/// applied in chronological order.
pub fn replay_chain_at(git: &GitOps, start_rev: &str) -> ForumResult<ThreadState> {
    // `rev_list` returns newest-first; replay needs oldest-first.
    let mut shas: Vec<String> = git.rev_list(start_rev)?;
    shas.reverse();

    let mut state: Option<ThreadState> = None;
    let mut tail_events: Vec<Event> = Vec::new();
    let mut issues: Vec<StrictReplayIssue> = Vec::new();

    for sha in &shas {
        let listing = git.run(&["ls-tree", "--name-only", sha])?;
        let names: Vec<&str> = listing.lines().collect();
        if names.contains(&"thread.toml") {
            // SPEC-3.0 snapshot commit — reset state to this snapshot's
            // view. Any prior tail events are subsumed.
            let doc = super::super::snapshot::read_snapshot_at(git, sha)?;
            state = Some(materialize_thread_state_from_snapshot(doc));
            tail_events.clear();
        } else if names.contains(&"event.json") {
            // Legacy v1/v2 event commit — accumulate for projection.
            tail_events.push(event::read_event(git, sha)?);
        }
        // Unknown tree shapes (e.g. an empty merge) are skipped; they
        // do not affect state under either storage model.
    }

    if let Some(mut s) = state {
        // Apply any events that landed AFTER the most recent snapshot.
        for ev in &tail_events {
            if ev.created_at > s.updated_at {
                s.updated_at = ev.created_at;
            }
            match ev.project() {
                Ok(domain) => apply_event(&mut s, &domain, &mut issues)?,
                Err(ProjectionError::MissingRequiredField { .. }) => {
                    // Lenient mode: a malformed event is silently
                    // skipped. Strict callers route through
                    // `replay_chain_strict_at` which surfaces this as
                    // `MissingRequiredField`.
                }
            }
        }
        Ok(s)
    } else if !tail_events.is_empty() {
        // Pure legacy event chain.
        replay(&tail_events)
    } else {
        Err(ForumError::Repo(format!(
            "rev {start_rev} has no replayable content"
        )))
    }
}

/// Strict variant of [`replay_chain_at`] — surfaces every silent
/// no-op as a [`StrictReplayIssue`] alongside the materialized
/// state. Mirror of the v3.0 `thread::replay_thread_strict_at`.
pub fn replay_chain_strict_at(
    git: &GitOps,
    start_rev: &str,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    let mut shas: Vec<String> = git.rev_list(start_rev)?;
    shas.reverse();

    let mut state: Option<ThreadState> = None;
    let mut tail_events: Vec<Event> = Vec::new();

    for sha in &shas {
        let listing = git.run(&["ls-tree", "--name-only", sha])?;
        let names: Vec<&str> = listing.lines().collect();
        if names.contains(&"thread.toml") {
            // SPEC-3.0 snapshot ancestor — reset state to its view.
            // Anything before is subsumed; the snapshot's tags/links
            // were already validated at write time, so they enter
            // the strict path without further checks.
            let doc = super::super::snapshot::read_snapshot_at(git, sha)?;
            state = Some(materialize_thread_state_from_snapshot(doc));
            tail_events.clear();
        } else if names.contains(&"event.json") {
            tail_events.push(event::read_event(git, sha)?);
        }
        // Unknown tree shapes (empty merges, etc.) are skipped —
        // same lenience as `replay_chain_at`.
    }

    if let Some(mut s) = state {
        // Snapshot-bottom + event-tail. Apply tail events strictly.
        let mut issues = Vec::new();
        for ev in &tail_events {
            if ev.created_at > s.updated_at {
                s.updated_at = ev.created_at;
            }
            match ev.project() {
                Ok(domain) => apply_event(&mut s, &domain, &mut issues)?,
                Err(ProjectionError::MissingRequiredField { field }) => {
                    issues.push(StrictReplayIssue::MissingRequiredField {
                        event_id: ev.event_id.clone(),
                        event_type: ev.event_type.to_string(),
                        field,
                    });
                }
            }
        }
        Ok((s, issues))
    } else if !tail_events.is_empty() {
        // Pure legacy event chain.
        replay_strict(&tail_events)
    } else {
        Err(ForumError::Repo(format!(
            "rev {start_rev} has no replayable content"
        )))
    }
}

/// Load events from Git and replay strictly, returning every
/// silent-no-op alongside the materialized state.
///
/// Used only by the doctor command via the migrate-internal façade
/// `commands::migrate::replay_thread_strict_via_chain`.
pub fn replay_thread_strict(
    git: &GitOps,
    thread_id: &str,
) -> ForumResult<(ThreadState, Vec<StrictReplayIssue>)> {
    let events = event::load_thread_events(git, thread_id)?;
    replay_strict(&events)
}

#[cfg(test)]
mod tests {
    use super::super::super::thread::ThreadKind;
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_create(thread_id: &str, kind: ThreadKind, title: &str) -> Event {
        Event {
            event_id: "evt-0001".into(),
            thread_id: thread_id.into(),
            event_type: EventType::Create,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            actor: "human/alice".into(),
            title: Some(title.into()),
            kind: Some(kind),
            ..Event::default()
        }
    }

    fn make_state(thread_id: &str, new_state: &str) -> Event {
        Event {
            event_id: "evt-0002".into(),
            thread_id: thread_id.into(),
            event_type: EventType::State,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
            actor: "human/alice".into(),
            new_state: Some(new_state.into()),
            ..Event::default()
        }
    }

    #[test]
    fn replay_single_create() {
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "Test RFC")];
        let state = replay(&events).unwrap();
        assert_eq!(state.id, "RFC-0001");
        assert_eq!(state.kind, ThreadKind::Rfc);
        assert_eq!(state.title, "Test RFC");
        assert_eq!(state.body, None);
        assert_eq!(state.status, "draft");
        assert_eq!(state.created_by, "human/alice");
    }

    #[test]
    fn replay_create_then_state() {
        // Phase 2a: 1.x "proposed" is normalized by parse_lenient into the
        // canonical 2.0 status `Open`.
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "Test RFC"),
            make_state("RFC-0001", "proposed"),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Open);
    }

    #[test]
    fn replay_empty_events_fails() {
        let result = replay(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn replay_non_create_first_fails() {
        let events = vec![make_state("RFC-0001", "proposed")];
        let result = replay(&events);
        assert!(result.is_err());
    }

    #[test]
    fn replay_issue_initial_status() {
        let events = vec![make_create("ISSUE-0001", ThreadKind::Issue, "Bug")];
        let state = replay(&events).unwrap();
        assert_eq!(state.status, "open");
    }

    // ---- facet_set replay (SPEC-2.0 §2.4.1) ----

    fn make_facet_set(
        thread_id: &str,
        seq: u32,
        lifecycle: Option<&str>,
        tags_add: &[&str],
        tags_remove: &[&str],
    ) -> Event {
        Event {
            event_id: format!("evt-facet-{seq:04}"),
            thread_id: thread_id.into(),
            event_type: EventType::FacetSet,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, seq.min(59), 0).unwrap(),
            actor: "human/alice".into(),
            lifecycle: lifecycle.map(str::to_string),
            tags_add: tags_add.iter().map(|s| s.to_string()).collect(),
            tags_remove: tags_remove.iter().map(|s| s.to_string()).collect(),
            ..Event::default()
        }
    }

    #[test]
    fn facet_set_first_lifecycle_wins() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 2, Some("execution"), &[], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
    }

    #[test]
    fn facet_set_lifecycle_optional() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &[], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(!state.lifecycle_explicit);
        assert!(state.tags.is_empty());
    }

    #[test]
    fn facet_set_tags_add_then_remove_within_event() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &["bug", "ux"], &["bug"]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.tags, vec!["ux".to_string()]);
    }

    #[test]
    fn facet_set_tags_accumulate_across_events() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &["a", "b"], &[]),
            make_facet_set("RFC-0001", 2, None, &["c"], &["a"]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.tags, vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn facet_set_tags_add_dedupes() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &["bug"], &[]),
            make_facet_set("RFC-0001", 2, None, &["bug"], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.tags, vec!["bug".to_string()]);
    }

    #[test]
    fn lifecycle_accessor_falls_back_to_kind() {
        let state = replay(&[make_create("RFC-0001", ThreadKind::Rfc, "T")]).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(!state.lifecycle_explicit);

        let state = replay(&[make_create("ASK-0001", ThreadKind::Issue, "T")]).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Execution);
        assert!(!state.lifecycle_explicit);

        let state = replay(&[make_create("DEC-0001", ThreadKind::Dec, "T")]).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Record);
        assert!(!state.lifecycle_explicit);
    }

    #[test]
    fn lifecycle_accessor_prefers_explicit_facet_set() {
        let events = vec![
            make_create("ASK-0001", ThreadKind::Issue, "T"),
            make_facet_set("ASK-0001", 1, Some("record"), &[], &[]),
        ];
        let state = replay(&events).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Record);
        assert!(state.lifecycle_explicit);
    }

    #[test]
    fn facet_set_tags_remove_unknown_is_noop() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, None, &[], &["nonexistent"]),
        ];
        let state = replay(&events).unwrap();
        assert!(state.tags.is_empty());
    }

    // ---- replay_strict ----

    fn make_resolve(thread_id: &str, target: &str, seq: u32) -> Event {
        Event {
            event_id: format!("evt-resolve-{seq:04}"),
            thread_id: thread_id.into(),
            event_type: EventType::Resolve,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, seq.min(59), 0).unwrap(),
            actor: "human/alice".into(),
            target_node_id: Some(target.into()),
            ..Event::default()
        }
    }

    fn make_edit(thread_id: &str, target: &str, body: Option<&str>, seq: u32) -> Event {
        Event {
            event_id: format!("evt-edit-{seq:04}"),
            thread_id: thread_id.into(),
            event_type: EventType::Edit,
            created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, seq.min(59), 0).unwrap(),
            actor: "human/alice".into(),
            target_node_id: Some(target.into()),
            body: body.map(str::to_string),
            ..Event::default()
        }
    }

    #[test]
    fn replay_strict_clean_thread_yields_no_issues() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &["bug"], &[]),
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert!(issues.is_empty(), "unexpected issues: {issues:?}");
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
    }

    #[test]
    fn replay_strict_flags_resolve_on_unknown_node() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_resolve("RFC-0001", "ghost-node", 1),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(matches!(
            issues.as_slice(),
            [StrictReplayIssue::UnknownTargetNode { target_node_id, .. }] if target_node_id == "ghost-node"
        ));
    }

    #[test]
    fn replay_strict_flags_edit_missing_body() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_edit("RFC-0001", "any-node", None, 1),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert_eq!(issues.len(), 1, "got: {issues:?}");
        assert!(matches!(
            &issues[0],
            StrictReplayIssue::MissingRequiredField { field, .. } if *field == "body"
        ));
    }

    #[test]
    fn replay_strict_flags_lifecycle_reset() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 2, Some("execution"), &[], &[]),
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
        assert!(matches!(
            issues.as_slice(),
            [StrictReplayIssue::LifecycleResetAttempted { existing, attempted, .. }]
                if existing == "proposal" && attempted == "execution"
        ));
    }

    #[test]
    fn replay_strict_idempotent_lifecycle_reset_is_clean() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 2, Some("proposal"), &[], &[]),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues.is_empty(),
            "idempotent re-set should not flag: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_flags_invalid_lifecycle_value() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_facet_set("RFC-0001", 1, Some("nonsense"), &[], &[]),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues.iter().any(|i| matches!(
                i,
                StrictReplayIssue::InvalidLifecycleValue { value, .. } if value == "nonsense"
            )),
            "got: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_flags_state_event_missing_new_state() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            Event {
                event_id: "evt-state-bad".into(),
                thread_id: "RFC-0001".into(),
                event_type: EventType::State,
                created_at: Utc.with_ymd_and_hms(2026, 1, 1, 0, 1, 0).unwrap(),
                actor: "human/alice".into(),
                ..Event::default()
            },
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(matches!(
            issues.as_slice(),
            [StrictReplayIssue::MissingRequiredField { field, .. }] if *field == "new_state"
        ));
    }

    #[test]
    fn replay_strict_flags_illegal_transition_for_lifecycle() {
        let mut state_event = make_state("RFC-0001", "review");
        state_event.event_id = "evt-illegal".into();
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "open"),
            make_state("RFC-0001", "done"),
            state_event,
        ];
        let (final_state, issues) = replay_strict(&events).unwrap();
        assert_eq!(final_state.status, ThreadStatus::Review);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                StrictReplayIssue::InvalidTransition {
                    event_id, from, to, lifecycle
                } if event_id == "evt-illegal"
                    && from == "done"
                    && to == "review"
                    && lifecycle == "proposal"
            )),
            "expected an InvalidTransition issue, got: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_clean_for_legal_transition() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "open"),
            make_state("RFC-0001", "review"),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "legal transition should not flag: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_legacy_state_synonyms_remain_legal() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "proposed"),
            make_state("RFC-0001", "under-review"),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues.is_empty(),
            "1.x synonyms on a legal path should not flag: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_idempotent_state_does_not_flag() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_state("RFC-0001", "draft"),
        ];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(issues.is_empty(), "idempotent re-state: {issues:?}");
    }

    #[test]
    fn replay_strict_self_heal_suppresses_invalid_transition_after_corrective_tail() {
        let mut bad = make_state("RFC-0001", "rejected");
        bad.event_id = "evt-bad".into();
        let mut fix1 = make_state("RFC-0001", "open");
        fix1.event_id = "evt-fix1".into();
        let mut fix2 = make_state("RFC-0001", "rejected");
        fix2.event_id = "evt-fix2".into();
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            bad,
            fix1,
            fix2,
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Rejected);
        assert!(
            !issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "self-healed chain must not surface InvalidTransition: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_no_self_heal_without_corrective_tail() {
        let mut bad = make_state("RFC-0001", "rejected");
        bad.event_id = "evt-bad".into();
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "T"), bad];
        let (_, issues) = replay_strict(&events).unwrap();
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "no corrective tail → issue must remain: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_no_self_heal_when_terminal_status_differs() {
        let mut bad = make_state("RFC-0001", "rejected");
        bad.event_id = "evt-bad".into();
        let mut fix1 = make_state("RFC-0001", "open");
        fix1.event_id = "evt-fix1".into();
        let events = vec![make_create("RFC-0001", ThreadKind::Rfc, "T"), bad, fix1];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Open);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "terminal mismatch → issue must remain: {issues:?}"
        );
    }

    #[test]
    fn replay_strict_review_to_withdrawn_cannot_self_heal() {
        let mut intake = make_state("RFC-0001", "open");
        intake.event_id = "evt-intake".into();
        let mut review = make_state("RFC-0001", "review");
        review.event_id = "evt-review".into();
        let mut bad = make_state("RFC-0001", "withdrawn");
        bad.event_id = "evt-bad".into();
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            intake,
            review,
            bad,
        ];
        let (state, issues) = replay_strict(&events).unwrap();
        assert_eq!(state.status, ThreadStatus::Withdrawn);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, StrictReplayIssue::InvalidTransition { .. })),
            "Category B (review→withdrawn) cannot self-heal: {issues:?}"
        );
    }

    #[test]
    fn replay_lenient_unchanged_under_strict_failures() {
        let events = vec![
            make_create("RFC-0001", ThreadKind::Rfc, "T"),
            make_resolve("RFC-0001", "ghost-node", 1),
            make_facet_set("RFC-0001", 2, Some("proposal"), &[], &[]),
            make_facet_set("RFC-0001", 3, Some("execution"), &[], &[]),
        ];
        let state = replay(&events).expect("lenient replay must still succeed");
        assert_eq!(state.lifecycle, Lifecycle::Proposal);
        assert!(state.lifecycle_explicit);
    }
}
