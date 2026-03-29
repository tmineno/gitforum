//! Detect and repair thread ID conflicts with a remote.
//!
//! When two users independently create threads with colliding IDs,
//! the second push is rejected. This module detects such conflicts
//! (using `git ls-remote` to avoid overwriting local refs) and
//! re-allocates the local thread ID by rewriting its commit chain.
//!
//! Preconditions: git-forum is initialized; remote is configured.
//! Postconditions: conflicting local thread IDs are re-allocated;
//!   old refs deleted, new refs created.
//! Failure modes: ForumError::Git on plumbing or network failure.
//! Side effects: rewrites git commits and refs for conflicting threads.

use super::error::{ForumError, ForumResult};
use super::event::{self, EventType};
use super::git_ops::GitOps;
use super::id_alloc;
use super::refs;

/// A detected thread ID conflict between local and remote.
pub struct ConflictInfo {
    pub thread_id: String,
    pub local_sha: String,
    pub remote_sha: String,
}

/// Summary of a repair operation.
pub struct RepairReport {
    /// Pairs of (old_id, new_id) for re-allocated threads.
    pub reallocated: Vec<(String, String)>,
    /// Non-fatal errors encountered during repair.
    pub errors: Vec<String>,
}

/// Detect thread ID conflicts between local refs and a remote.
///
/// A conflict exists when both local and remote have a ref for the same
/// thread ID, but the local commit is NOT an ancestor of the remote commit
/// (i.e., they diverged rather than one being ahead of the other).
pub fn detect_conflicts(git: &GitOps, remote: &str) -> ForumResult<Vec<ConflictInfo>> {
    let remote_refs = git
        .ls_remote(remote, "refs/forum/threads/*")
        .map_err(|e| {
            ForumError::Git(format!(
                "could not query remote '{remote}': {e}\n  hint: check that the remote is reachable"
            ))
        })?;

    let local_refs = git.list_refs_with_shas(refs::THREADS_PREFIX)?;

    // Build lookup: thread_id -> local_sha
    let mut local_map = std::collections::HashMap::new();
    for (refname, sha) in &local_refs {
        if let Some(tid) = refs::thread_id_from_ref(refname) {
            local_map.insert(tid.to_string(), sha.clone());
        }
    }

    let mut conflicts = Vec::new();
    for (refname, remote_sha) in &remote_refs {
        if let Some(tid) = refs::thread_id_from_ref(refname) {
            if let Some(local_sha) = local_map.get(tid) {
                if local_sha == remote_sha {
                    continue; // Same commit, no conflict
                }
                // Check if local is an ancestor of remote (local is just behind — not a conflict)
                if git.is_ancestor(local_sha, remote_sha)? {
                    continue;
                }
                // Check if remote is an ancestor of local (local is ahead — not a conflict)
                if git.is_ancestor(remote_sha, local_sha)? {
                    continue;
                }
                conflicts.push(ConflictInfo {
                    thread_id: tid.to_string(),
                    local_sha: local_sha.clone(),
                    remote_sha: remote_sha.clone(),
                });
            }
        }
    }

    Ok(conflicts)
}

/// Re-allocate a thread's ID by rewriting its entire commit chain.
///
/// Reads the Create event to determine the thread kind, then generates
/// a new opaque ID and rewrites all events with the new thread_id.
/// Creates a new ref and deletes the old one.
///
/// Returns `(old_id, new_id)`.
pub fn reallocate_thread(git: &GitOps, old_thread_id: &str) -> ForumResult<(String, String)> {
    let old_ref = refs::thread_ref(old_thread_id);
    let shas = git.rev_list(&old_ref)?;
    if shas.is_empty() {
        return Err(ForumError::Repo(format!(
            "thread '{old_thread_id}' has no events"
        )));
    }

    // Work oldest-first (rev_list returns newest-first)
    let mut ordered: Vec<&str> = shas.iter().map(|s| s.as_str()).collect();
    ordered.reverse();

    // Read the Create event to determine kind, actor, title for ID generation
    let create_event = event::read_event(git, ordered[0])?;
    if create_event.event_type != EventType::Create {
        return Err(ForumError::Repo(format!(
            "first event of thread '{old_thread_id}' is not a Create event"
        )));
    }
    let kind = create_event.kind.ok_or_else(|| {
        ForumError::Repo(format!(
            "Create event for '{old_thread_id}' is missing thread kind"
        ))
    })?;
    let title = create_event.title.as_deref().unwrap_or("");
    let timestamp = create_event.created_at.to_rfc3339();

    let new_thread_id = id_alloc::alloc_thread_id(kind, &create_event.actor, title, &timestamp);

    // Rewrite all commits with the new thread_id
    let new_head = rewrite_chain_with_new_id(git, &ordered, &new_thread_id)?;

    // Create new ref first (safe: if delete fails, we have both refs rather than none)
    let new_ref = refs::thread_ref(&new_thread_id);
    git.create_ref(&new_ref, &new_head)?;
    git.delete_ref(&old_ref)?;

    Ok((old_thread_id.to_string(), new_thread_id))
}

