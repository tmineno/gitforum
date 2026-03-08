use super::approval::{Approval, ApprovalMechanism};
use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType, NodeType};
use super::git_ops::GitOps;
use super::id::IdGenerator;
use super::policy::Policy;
use super::state_machine;
use super::thread;

/// Add a typed discussion node to a thread.
///
/// Preconditions: `git` is bound to an initialised git-forum repo; thread_id exists.
/// Postconditions: a Say event is written and the thread ref updated.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn say_node(
    git: &GitOps,
    thread_id: &str,
    node_type: NodeType,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
) -> ForumResult<String> {
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Say,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: Some(body.to_string()),
        node_type: Some(node_type),
        target_node_id: None,
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)
}

/// Revise the body of an existing node.
///
/// Preconditions: thread_id and node_id exist.
/// Postconditions: an Edit event is written with the new body.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn revise_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
) -> ForumResult<()> {
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Edit,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: Some(body.to_string()),
        node_type: None,
        target_node_id: Some(node_id.to_string()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Retract a node (soft-delete: marks retracted in replay).
///
/// Preconditions: thread_id and node_id exist.
/// Postconditions: a Retract event is written.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn retract_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
) -> ForumResult<()> {
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Retract,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: Some(node_id.to_string()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Resolve a node (marks it addressed, e.g. an objection that has been answered).
///
/// Preconditions: thread_id and node_id exist.
/// Postconditions: a Resolve event is written.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn resolve_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
) -> ForumResult<()> {
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Resolve,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: Some(node_id.to_string()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Reopen a resolved or retracted node.
///
/// Preconditions: thread_id and node_id exist.
/// Postconditions: a Reopen event is written.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn reopen_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
) -> ForumResult<()> {
    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Reopen,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: Some(node_id.to_string()),
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Attempt a thread state transition, checking the state machine and policy guards.
///
/// Preconditions: thread_id exists; sign_actors is the list of approving actor IDs.
/// Postconditions: on success, a State event with attached approvals is written.
/// Failure modes: ForumError::StateMachine if transition invalid; ForumError::Policy if guards fail.
/// Side effects: writes git objects, updates ref.
#[allow(clippy::too_many_arguments)]
pub fn change_state(
    git: &GitOps,
    thread_id: &str,
    new_state: &str,
    sign_actors: &[String],
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
    policy: &Policy,
) -> ForumResult<()> {
    let state = thread::replay_thread(git, thread_id)?;
    let from = &state.status.clone();

    if !state_machine::is_valid_transition(state.kind, from, new_state) {
        return Err(ForumError::StateMachine(format!(
            "transition {from}->{new_state} is not valid for {:?}",
            state.kind
        )));
    }

    let now = clock.now();
    let approvals: Vec<Approval> = sign_actors
        .iter()
        .map(|a| Approval {
            actor_id: a.clone(),
            approved_at: now,
            mechanism: ApprovalMechanism::Recorded,
            key_id: None,
            proof_ref: None,
        })
        .collect();

    let violations = super::policy::check_guards(policy, &state, from, new_state, &approvals);
    if !violations.is_empty() {
        let msgs: Vec<String> = violations
            .iter()
            .map(|v| format!("{}: {}", v.rule, v.reason))
            .collect();
        return Err(ForumError::Policy(msgs.join("; ")));
    }

    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::State,
        created_at: now,
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: None,
        new_state: Some(new_state.to_string()),
        approvals,
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &ev)?;
    Ok(())
}
