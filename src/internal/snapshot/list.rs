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

use std::collections::{HashMap, HashSet};

use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::refs::{
    thread_id_from_published_ref, thread_id_from_ref, PUBLISHED_PREFIX, THREADS_PREFIX,
};
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
    /// RFC fls856j3 publish visibility carried from `thread.toml`.
    pub visibility: super::super::thread::Visibility,
    /// `true` when the row was sourced from `refs/forum/published/<id>`
    /// rather than `refs/forum/threads/<id>`. Public-consumer clones
    /// (`git forum init --public-only`) see only published rows.
    pub from_published: bool,
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

/// Walk `refs/forum/threads/*` and `refs/forum/published/*`, returning
/// one [`ThreadRow`] per snapshot ref deduplicated by thread id. When
/// both refs exist for a given id, the authoritative ref wins
/// (RFC fls856j3 §5.1 read protocol). Refs that fail to parse as
/// snapshots (legacy event chains, missing `thread.toml`, malformed
/// TOML) are skipped.
///
/// Postcondition: the returned vec is sorted lexicographically by
/// thread id.
pub fn list_threads(git: &GitOps) -> ForumResult<Vec<ThreadRow>> {
    let auth = git.list_refs_with_shas(THREADS_PREFIX)?;
    let published = git.list_refs_with_shas(PUBLISHED_PREFIX)?;
    let mut rows: Vec<ThreadRow> = Vec::with_capacity(auth.len() + published.len());
    let mut seen: HashSet<String> = HashSet::with_capacity(auth.len());

    for (refname, sha) in &auth {
        let Some(thread_id) = thread_id_from_ref(refname) else {
            continue;
        };
        match read_snapshot_at(git, sha) {
            Ok(doc) => {
                seen.insert(thread_id.to_string());
                rows.push(row_from_doc(thread_id, &doc, false));
            }
            Err(ForumError::LegacyEventChain)
            | Err(ForumError::SnapshotMissing(_))
            | Err(ForumError::SnapshotInvalid(_))
            | Err(ForumError::SnapshotSchemaUnsupported(_)) => continue,
            Err(e) => return Err(e),
        }
    }

    for (refname, sha) in &published {
        let Some(thread_id) = thread_id_from_published_ref(refname) else {
            continue;
        };
        if seen.contains(thread_id) {
            // Authoritative wins per RFC fls856j3 §5.1.
            continue;
        }
        match read_snapshot_at(git, sha) {
            Ok(doc) => rows.push(row_from_doc(thread_id, &doc, true)),
            Err(ForumError::LegacyEventChain)
            | Err(ForumError::SnapshotMissing(_))
            | Err(ForumError::SnapshotInvalid(_))
            | Err(ForumError::SnapshotSchemaUnsupported(_)) => continue,
            Err(e) => return Err(e),
        }
    }

    rows.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rows)
}

/// Read a single thread by id. Returns `None` when neither the
/// authoritative nor the published ref exists, or when the snapshot
/// can't be parsed. Returns a hard error only for git-level failures.
pub fn read_row(git: &GitOps, thread_id: &str) -> ForumResult<Option<ThreadRow>> {
    let auth_ref = super::super::refs::thread_ref(thread_id);
    if let Some(sha) = git.resolve_ref(&auth_ref)? {
        return match read_snapshot_at(git, &sha) {
            Ok(doc) => Ok(Some(row_from_doc(thread_id, &doc, false))),
            Err(ForumError::LegacyEventChain)
            | Err(ForumError::SnapshotMissing(_))
            | Err(ForumError::SnapshotInvalid(_))
            | Err(ForumError::SnapshotSchemaUnsupported(_)) => Ok(None),
            Err(e) => Err(e),
        };
    }
    let pub_ref = super::super::refs::published_ref(thread_id);
    if let Some(sha) = git.resolve_ref(&pub_ref)? {
        return match read_snapshot_at(git, &sha) {
            Ok(doc) => Ok(Some(row_from_doc(thread_id, &doc, true))),
            Err(ForumError::LegacyEventChain)
            | Err(ForumError::SnapshotMissing(_))
            | Err(ForumError::SnapshotInvalid(_))
            | Err(ForumError::SnapshotSchemaUnsupported(_)) => Ok(None),
            Err(e) => Err(e),
        };
    }
    Ok(None)
}

/// Snapshot of `thread_id -> tip_sha` for incremental refresh.
///
/// The TUI's `auto_refresh` loop uses this to detect ref changes
/// between ticks. Walks both `refs/forum/threads/*` and
/// `refs/forum/published/*` so a published-only update on a
/// public-consumer clone still triggers a refresh.
pub fn thread_tip_shas(git: &GitOps) -> ForumResult<HashMap<String, String>> {
    let auth = git.list_refs_with_shas(THREADS_PREFIX)?;
    let published = git.list_refs_with_shas(PUBLISHED_PREFIX)?;
    let mut out: HashMap<String, String> = HashMap::with_capacity(auth.len() + published.len());
    for (refname, sha) in auth {
        if let Some(thread_id) = thread_id_from_ref(&refname) {
            out.insert(thread_id.to_string(), sha);
        }
    }
    for (refname, sha) in published {
        if let Some(thread_id) = thread_id_from_published_ref(&refname) {
            out.entry(thread_id.to_string()).or_insert(sha);
        }
    }
    Ok(out)
}

fn row_from_doc(thread_id: &str, doc: &super::ThreadDocument, from_published: bool) -> ThreadRow {
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
        visibility: snap.visibility,
        from_published,
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
