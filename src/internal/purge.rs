//! Purge (hard-delete) event content from git-forum history.
//!
//! Rewrites commit chains to replace purged event bodies and actor IDs
//! with `[purged]`. This is destructive: commit SHAs change and all
//! clones must re-fetch affected refs.
//!
//! Preconditions: git-forum is initialized; target thread/event/actor exists.
//! Postconditions: affected commits rewritten; ref updated; original objects
//!   remain in `.git/objects/` until `git gc` prunes them.
//! Failure modes: ForumError::Git on plumbing failure.
//! Side effects: rewrites git commits and refs.

use std::collections::HashMap;

use super::error::{ForumError, ForumResult};
use super::event::{self, Event};
use super::git_ops::GitOps;
use super::refs;
use super::thread;

const PURGED: &str = "[purged]";

/// Summary of a purge operation.
pub struct PurgeReport {
    /// Number of events whose content was purged.
    pub events_purged: usize,
    /// Number of commits rewritten (includes descendants of purged events).
    pub commits_rewritten: usize,
    /// Thread IDs that were affected.
    pub threads_affected: Vec<String>,
}

/// Plan for a purge (dry-run).
pub struct PurgePlan {
    /// Events that would be purged.
    pub events: Vec<PurgePlanEntry>,
}

pub struct PurgePlanEntry {
    pub thread_id: String,
    pub event_sha: String,
    pub event_type: String,
    pub actor: String,
    pub has_body: bool,
}

/// Purge a single event's content from a thread.
///
/// Replaces the event's `body` and `title` with `[purged]`.
/// Rewrites the commit chain from the target event to the tip.
pub fn purge_event(git: &GitOps, thread_id: &str, target_sha: &str) -> ForumResult<PurgeReport> {
    let shas = git.rev_list(&refs::thread_ref(thread_id))?;
    if !shas.contains(&target_sha.to_string()) {
        return Err(ForumError::Repo(format!(
            "event {target_sha} not found in thread {thread_id}"
        )));
    }
    rewrite_chain(git, thread_id, &shas, |sha, ev| {
        if sha == target_sha {
            purge_event_content(ev);
            true
        } else {
            false
        }
    })
}

/// Plan a single-event purge (dry-run).
pub fn plan_purge_event(git: &GitOps, thread_id: &str, target_sha: &str) -> ForumResult<PurgePlan> {
    let shas = git.rev_list(&refs::thread_ref(thread_id))?;
    if !shas.contains(&target_sha.to_string()) {
        return Err(ForumError::Repo(format!(
            "event {target_sha} not found in thread {thread_id}"
        )));
    }
    let ev = event::read_event(git, target_sha)?;
    Ok(PurgePlan {
        events: vec![PurgePlanEntry {
            thread_id: thread_id.to_string(),
            event_sha: target_sha.to_string(),
            event_type: ev.event_type.to_string(),
            actor: ev.actor.clone(),
            has_body: ev.body.is_some(),
        }],
    })
}

/// Purge all events by a specific actor across all threads.
///
/// Replaces the actor ID, body, and title with `[purged]` on every
/// event where `event.actor == actor_id`.
pub fn purge_actor(git: &GitOps, actor_id: &str) -> ForumResult<PurgeReport> {
    let thread_ids = thread::list_thread_ids(git)?;
    let mut total = PurgeReport {
        events_purged: 0,
        commits_rewritten: 0,
        threads_affected: vec![],
    };
    for tid in &thread_ids {
        let shas = git.rev_list(&refs::thread_ref(tid))?;
        // Check if any event in this thread matches the actor
        let has_match = shas.iter().any(|sha| {
            event::read_event(git, sha)
                .map(|ev| ev.actor == actor_id)
                .unwrap_or(false)
        });
        if !has_match {
            continue;
        }
        let actor_owned = actor_id.to_string();
        let report = rewrite_chain(git, tid, &shas, |_sha, ev| {
            if ev.actor == actor_owned {
                ev.actor = PURGED.to_string();
                purge_event_content(ev);
                true
            } else {
                false
            }
        })?;
        if report.events_purged > 0 {
            total.events_purged += report.events_purged;
            total.commits_rewritten += report.commits_rewritten;
            total.threads_affected.push(tid.clone());
        }
    }
    Ok(total)
}

/// Plan an actor purge (dry-run).
pub fn plan_purge_actor(git: &GitOps, actor_id: &str) -> ForumResult<PurgePlan> {
    let thread_ids = thread::list_thread_ids(git)?;
    let mut entries = vec![];
    for tid in &thread_ids {
        let events = event::load_thread_events(git, tid)?;
        for ev in &events {
            if ev.actor == actor_id {
                entries.push(PurgePlanEntry {
                    thread_id: tid.clone(),
                    event_sha: ev.event_id.clone(),
                    event_type: ev.event_type.to_string(),
                    actor: ev.actor.clone(),
                    has_body: ev.body.is_some(),
                });
            }
        }
    }
    Ok(PurgePlan { events: entries })
}

