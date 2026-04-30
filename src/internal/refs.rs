//! Ref namespace constants and helpers for git-forum.
//!
//! All authoritative data is stored under `refs/forum/`.
//!
//! 2.0 storage form is `refs/forum/threads/<bare-token>` (SPEC-2.0 §5.1, §6.2).
//! Legacy 1.x forms (`refs/forum/threads/RFC-…`, `refs/forum/threads/RFC-NNNN`)
//! remain readable; the helpers in this module are agnostic to which form the
//! caller passes — both fit `refs/forum/threads/<id>` so no branching is needed
//! at the ref layer.

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
    fn thread_ref_accepts_bare_token() {
        // SPEC-2.0 §5.1 / §6.2: 2.0 stores threads under their bare token.
        assert_eq!(thread_ref("a7f3b2x1"), "refs/forum/threads/a7f3b2x1");
    }

    #[test]
    fn extract_thread_id() {
        assert_eq!(
            thread_id_from_ref("refs/forum/threads/RFC-0001"),
            Some("RFC-0001")
        );
        assert_eq!(thread_id_from_ref("refs/heads/main"), None);
    }

    #[test]
    fn extract_bare_thread_id_round_trips() {
        let id = "a7f3b2x1";
        assert_eq!(thread_id_from_ref(&thread_ref(id)), Some(id));
    }
}
