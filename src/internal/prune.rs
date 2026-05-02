//! Discover and remove orphan thread refs.
//!
//! An *orphan* ref is one whose oldest commit is not a parseable
//! `event.json` — typically a manually-created ref under
//! `refs/forum/threads/`, or a ref whose history was rewritten away from a
//! valid create event. There is no recoverable thread behind such a ref.
//!
//! Detection delegates to [`event::is_orphan_ref`]. This module only adds
//! batch enumeration over every thread ref and the actual ref deletion.

use super::error::ForumResult;
use super::event;
use super::git_ops::GitOps;
use super::refs;
use super::thread;

/// One orphan ref found by [`scan`].
pub struct OrphanRef {
    pub thread_id: String,
    pub ref_name: String,
}

/// Walk every thread ref and return those whose oldest commit lacks a
/// parseable `event.json`.
///
/// Refs that simply fail mid-chain (data damage on a real thread) are NOT
/// returned — those need manual investigation, not a prune.
pub fn scan(git: &GitOps) -> ForumResult<Vec<OrphanRef>> {
    let ids = thread::list_thread_ids(git)?;
    let mut orphans = Vec::new();
    for id in ids {
        if event::is_orphan_ref(git, &id)? {
            orphans.push(OrphanRef {
                ref_name: refs::thread_ref(&id),
                thread_id: id,
            });
        }
    }
    Ok(orphans)
}

/// Delete every orphan ref found by [`scan`]. Returns the deleted refs.
pub fn delete(git: &GitOps, orphans: &[OrphanRef]) -> ForumResult<()> {
    for orphan in orphans {
        git.delete_ref(&orphan.ref_name)?;
    }
    Ok(())
}
