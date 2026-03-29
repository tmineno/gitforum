use chrono::{DateTime, Utc};

use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType, ThreadKind};
use super::git_ops::GitOps;
use super::id_alloc;

/// Maximum number of CAS retries on ref collision during thread creation.
const MAX_CREATE_RETRIES: usize = 5;

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
) -> ForumResult<String> {
    create_thread_with_branch(git, kind, title, body, None, actor, clock)
}

/// Create a new thread with an optional branch scope.
pub fn create_thread_with_branch(
    git: &GitOps,
    kind: ThreadKind,
    title: &str,
    body: Option<&str>,
    branch: Option<&str>,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<String> {
    create_thread_core(git, kind, title, body, branch, actor, clock, None)
}

/// Create a new thread with a timestamp override.
///
/// Like `create_thread_with_branch`, but uses the given `created_at`
/// instead of the clock. Intended for import/migration scenarios.
#[allow(clippy::too_many_arguments)]
pub fn create_thread_with_timestamp(
    git: &GitOps,
    kind: ThreadKind,
    title: &str,
    body: Option<&str>,
    branch: Option<&str>,
    actor: &str,
    clock: &dyn Clock,
    created_at: DateTime<Utc>,
) -> ForumResult<String> {
    create_thread_core(
        git,
        kind,
        title,
        body,
        branch,
        actor,
        clock,
        Some(created_at),
    )
}

#[allow(clippy::too_many_arguments)]
fn create_thread_core(
    git: &GitOps,
    kind: ThreadKind,
    title: &str,
    body: Option<&str>,
    branch: Option<&str>,
    actor: &str,
    clock: &dyn Clock,
    created_at: Option<DateTime<Utc>>,
) -> ForumResult<String> {
    if let Some(branch) = branch {
        let refname = format!("refs/heads/{branch}");
        if git.resolve_ref(&refname)?.is_none() {
            return Err(super::error::ForumError::Repo(format!(
                "branch '{branch}' does not exist in this repository"
            )));
        }
    }

    let ts = created_at.unwrap_or_else(|| clock.now());
    let timestamp_str = ts.to_rfc3339();
    let mut last_err = None;

    for _ in 0..MAX_CREATE_RETRIES {
        let thread_id = id_alloc::alloc_thread_id(kind, actor, title, &timestamp_str);
        let mut event = Event::base(&thread_id, EventType::Create, actor, clock)
            .with_title(title)
            .with_kind(kind)
            .with_branch(branch);
        if let Some(ts) = created_at {
            event = event.with_created_at(ts);
        }
        if let Some(body) = body {
            event = event.with_body(body);
        }
        match super::event::write_event(git, &event) {
            Ok(_) => return Ok(thread_id),
            Err(ForumError::Git(msg)) if msg.contains("already exists") => {
                last_err = Some(ForumError::Git(msg));
                continue;
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        ForumError::Git("thread ID collision: exhausted retries".into())
    }))
}
