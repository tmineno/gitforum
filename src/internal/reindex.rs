use super::error::{ForumError, ForumResult};
use super::git_ops::GitOps;
use super::thread::{self, ThreadState};

pub struct ReindexReport {
    pub threads_found: usize,
    pub threads_replayed: Vec<ThreadState>,
    pub errors: Vec<(String, ForumError)>,
}

/// Walk all thread refs and replay each one.
///
/// M1 skeleton: verifies replay works for all threads.
/// M5 will write results to SQLite index.
pub fn run_reindex(git: &GitOps) -> ForumResult<ReindexReport> {
    let ids = thread::list_thread_ids(git)?;
    let threads_found = ids.len();
    let mut threads_replayed = Vec::new();
    let mut errors = Vec::new();

    for id in &ids {
        match thread::replay_thread(git, id) {
            Ok(state) => threads_replayed.push(state),
            Err(e) => errors.push((id.clone(), e)),
        }
    }

    Ok(ReindexReport {
        threads_found,
        threads_replayed,
        errors,
    })
}
