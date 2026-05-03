//! `git forum branch {bind|clear}` orchestration.
//!
//! Phase 2 slot 6 (RFC `7ymtc4b2`): writes the `branch` field on
//! `thread.toml` directly via `snapshot::store::write_snapshot` per
//! SPEC-3.0 §3.1. The legacy `Scope` event-write path is no longer
//! invoked here.

use super::super::clock::Clock;
use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::snapshot::{self, store::write_snapshot};

use super::shorthand_say::migrate_legacy_to_snapshot;

/// Bind or clear a thread's branch scope.
///
/// Preconditions: thread_id exists; when `branch` is Some, the branch
/// exists in `refs/heads/`.
/// Postconditions: `thread.toml.branch` is updated to the new value
/// (or removed when clearing).
/// Failure modes: `ForumError::Repo` when the branch does not exist;
/// `ForumError::Git` on subprocess failure.
/// Side effects: writes one snapshot commit and updates the thread ref.
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

    let mut doc = match snapshot::read_snapshot(git, thread_id) {
        Ok(doc) => doc,
        Err(ForumError::LegacyEventChain) => migrate_legacy_to_snapshot(git, thread_id)?,
        Err(other) => return Err(other),
    };
    let now = clock.now();
    doc.snapshot.branch = branch.map(String::from);
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();

    let msg = match branch {
        Some(b) => format!("branch bind {thread_id} -> {b}"),
        None => format!("branch clear {thread_id}"),
    };
    write_snapshot(git, thread_id, &doc, &msg)?;
    Ok(())
}
