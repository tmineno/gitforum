use super::approval::{Approval, ApprovalMechanism};
use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType};
use super::git_ops::GitOps;
use super::policy::Policy;
use super::state_machine;
use super::thread;
use super::write_ops;

#[derive(Debug, Clone, Default)]
pub struct StateChangeOptions {
    pub resolve_open_actions: bool,
    pub comment: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StateChangePlan {
    pub from_state: String,
    pub approvals: Vec<Approval>,
    pub resolve_action_ids: Vec<String>,
}

pub fn prepare_state_change(
    git: &GitOps,
    thread_id: &str,
    new_state: &str,
    approve_actors: &[String],
    clock: &dyn Clock,
    policy: &Policy,
    options: StateChangeOptions,
) -> ForumResult<StateChangePlan> {
    let state = thread::replay_thread(git, thread_id)?;
    let from = state.status.clone();

    if !state_machine::is_valid_transition(state.kind, &from, new_state) {
        let valid = state_machine::valid_targets(state.kind, &from);
        let valid_msg = if valid.is_empty() {
            "none".to_string()
        } else {
            valid.join(", ")
        };
        return Err(ForumError::StateMachine(format!(
            "transition {from}->{new_state} is not valid for {:?}; valid transitions from '{from}': [{valid_msg}]",
            state.kind
        )));
    }

    let now = clock.now();
    // Deduplicate approval actors to prevent forged duplicate approvals.
    let mut seen = std::collections::HashSet::new();
    let approvals: Vec<Approval> = approve_actors
        .iter()
        .filter(|a| seen.insert(a.as_str()))
        .map(|a| Approval {
            actor_id: a.clone(),
            approved_at: now,
            mechanism: ApprovalMechanism::Recorded,
            key_id: None,
            proof_ref: None,
        })
        .collect();

    let resolve_action_ids = if options.resolve_open_actions
        && matches!(
            state.kind,
            super::event::ThreadKind::Issue | super::event::ThreadKind::Task
        )
        && new_state == "closed"
    {
        state
            .open_actions()
            .iter()
            .map(|node| node.node_id.clone())
            .collect()
    } else {
        Vec::new()
    };

    let effective_state = if resolve_action_ids.is_empty() {
        state
    } else {
        let mut effective = state.clone();
        for node in &mut effective.nodes {
            if resolve_action_ids.iter().any(|id| id == &node.node_id) {
                node.resolved = true;
            }
        }
        effective
    };

    let violations =
        super::policy::check_guards(policy, &effective_state, &from, new_state, &approvals);
    if !violations.is_empty() {
        let msgs: Vec<String> = violations
            .iter()
            .map(|v| {
                let hint = remediation_hint(&v.rule, &effective_state, thread_id);
                if hint.is_empty() {
                    format!("{}: {}", v.rule, v.reason)
                } else {
                    format!("{}: {} — {}", v.rule, v.reason, hint)
                }
            })
            .collect();
        return Err(ForumError::Policy(msgs.join("; ")));
    }

    Ok(StateChangePlan {
        from_state: from,
        approvals,
        resolve_action_ids,
    })
}

/// Attempt a thread state transition, checking the state machine and policy guards.
///
/// Preconditions: thread_id exists; approve_actors is the list of approving actor IDs.
/// Postconditions: on success, a State event with attached approvals is written.
/// Failure modes: ForumError::StateMachine if transition invalid; ForumError::Policy if guards fail.
/// Side effects: writes git objects, updates ref.
#[allow(clippy::too_many_arguments)]
pub fn change_state(
    git: &GitOps,
    thread_id: &str,
    new_state: &str,
    approve_actors: &[String],
    actor: &str,
    clock: &dyn Clock,
    policy: &Policy,
    options: StateChangeOptions,
) -> ForumResult<()> {
    let comment = options.comment.clone();
    let plan = prepare_state_change(
        git,
        thread_id,
        new_state,
        approve_actors,
        clock,
        policy,
        options,
    )?;

    for node_id in &plan.resolve_action_ids {
        write_ops::resolve_node(git, thread_id, node_id, actor, clock)?;
    }

    let mut ev = Event::base(thread_id, EventType::State, actor, clock)
        .with_new_state(new_state)
        .with_approvals(plan.approvals);
    if let Some(ref text) = comment {
        ev = ev.with_body(text);
    }
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Walk through intermediate states to reach `target`, checking guards at each step.
///
/// Preconditions: thread_id exists; target is a valid state for the thread's kind.
/// Postconditions: on success, one State event per step is written; returns list of states walked.
/// Failure modes: ForumError::StateMachine if no path exists; ForumError::Policy if a guard fails
///   (thread is left at the last successfully transitioned state).
/// Side effects: writes git objects, updates ref (one event per intermediate step).
#[allow(clippy::too_many_arguments)]
pub fn fast_track_state(
    git: &GitOps,
    thread_id: &str,
    target: &str,
    approve_actors: &[String],
    actor: &str,
    clock: &dyn Clock,
    policy: &Policy,
    options: StateChangeOptions,
) -> ForumResult<Vec<String>> {
    let state = thread::replay_thread(git, thread_id)?;
    let path = state_machine::find_path(state.kind, &state.status, target).ok_or_else(|| {
        ForumError::StateMachine(format!(
            "no path from '{}' to '{}' for {:?}",
            state.status, target, state.kind
        ))
    })?;

    if path.is_empty() {
        return Ok(vec![]);
    }

    let mut walked = Vec::new();
    for step in &path {
        // Only pass approve_actors, comment, and resolve_open_actions on the final step
        let is_final = *step == target;
        let step_sign = if is_final { approve_actors } else { &[] };
        let step_options = if is_final {
            options.clone()
        } else {
            StateChangeOptions::default()
        };
        change_state(
            git,
            thread_id,
            step,
            step_sign,
            actor,
            clock,
            policy,
            step_options,
        )?;
        walked.push(step.to_string());
    }
    Ok(walked)
}

fn remediation_hint(rule: &str, state: &thread::ThreadState, thread_id: &str) -> String {
    match rule {
        "no_open_actions" => {
            let ids: Vec<String> = state
                .open_actions()
                .iter()
                .map(|n| n.node_id[..n.node_id.len().min(16)].to_string())
                .collect();
            if ids.is_empty() {
                return String::new();
            }
            format!(
                "resolve each with `resolve {thread_id} <NODE_ID>` (open: {}) or use --resolve-open-actions",
                ids.join(", ")
            )
        }
        "no_open_objections" => {
            let ids: Vec<String> = state
                .open_objections()
                .iter()
                .map(|n| n.node_id[..n.node_id.len().min(16)].to_string())
                .collect();
            if ids.is_empty() {
                return String::new();
            }
            format!(
                "resolve each with `resolve {thread_id} <NODE_ID>` (open: {})",
                ids.join(", ")
            )
        }
        "at_least_one_summary" => {
            format!("add a summary first: `summary {thread_id} \"<text>\"`")
        }
        "one_human_approval" => "supply --approve human/<name>".to_string(),
        _ => String::new(),
    }
}
