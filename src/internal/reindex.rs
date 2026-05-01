use std::path::Path;

use super::error::{ForumError, ForumResult};
use super::git_ops::GitOps;
use super::index;
use super::thread::{self, ThreadState};

pub struct ReindexReport {
    pub threads_found: usize,
    pub threads_replayed: Vec<ThreadState>,
    pub errors: Vec<(String, ForumError)>,
}

/// Walk all thread refs, replay each one, and write results to the SQLite index.
///
/// Preconditions: `db_path` parent directory is writable.
/// Postconditions: index contains current state of all threads; errors are reported, not fatal.
/// Failure modes: ForumError::Git if ref listing fails; individual replay errors are collected.
/// Side effects: creates or truncates the SQLite index at `db_path`.
pub fn run_reindex(git: &GitOps, db_path: &Path) -> ForumResult<ReindexReport> {
    let conn = index::open_db(db_path)?;
    index::clear_all(&conn)?;

    let ids = thread::list_thread_ids(git)?;
    let threads_found = ids.len();
    let mut threads_replayed = Vec::new();
    let mut errors = Vec::new();

    // Pass 1: upsert all thread rows (and per-thread nodes/evidence). This
    // populates the threads table so that pass 2 can canonicalize link
    // targets against the full set of known thread IDs (Track G reverse-link
    // index needs the parent to be upserted before children's links can be
    // resolved).
    for id in &ids {
        match thread::replay_thread(git, id) {
            Ok(state) => {
                if let Err(e) = index::upsert_thread(&conn, &state)
                    .and_then(|_| index::replace_nodes_for_thread(&conn, &state))
                    .and_then(|_| index::replace_evidence_for_thread(&conn, &state))
                {
                    errors.push((id.clone(), e));
                } else {
                    threads_replayed.push(state);
                }
            }
            Err(e) => errors.push((id.clone(), e)),
        }
    }

    // Pass 2: write link rows. Done after pass 1 so canonicalization in
    // `replace_links_for_thread` sees every thread that has been upserted.
    for state in &threads_replayed {
        if let Err(e) = index::replace_links_for_thread(&conn, state) {
            errors.push((state.id.clone(), e));
        }
    }

    Ok(ReindexReport {
        threads_found,
        threads_replayed,
        errors,
    })
}