/// Rewrite a commit chain, replacing thread_id in every event.
fn rewrite_chain_with_new_id(
    git: &GitOps,
    ordered_shas: &[&str],
    new_thread_id: &str,
) -> ForumResult<String> {
    let mut prev_new_sha: Option<String> = None;

    for &old_sha in ordered_shas {
        let mut ev = event::read_event(git, old_sha)?;
        ev.thread_id = new_thread_id.to_string();

        let json = serde_json::to_string_pretty(&ev)?;
        let blob_sha = git.hash_object(json.as_bytes())?;
        let tree_sha = git.mktree_single("event.json", &blob_sha)?;
        let message = format!("[git-forum] {} {}", ev.event_type, new_thread_id);

        let parents: Vec<&str> = prev_new_sha.iter().map(|s| s.as_str()).collect();
        let new_sha = git.commit_tree(&tree_sha, &parents, &message)?;
        prev_new_sha = Some(new_sha);
    }

    prev_new_sha.ok_or_else(|| ForumError::Repo("empty commit chain".into()))
}

/// Detect and fix all ID conflicts with a remote.
pub fn repair_conflicts(
    git: &GitOps,
    remote: &str,
    dry_run: bool,
) -> ForumResult<RepairReport> {
    let conflicts = detect_conflicts(git, remote)?;
    let mut report = RepairReport {
        reallocated: Vec::new(),
        errors: Vec::new(),
    };

    if dry_run || conflicts.is_empty() {
        for c in &conflicts {
            report
                .reallocated
                .push((c.thread_id.clone(), String::new()));
        }
        return Ok(report);
    }

    for conflict in &conflicts {
        match reallocate_thread(git, &conflict.thread_id) {
            Ok((old_id, new_id)) => {
                report.reallocated.push((old_id, new_id));
            }
            Err(e) => {
                report
                    .errors
                    .push(format!("{}: {e}", conflict.thread_id));
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::clock::Clock;
    use crate::internal::create;
    use crate::internal::event::{NodeType, ThreadKind};
    use crate::internal::git_ops::GitOps;
    use crate::internal::write_ops;

    use chrono::{TimeZone, Utc};
    use std::process::Command;
    use tempfile::TempDir;

    struct FixedClock;
    impl Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
        }
    }

    fn init_test_repo() -> (TempDir, GitOps) {
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(path)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .output()
                .expect("git failed");
        };

        run(&["init"]);
        run(&["config", "user.name", "Test"]);
        run(&["config", "user.email", "test@test.com"]);

        let git = GitOps::new(path.to_path_buf());
        (dir, git)
    }

    #[test]
    fn reallocate_single_event_thread() {
        let (_dir, git) = init_test_repo();
        let clock = FixedClock;

        let old_id = create::create_thread(
            &git,
            ThreadKind::Issue,
            "Test issue",
            Some("body text"),
            "human/alice",
            &clock,
        )
        .unwrap();

        let (returned_old, new_id) = reallocate_thread(&git, &old_id).unwrap();
        assert_eq!(returned_old, old_id);
        assert_ne!(old_id, new_id);
        assert!(new_id.starts_with("ASK-"));

        // Old ref should be gone
        let old_ref = refs::thread_ref(&old_id);
        assert!(git.resolve_ref(&old_ref).unwrap().is_none());

        // New ref should exist and replay correctly
        let state = crate::internal::thread::replay_thread(&git, &new_id).unwrap();
        assert_eq!(state.title, "Test issue");
        assert_eq!(state.body.as_deref(), Some("body text"));
        assert_eq!(state.id, new_id);
    }

    #[test]
    fn reallocate_multi_event_thread() {
        let (_dir, git) = init_test_repo();
        let clock = FixedClock;

        let old_id = create::create_thread(
            &git,
            ThreadKind::Rfc,
            "Test RFC",
            Some("initial body"),
            "human/bob",
            &clock,
        )
        .unwrap();

        // Add a discussion node
        write_ops::say_node(
            &git,
            &old_id,
            NodeType::Claim,
            "This is a claim",
            "human/bob",
            &clock,
            None,
        )
        .unwrap();

        let (_, new_id) = reallocate_thread(&git, &old_id).unwrap();
        assert_ne!(old_id, new_id);
        assert!(new_id.starts_with("RFC-"));

        // Verify thread replays with all events
        let state = crate::internal::thread::replay_thread(&git, &new_id).unwrap();
        assert_eq!(state.title, "Test RFC");
        assert_eq!(state.events.len(), 2);
        assert_eq!(state.nodes.len(), 1);
        assert_eq!(state.nodes[0].body, "This is a claim");
    }
}
