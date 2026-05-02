//! Discover and remove forum-storage debris that doctor identifies.
//!
//! Two kinds of debris live here, both surfaced by `git forum doctor`:
//!
//! 1. **Orphan refs** — a ref under `refs/forum/threads/` whose oldest
//!    commit is not a parseable `event.json`. There is no recoverable
//!    thread behind it (manual ref creation, or history rewritten away
//!    from a valid create event). Cleanup is a plain ref deletion;
//!    handled by [`scan`] / [`delete`].
//! 2. **Stale-target events** — an `edit` / `retract` / `resolve` /
//!    `reopen` / `retype` / `revise-body` event whose `target_node_id`
//!    references a node that does not exist in the thread's chain.
//!    Surfaced by [`thread::replay_strict`] as
//!    [`StrictReplayIssue::UnknownTargetNode`]. Cleanup rewrites the
//!    affected chain to drop the stale event; handled by
//!    [`scan_stale_events`] / [`apply_stale_event_plans`].
//!
//! The two flows share the same vocabulary (`scan` builds the plan,
//! `apply` / `delete` mutates Git) and surface as separate CLI commands
//! (`prune-orphans`, `prune-stale-events`).

use std::collections::HashSet;

use super::error::ForumResult;
use super::event;
use super::git_ops::GitOps;
use super::refs;
use super::thread;
use super::validate::StrictReplayIssue;

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

// ---- Stale-target events (Phase 1.5: Finding 4 cleanup) ----

/// One thread's worth of stale-target events to drop.
pub struct StaleEventPlan {
    pub thread_id: String,
    /// Distinct event SHAs that emitted at least one
    /// [`StrictReplayIssue::UnknownTargetNode`].
    pub events_to_drop: Vec<String>,
    /// Total orphan target references aggregated across all dropped events
    /// (one event can list multiple `incorporated_node_ids`).
    pub orphan_target_count: usize,
}

/// Walk every thread, run strict replay, collect events whose
/// `target_node_id` (or `incorporated_node_ids`) reference vanished nodes.
///
/// Returns one [`StaleEventPlan`] per affected thread; threads with zero
/// stale events are omitted.
pub fn scan_stale_events(git: &GitOps) -> ForumResult<Vec<StaleEventPlan>> {
    let ids = thread::list_thread_ids(git)?;
    let mut plans = Vec::new();
    for id in ids {
        // A thread that fails to load (orphan ref, mid-chain corruption)
        // is not in scope here — `prune-orphans` covers ref-level damage.
        let Ok((_state, issues)) = thread::replay_thread_strict(git, &id) else {
            continue;
        };
        let mut shas = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut target_count = 0;
        for issue in &issues {
            if let StrictReplayIssue::UnknownTargetNode { event_id, .. } = issue {
                target_count += 1;
                if seen.insert(event_id.clone()) {
                    shas.push(event_id.clone());
                }
            }
        }
        if !shas.is_empty() {
            plans.push(StaleEventPlan {
                thread_id: id,
                events_to_drop: shas,
                orphan_target_count: target_count,
            });
        }
    }
    Ok(plans)
}

/// Apply every plan in `plans`, rewriting affected thread refs.
///
/// Returns the total number of events dropped across all threads. Threads
/// whose chain becomes structurally invalid (e.g., the create event itself
/// is in the drop set, which strict replay never produces) are skipped with
/// no ref change.
pub fn apply_stale_event_plans(git: &GitOps, plans: &[StaleEventPlan]) -> ForumResult<usize> {
    let mut total = 0;
    for plan in plans {
        total += drop_events_from_thread(git, &plan.thread_id, &plan.events_to_drop)?;
    }
    Ok(total)
}

/// Rewrite `thread_id`'s chain dropping commits whose SHA appears in
/// `drop_shas`. Events before the first dropped commit keep their original
/// SHA; events after are re-emitted with new parents (and therefore new
/// SHAs).
///
/// Returns the number of events actually dropped from this thread. Returns
/// 0 (and leaves the ref untouched) when no SHA in `drop_shas` is present in
/// the chain, when the chain is empty, or when the create event is in the
/// drop set — strict replay never proposes that, but defensively we refuse
/// to write a thread with no create.
fn drop_events_from_thread(
    git: &GitOps,
    thread_id: &str,
    drop_shas: &[String],
) -> ForumResult<usize> {
    let drop_set: HashSet<&str> = drop_shas.iter().map(String::as_str).collect();
    let chain = git.rev_list(&refs::thread_ref(thread_id))?; // newest first
    if chain.is_empty() {
        return Ok(0);
    }
    let mut ordered: Vec<&str> = chain.iter().map(String::as_str).collect();
    ordered.reverse(); // chronological

    // Refuse to drop the create event — that would yield an unparseable
    // chain. Strict replay's UnknownTargetNode never targets a Create
    // (Create has no target_node_id), so this is purely defensive.
    if drop_set.contains(ordered[0]) {
        return Ok(0);
    }

    let mut current_parent: Option<String> = None;
    let mut new_head: Option<String> = None;
    let mut chain_modified = false;
    let mut dropped = 0usize;

    for &sha in &ordered {
        if drop_set.contains(sha) {
            dropped += 1;
            chain_modified = true;
            continue;
        }
        if !chain_modified {
            // Unchanged prefix: keep the original commit SHA as parent for
            // whatever comes next.
            current_parent = Some(sha.to_string());
            new_head = Some(sha.to_string());
            continue;
        }
        // Re-emit this event with the rewritten parent.
        let ev = event::read_event(git, sha)?;
        let json = serde_json::to_string_pretty(&ev)?;
        let blob = git.hash_object(json.as_bytes())?;
        let tree = git.mktree_single("event.json", &blob)?;
        let parents: Vec<&str> = match &current_parent {
            Some(p) => vec![p.as_str()],
            None => vec![],
        };
        let msg = format!("[git-forum] {} {}", ev.event_type, ev.thread_id);
        let new_sha = git.commit_tree(&tree, &parents, &msg)?;
        current_parent = Some(new_sha.clone());
        new_head = Some(new_sha);
    }

    if chain_modified {
        if let Some(head) = new_head {
            git.update_ref(&refs::thread_ref(thread_id), &head)?;
        }
    }
    Ok(dropped)
}
