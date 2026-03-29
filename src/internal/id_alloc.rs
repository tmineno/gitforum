use super::error::ForumResult;
use super::event::ThreadKind;
use super::git_ops::GitOps;
use super::refs;

/// Allocate the next human-readable thread ID for a given kind.
///
/// Scans `refs/forum/threads/<PREFIX>-*` to find the current max
/// sequence number, then returns `<PREFIX>-NNNN` where NNNN = max + 1.
///
/// Preconditions: git is bound to a valid repo.
/// Postconditions: returned ID is unique among existing thread refs.
/// Failure modes: ForumError::Git on subprocess failure.
/// Side effects: none (read-only).
pub fn alloc_thread_id(git: &GitOps, kind: ThreadKind) -> ForumResult<String> {
    let prefix = kind.id_prefix();
    let all_refs = git.list_refs(refs::THREADS_PREFIX)?;
    let max = all_refs
        .iter()
        .filter_map(|r| {
            let id = r.strip_prefix(refs::THREADS_PREFIX)?;
            let (pfx, num) = id.split_once('-')?;
            // Accept both current and legacy prefixes (e.g. ASK and ISSUE)
            if ThreadKind::from_id_prefix(pfx) == Some(kind) {
                num.parse::<u32>().ok()
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0);
    Ok(format!("{prefix}-{:04}", max + 1))
}

#[cfg(test)]
mod tests {
    #[test]
    fn alloc_id_format() {
        // Offline: just verify the format string logic
        let formatted = format!("{}-{:04}", "RFC", 1u32);
        assert_eq!(formatted, "RFC-0001");
        let formatted2 = format!("{}-{:04}", "ASK", 42u32);
        assert_eq!(formatted2, "ASK-0042");
        let formatted3 = format!("{}-{:04}", "JOB", 1u32);
        assert_eq!(formatted3, "JOB-0001");
    }
}
