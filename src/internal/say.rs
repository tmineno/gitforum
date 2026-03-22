use chrono::{DateTime, Utc};

use super::clock::Clock;
use super::error::ForumResult;
use super::event::{Event, EventType, NodeType};
use super::git_ops::GitOps;
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
    reply_to: Option<&str>,
) -> ForumResult<String> {
    say_node_core(
        git, thread_id, node_type, body, actor, clock, reply_to, None,
    )
}

/// Add a typed discussion node with a timestamp override.
///
/// Like `say_node`, but uses the given `created_at` instead of the clock.
/// Intended for import/migration scenarios.
#[allow(clippy::too_many_arguments)]
pub fn say_node_with_timestamp(
    git: &GitOps,
    thread_id: &str,
    node_type: NodeType,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    reply_to: Option<&str>,
    created_at: DateTime<Utc>,
) -> ForumResult<String> {
    say_node_core(
        git,
        thread_id,
        node_type,
        body,
        actor,
        clock,
        reply_to,
        Some(created_at),
    )
}

#[allow(clippy::too_many_arguments)]
fn say_node_core(
    git: &GitOps,
    thread_id: &str,
    node_type: NodeType,
    body: &str,
    actor: &str,
    clock: &dyn Clock,
    reply_to: Option<&str>,
    created_at: Option<DateTime<Utc>>,
) -> ForumResult<String> {
    let mut ev = Event::base(thread_id, EventType::Say, actor, clock)
        .with_body(body)
        .with_node_type(node_type)
        .with_reply_to(reply_to);
    if let Some(ts) = created_at {
        ev = ev.with_created_at(ts);
    }
    super::event::write_event(git, &ev)
}

/// Revise the body of a thread, optionally incorporating referenced nodes.
///
/// Preconditions: thread_id exists; all incorporated node IDs must exist in the thread.
/// Postconditions: a ReviseBody event is written with the new body.
/// Failure modes: ForumError::Git on subprocess failure; ForumError::Repo if
///   an incorporated node ID is not found in the thread.
/// Side effects: writes git objects, updates ref.
pub fn revise_body(
    git: &GitOps,
    thread_id: &str,
    body: &str,
    incorporates: &[String],
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    let resolved_ids: Vec<String> = if incorporates.is_empty() {
        vec![]
    } else {
        incorporates
            .iter()
            .map(|id| thread::resolve_node_id_in_thread(git, thread_id, id))
            .collect::<Result<Vec<_>, _>>()?
    };
    let ev = Event::base(thread_id, EventType::ReviseBody, actor, clock)
        .with_body(body)
        .with_incorporated_node_ids(resolved_ids);
    super::event::write_event(git, &ev)?;
    Ok(())
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
) -> ForumResult<()> {
    let ev = Event::base(thread_id, EventType::Edit, actor, clock)
        .with_body(body)
        .with_target_node_id(node_id);
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Apply a lifecycle event (Retract, Resolve, or Reopen) to a node.
///
/// Preconditions: thread_id and node_id exist; event_type is Retract, Resolve, or Reopen.
/// Postconditions: the corresponding event is written.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn node_lifecycle(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
    event_type: EventType,
) -> ForumResult<()> {
    let ev = Event::base(thread_id, event_type, actor, clock).with_target_node_id(node_id);
    super::event::write_event(git, &ev)?;
    Ok(())
}

/// Retract a node (soft-delete: marks retracted in replay).
pub fn retract_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    node_lifecycle(git, thread_id, node_id, actor, clock, EventType::Retract)
}

/// Resolve a node (marks it addressed, e.g. an objection that has been answered).
pub fn resolve_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    node_lifecycle(git, thread_id, node_id, actor, clock, EventType::Resolve)
}

/// Reopen a resolved or retracted node.
pub fn reopen_node(
    git: &GitOps,
    thread_id: &str,
    node_id: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    node_lifecycle(git, thread_id, node_id, actor, clock, EventType::Reopen)
}
