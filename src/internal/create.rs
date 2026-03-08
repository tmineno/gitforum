use super::clock::Clock;
use super::error::ForumResult;
use super::event::{Event, EventType, ThreadKind};
use super::git_ops::GitOps;
use super::id::IdGenerator;
use super::id_alloc;

/// Create a new thread, store a `create` event, and return the thread ID.
///
/// Preconditions: `git` is bound to an initialised git-forum repo.
///
/// Postconditions:
/// - `refs/forum/threads/<THREAD_ID>` is created pointing to a commit.
/// - The commit tree contains a valid `event.json`.
///
/// Failure modes: ForumError::Git on subprocess failure.
///
/// Side effects: writes git objects and updates a ref.
pub fn create_thread(
    git: &GitOps,
    kind: ThreadKind,
    title: &str,
    body: Option<&str>,
    actor: &str,
    clock: &dyn Clock,
    _ids: &dyn IdGenerator,
) -> ForumResult<String> {
    let thread_id = id_alloc::alloc_thread_id(git, kind)?;
    let event = Event {
        event_id: String::new(),
        thread_id: thread_id.clone(),
        event_type: EventType::Create,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: Some(title.to_string()),
        kind: Some(kind),
        body: body.map(str::to_string),
        node_type: None,
        target_node_id: None,
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        run_label: None,
    };
    super::event::write_event(git, &event)?;
    Ok(thread_id)
}
