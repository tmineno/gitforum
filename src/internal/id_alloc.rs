use sha2::{Digest, Sha256};

use super::event::ThreadKind;
use super::id::rand_bytes;

/// Generate an opaque content-addressed thread ID.
///
/// Format: `KIND-XXXXXXXX` where X is base36 `[a-z0-9]`.
///
/// Preconditions: none (pure function).
/// Postconditions: returns a valid thread ID string.
/// Failure modes: none.
/// Side effects: reads system entropy for nonce.
pub fn alloc_thread_id(kind: ThreadKind, actor: &str, title: &str, timestamp: &str) -> String {
    alloc_thread_id_with_nonce(kind, actor, title, timestamp, &rand_bytes::<8>())
}

/// Generate an opaque thread ID with a specific nonce (for deterministic testing).
pub fn alloc_thread_id_with_nonce(
    kind: ThreadKind,
    actor: &str,
    title: &str,
    timestamp: &str,
    nonce: &[u8],
) -> String {
    let prefix = kind.id_prefix();
    let token = compute_token(actor, title, timestamp, nonce);
    format!("{prefix}-{token}")
}

/// Compute the 8-char base36 token from inputs.
fn compute_token(actor: &str, title: &str, timestamp: &str, nonce: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(actor.as_bytes());
    hasher.update(timestamp.as_bytes());
    hasher.update(title.as_bytes());
    hasher.update(nonce);
    let hash = hasher.finalize();
    base36_encode(&hash, 8)
}

/// Encode bytes as base36 `[0-9a-z]`, returning `len` characters.
fn base36_encode(bytes: &[u8], len: usize) -> String {
    const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut result = String::with_capacity(len);
    for &b in bytes.iter().take(len) {
        result.push(ALPHABET[(b as usize) % 36] as char);
    }
    result
}

/// Check whether a thread ID uses the new opaque format: KIND-[a-z0-9]{8}.
pub fn is_opaque_id(id: &str) -> bool {
    let Some((prefix, token)) = id.split_once('-') else {
        return false;
    };
    ThreadKind::from_id_prefix(prefix).is_some()
        && token.len() == 8
        && token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && !token.chars().all(|c| c.is_ascii_digit())
}

/// Check whether a thread ID uses the legacy sequential format: KIND-NNNN.
pub fn is_sequential_id(id: &str) -> bool {
    let Some((prefix, num)) = id.split_once('-') else {
        return false;
    };
    ThreadKind::from_id_prefix(prefix).is_some()
        && num.len() == 4
        && num.chars().all(|c| c.is_ascii_digit())
}

/// Check whether a string is a valid thread ID (either format).
pub fn is_valid_thread_id(id: &str) -> bool {
    is_opaque_id(id) || is_sequential_id(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_id_format_opaque() {
        let id = alloc_thread_id_with_nonce(
            ThreadKind::Rfc,
            "human/alice",
            "Test RFC",
            "2026-01-01T00:00:00Z",
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        assert!(id.starts_with("RFC-"), "got: {id}");
        let token = &id[4..];
        assert_eq!(token.len(), 8, "token length: {}", token.len());
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "token contains invalid chars: {token}"
        );
    }

    #[test]
    fn different_nonces_produce_different_ids() {
        let id1 = alloc_thread_id_with_nonce(
            ThreadKind::Issue,
            "human/alice",
            "Same title",
            "2026-01-01T00:00:00Z",
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        let id2 = alloc_thread_id_with_nonce(
            ThreadKind::Issue,
            "human/alice",
            "Same title",
            "2026-01-01T00:00:00Z",
            &[9, 10, 11, 12, 13, 14, 15, 16],
        );
        assert_ne!(id1, id2);
    }

    #[test]
    fn nonce_randomness_produces_unique_ids() {
        let id1 = alloc_thread_id(
            ThreadKind::Rfc,
            "human/alice",
            "Title",
            "2026-01-01T00:00:00Z",
        );
        let id2 = alloc_thread_id(
            ThreadKind::Rfc,
            "human/alice",
            "Title",
            "2026-01-01T00:00:00Z",
        );
        assert_ne!(id1, id2);
    }

    #[test]
    fn is_opaque_id_valid() {
        assert!(is_opaque_id("RFC-a7f3b2x1"));
        assert!(is_opaque_id("ASK-0a1b2c3d"));
        assert!(is_opaque_id("JOB-x8n2q1d4"));
    }

    #[test]
    fn is_opaque_id_rejects_sequential() {
        assert!(!is_opaque_id("RFC-0001"));
        assert!(!is_opaque_id("ASK-0042"));
    }

    #[test]
    fn is_opaque_id_rejects_invalid() {
        assert!(!is_opaque_id("RFC-short"));
        assert!(!is_opaque_id("RFC-TOOLONG9"));
        assert!(!is_opaque_id("UNKNOWN-a7f3b2x1"));
        assert!(!is_opaque_id("no-dash-prefix"));
    }

    #[test]
    fn is_sequential_id_valid() {
        assert!(is_sequential_id("RFC-0001"));
        assert!(is_sequential_id("ASK-0042"));
        assert!(is_sequential_id("ISSUE-0001"));
        assert!(is_sequential_id("TASK-0001"));
    }

    #[test]
    fn is_sequential_id_rejects_opaque() {
        assert!(!is_sequential_id("RFC-a7f3b2x1"));
    }

    #[test]
    fn is_valid_thread_id_both_formats() {
        assert!(is_valid_thread_id("RFC-0001"));
        assert!(is_valid_thread_id("RFC-a7f3b2x1"));
        assert!(!is_valid_thread_id("UNKNOWN-0001"));
        assert!(!is_valid_thread_id("garbage"));
    }

    #[test]
    fn alloc_id_format_legacy() {
        // Legacy format test preserved for reference
        let formatted = format!("{}-{:04}", "RFC", 1u32);
        assert_eq!(formatted, "RFC-0001");
    }

    #[test]
    fn base36_encode_deterministic() {
        let input = [0u8, 1, 35, 36, 37, 255, 100, 200];
        let result = base36_encode(&input, 8);
        assert_eq!(result.len(), 8);
        assert!(result
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }
}