/// Replace body and title content with `[purged]`.
fn purge_event_content(ev: &mut Event) {
    if ev.body.is_some() {
        ev.body = Some(PURGED.to_string());
    }
    if ev.title.is_some() {
        ev.title = Some(PURGED.to_string());
    }
}

/// Rewrite a thread's commit chain, applying a mutator to each event.
///
/// The mutator receives `(commit_sha, &mut Event)` and returns `true`
/// if the event was modified. All descendants of modified commits are
/// also rewritten (with new parent SHAs).
///
/// `shas` must be newest-first (as returned by `rev_list`).
fn rewrite_chain(
    git: &GitOps,
    thread_id: &str,
    shas: &[String],
    mut mutator: impl FnMut(&str, &mut Event) -> bool,
) -> ForumResult<PurgeReport> {
    // Work oldest-first
    let mut ordered: Vec<&str> = shas.iter().map(|s| s.as_str()).collect();
    ordered.reverse();

    // Build parent map: for each commit, find its parent (the previous commit
    // in chronological order). The first commit is the root (no parent).
    // Since this is a linear chain (event-sourced), each commit has at most
    // one parent which is the previous commit in the ordered list.
    let mut parent_of: HashMap<&str, &str> = HashMap::new();
    for i in 1..ordered.len() {
        parent_of.insert(ordered[i], ordered[i - 1]);
    }

    // Map old SHA → new SHA for rewritten commits
    let mut remap: HashMap<&str, String> = HashMap::new();
    let mut events_purged = 0;
    let mut commits_rewritten = 0;

    for &old_sha in &ordered {
        let mut ev = event::read_event(git, old_sha)?;
        let was_mutated = mutator(old_sha, &mut ev);

        // Check if parent was rewritten
        let parent_rewritten = parent_of.get(old_sha).and_then(|p| remap.get(p)).cloned();

        if !was_mutated && parent_rewritten.is_none() {
            // No change needed — keep original commit
            continue;
        }

        if was_mutated {
            events_purged += 1;
        }

        // Create new blob → tree → commit
        let json = serde_json::to_string_pretty(&ev)?;
        let blob_sha = git.hash_object(json.as_bytes())?;
        let tree_sha = git.mktree_single("event.json", &blob_sha)?;

        let message = format!("[git-forum] {} {}", ev.event_type, ev.thread_id);

        // Resolve parent: use remapped parent if available, else original parent
        let parents: Vec<&str> = if let Some(ref new_parent) = parent_rewritten {
            vec![new_parent.as_str()]
        } else if let Some(&orig_parent) = parent_of.get(old_sha) {
            vec![orig_parent]
        } else {
            // Root commit — no parent
            vec![]
        };

        let new_sha = git.commit_tree(&tree_sha, &parents, &message)?;
        remap.insert(old_sha, new_sha);
        commits_rewritten += 1;
    }

    if commits_rewritten == 0 {
        return Ok(PurgeReport {
            events_purged: 0,
            commits_rewritten: 0,
            threads_affected: vec![],
        });
    }

    // Update the ref to point to the new head (the newest commit)
    let old_head = ordered.last().expect("non-empty chain");
    let new_head = remap.get(old_head).map(|s| s.as_str()).unwrap_or(old_head);
    let ref_name = refs::thread_ref(thread_id);
    git.update_ref(&ref_name, new_head)?;

    Ok(PurgeReport {
        events_purged,
        commits_rewritten,
        threads_affected: vec![thread_id.to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purge_event_content_replaces_body_and_title() {
        let mut ev = Event {
            event_id: "test".into(),
            thread_id: "ISSUE-0001".into(),
            event_type: super::super::event::EventType::Say,
            created_at: chrono::Utc::now(),
            actor: "human/alice".into(),
            base_rev: None,
            parents: vec![],
            title: Some("Secret title".into()),
            kind: None,
            body: Some("Secret body".into()),
            node_type: None,
            target_node_id: None,
            new_state: None,
            approvals: vec![],
            evidence: None,
            link_rel: None,
            branch: None,
            incorporated_node_ids: vec![],
            reply_to: None,
        };
        purge_event_content(&mut ev);
        assert_eq!(ev.body.as_deref(), Some("[purged]"));
        assert_eq!(ev.title.as_deref(), Some("[purged]"));
        assert_eq!(ev.actor, "human/alice"); // actor not changed by content purge
    }

    #[test]
    fn purge_event_content_leaves_none_fields_as_none() {
        let mut ev = Event {
            event_id: "test".into(),
            thread_id: "ISSUE-0001".into(),
            event_type: super::super::event::EventType::State,
            created_at: chrono::Utc::now(),
            actor: "human/alice".into(),
            base_rev: None,
            parents: vec![],
            title: None,
            kind: None,
            body: None,
            node_type: None,
            target_node_id: None,
            new_state: Some("closed".into()),
            approvals: vec![],
            evidence: None,
            link_rel: None,
            branch: None,
            incorporated_node_ids: vec![],
            reply_to: None,
        };
        purge_event_content(&mut ev);
        assert!(ev.body.is_none());
        assert!(ev.title.is_none());
    }
}
