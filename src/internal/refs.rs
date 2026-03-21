//! Ref namespace constants and helpers for git-forum.
//!
//! All authoritative data is stored under `refs/forum/`.

pub const THREADS_PREFIX: &str = "refs/forum/threads/";
pub const ACTORS_PREFIX: &str = "refs/forum/actors/";
pub fn thread_ref(thread_id: &str) -> String {
    format!("{THREADS_PREFIX}{thread_id}")
}

pub fn actor_ref(actor_id: &str) -> String {
    format!("{ACTORS_PREFIX}{actor_id}")
}

/// Extract thread ID from a full ref name.
pub fn thread_id_from_ref(refname: &str) -> Option<&str> {
    refname.strip_prefix(THREADS_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_ref_format() {
        assert_eq!(thread_ref("RFC-0001"), "refs/forum/threads/RFC-0001");
    }

    #[test]
    fn extract_thread_id() {
        assert_eq!(
            thread_id_from_ref("refs/forum/threads/RFC-0001"),
            Some("RFC-0001")
        );
        assert_eq!(thread_id_from_ref("refs/heads/main"), None);
    }
}
