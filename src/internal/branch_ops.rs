use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType};
use super::git_ops::GitOps;

/// Bind or clear a thread's branch scope.
///
/// Preconditions: thread_id exists; when `branch` is Some, the branch exists in `refs/heads/`.
/// Postconditions: a Scope event is written and the thread ref is updated.
/// Failure modes: ForumError::Repo if the branch does not exist; ForumError::Git on subprocess failure.
/// Side effects: writes git objects, updates ref.
pub fn set_branch(
    git: &GitOps,
    thread_id: &str,
    branch: Option<&str>,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<()> {
    if let Some(branch) = branch {
        let refname = format!("refs/heads/{branch}");
        if git.resolve_ref(&refname)?.is_none() {
            return Err(ForumError::Repo(format!(
                "branch '{branch}' does not exist in this repository"
            )));
        }
    }

    let ev = Event {
        event_id: String::new(),
        thread_id: thread_id.to_string(),
        event_type: EventType::Scope,
        created_at: clock.now(),
        actor: actor.to_string(),
        base_rev: None,
        parents: vec![],
        title: None,
        kind: None,
        body: None,
        node_type: None,
        target_node_id: None,
        new_state: None,
        approvals: vec![],
        evidence: None,
        link_rel: None,
        branch: branch.map(str::to_string),
    };
    super::event::write_event(git, &ev)?;
    Ok(())
}
