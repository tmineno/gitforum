use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType, NodeType};
use super::git_ops::GitOps;
use super::node::Node;
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
    /// Distinct actor IDs whose `approval`-typed Say nodes will be emitted
    /// before the State event (SPEC-2.0 §2.8).
    pub approve_actors: Vec<String>,
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

    let lifecycle = state.lifecycle();
    if !state_machine::is_valid_transition(lifecycle, &from, new_state) {
        let valid = state_machine::valid_targets(lifecycle, &from);
        let valid_msg = if valid.is_empty() {
            "none".to_string()
        } else {
            valid.join(", ")
        };
        return Err(ForumError::StateMachine(format!(
            "transition {from}->{new_state} is not valid for {lifecycle}; valid transitions from '{from}': [{valid_msg}]",
        )));
    }

    let now = clock.now();
    // Deduplicate approval actors to prevent forged duplicate approvals.
    let mut seen = std::collections::HashSet::new();
    let approve_actors: Vec<String> = approve_actors
        .iter()
        .filter(|a| seen.insert(a.as_str()))
        .cloned()
        .collect();

    let resolve_action_ids = if options.resolve_open_actions
        && matches!(
            state.kind,
            super::event::ThreadKind::Issue | super::event::ThreadKind::Task
        )
        && state_machine::normalize_state_name(new_state) == "done"
    {
        state
            .open_actions()
            .iter()
            .map(|node| node.node_id.clone())
            .collect()
    } else {
        Vec::new()
    };

    // Build the post-write effective state for guard evaluation:
    //   1. Mark scheduled action resolutions as resolved.
    //   2. Splice in synthetic Approval-typed nodes for each approve actor —
    //      these are the nodes that will be written if guards pass
    //      (SPEC-2.0 §2.8).
    let effective_state = {
        let mut effective = state.clone();
        for node in &mut effective.nodes {
            if resolve_action_ids.iter().any(|id| id == &node.node_id) {
                node.resolved = true;
            }
        }
        for actor_id in &approve_actors {
            effective.nodes.push(Node {
                node_id: format!("pending-approval/{actor_id}"),
                node_type: NodeType::Approval,
                actor: actor_id.clone(),
                created_at: now,
                ..Node::default()
            });
        }
        effective
    };

    let violations = super::policy::check_guards(policy, &effective_state, &from, new_state);
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
        approve_actors,
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

    // SPEC-2.0 §2.8: emit `approval`-typed Say nodes (one per approver) before
    // the State event. The 1.x-style `Event.approvals` field is no longer
    // populated by 2.0 writes — replay still synthesizes nodes from it for
    // legacy reads.
    for approver in &plan.approve_actors {
        write_ops::say_node(
            git,
            thread_id,
            NodeType::Approval,
            "",
            approver,
            clock,
            None,
        )?;
    }

    // SPEC-2.0 §3.1: persist transitions in 2.0 state names so replay /
    // policy / search see a single canonical vocabulary regardless of the
    // 1.x verb the caller passed in.
    let canonical_state = state_machine::normalize_state_name(new_state);
    let mut ev =
        Event::base(thread_id, EventType::State, actor, clock).with_new_state(canonical_state);
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
    let lifecycle = state.lifecycle();
    // Normalize the target so legacy 1.x state names line up with the
    // 2.0-name path returned by find_path (otherwise the final-step check
    // below misses).
    let normalized_target = state_machine::normalize_state_name(target);
    let path =
        state_machine::find_path(lifecycle, &state.status, normalized_target).ok_or_else(|| {
            ForumError::StateMachine(format!(
                "no path from '{}' to '{}' for {lifecycle}",
                state.status, target,
            ))
        })?;

    if path.is_empty() {
        return Ok(vec![]);
    }

    let mut walked = Vec::new();
    for step in &path {
        // Only pass approve_actors, comment, and resolve_open_actions on the final step
        let is_final = *step == normalized_target;
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

pub fn remediation_hint(rule: &str, state: &thread::ThreadState, thread_id: &str) -> String {
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
