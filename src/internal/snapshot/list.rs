//! Snapshot-derived thread listing for the TUI and `git forum ls`.
//!
//! task `913c4s9v`: replaces the
//! SQLite-backed `internal::index::list_threads` + `internal::reindex`
//! refresh loop with on-demand `for-each-ref` + `snapshot::store::
//! read_snapshot`. Per RFC Exceptions, no index is reintroduced for
//! v3.0.0; the snapshot-walk latency is accepted as a v3.1 concern
//! if it bites in practice (the listing scales linearly with thread
//! count and reads one tree per ref).
//!
//! Legacy event-chain refs are skipped silently — non-migrate reads
//! reject `LegacyEventChain` (SPEC-3.0 §1.2.4); a v2 ref doesn't
//! appear in the listing until the user runs `git forum migrate`.

use std::collections::HashMap;

use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::refs::{thread_id_from_ref, THREADS_PREFIX};
use super::store::read_snapshot_at;

/// A snapshot-derived thread row, mirroring the field surface that
/// the TUI and `commands::ls` consumed from the legacy
/// `internal::index::ThreadRow`. The v2 index-only fields
/// (`open_objections`, `open_actions`, `has_summary`, `tip_sha`)
/// are deliberately omitted — TUI production code never read them.
#[derive(Debug, Clone)]
pub struct ThreadRow {
    pub id: String,
    /// SPEC-3.0 category (RFC §3.1). Carried forward as `kind` for
    /// surface compatibility with the v2 `ThreadRow` consumers.
    pub kind: String,
    /// v2-shaped lifecycle, derived from category for display
    /// compatibility (see [`category_lifecycle`]).
    pub lifecycle: String,
    /// Always `true` for 3.0 snapshots — category is a required field
    /// (SPEC-3.0 §4.2), so the v2 "implicit lifecycle from kind"
    /// fallback never applies.
    pub lifecycle_explicit: bool,
    pub tags: Vec<String>,
    pub status: String,
    pub title: String,
    pub body: Option<String>,
    pub branch: Option<String>,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
}

/// Map a SPEC-3.0 category to a v2-compatible lifecycle string for
/// TUI display. Mirrors the kind-keyed fallback in
/// `tui::render::row_lifecycle`.
pub fn category_lifecycle(category: &str) -> &'static str {
    match category {
        "rfc" => "proposal",
        "dec" => "record",
        // task / issue / bug / unknown all collapse to execution per
        // the SPEC-2.0 §3.1.1 lifecycle defaults.
        _ => "execution",
    }
}

/// Walk `refs/forum/threads/*` and return one [`ThreadRow`] per
/// 3.0 snapshot ref. Refs that fail to parse as snapshots (legacy
/// event chains, missing `thread.toml`, malformed TOML) are skipped.
///
/// Postcondition: the returned vec is sorted lexicographically by
/// thread id, matching the order `for-each-ref` returns refs.
pub fn list_threads(git: &GitOps) -> ForumResult<Vec<ThreadRow>> {
    let refs = git.list_refs_with_shas(THREADS_PREFIX)?;
    let mut rows = Vec::with_capacity(refs.len());
    for (refname, sha) in &refs {
        let Some(thread_id) = thread_id_from_ref(refname) else {
            continue;
        };
        match read_snapshot_at(git, sha) {
            Ok(doc) => rows.push(row_from_doc(thread_id, &doc)),
            Err(ForumError::LegacyEventChain)
            | Err(ForumError::SnapshotMissing(_))
            | Err(ForumError::SnapshotInvalid(_))
            | Err(ForumError::SnapshotSchemaUnsupported(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(rows)
}

/// Read a single thread by id. Returns `None` when the ref doesn't
/// exist or the snapshot can't be parsed; returns a hard error only
/// for git-level failures.
pub fn read_row(git: &GitOps, thread_id: &str) -> ForumResult<Option<ThreadRow>> {
    let refname = super::super::refs::thread_ref(thread_id);
    let Some(sha) = git.resolve_ref(&refname)? else {
        return Ok(None);
    };
    match read_snapshot_at(git, &sha) {
        Ok(doc) => Ok(Some(row_from_doc(thread_id, &doc))),
        Err(ForumError::LegacyEventChain)
        | Err(ForumError::SnapshotMissing(_))
        | Err(ForumError::SnapshotInvalid(_))
        | Err(ForumError::SnapshotSchemaUnsupported(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Snapshot of `thread_id -> tip_sha` for incremental refresh.
///
/// Replaces `internal::index::thread_tip_shas`. The TUI's
/// `auto_refresh` loop uses this to detect ref changes between ticks
/// and re-walk only when something moved.
pub fn thread_tip_shas(git: &GitOps) -> ForumResult<HashMap<String, String>> {
    let refs = git.list_refs_with_shas(THREADS_PREFIX)?;
    let mut out = HashMap::with_capacity(refs.len());
    for (refname, sha) in refs {
        if let Some(thread_id) = thread_id_from_ref(&refname) {
            out.insert(thread_id.to_string(), sha);
        }
    }
    Ok(out)
}

fn row_from_doc(thread_id: &str, doc: &super::ThreadDocument) -> ThreadRow {
    let snap = &doc.snapshot;
    ThreadRow {
        id: thread_id.to_string(),
        kind: snap.category.clone(),
        lifecycle: category_lifecycle(&snap.category).to_string(),
        lifecycle_explicit: true,
        tags: snap.tags.clone(),
        status: snap.status.clone(),
        title: snap.title.clone(),
        body: doc.body.clone(),
        branch: snap.branch.clone(),
        created_at: snap.created_at.to_rfc3339(),
        created_by: snap.created_by.clone(),
        updated_at: snap.updated_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_lifecycle_mirrors_v2_defaults() {
        assert_eq!(category_lifecycle("rfc"), "proposal");
        assert_eq!(category_lifecycle("dec"), "record");
        assert_eq!(category_lifecycle("task"), "execution");
        assert_eq!(category_lifecycle("issue"), "execution");
        assert_eq!(category_lifecycle("bug"), "execution");
        assert_eq!(category_lifecycle("unknown"), "execution");
    }
}
