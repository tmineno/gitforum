//! Module integration tests for `src/internal/id_alloc.rs`
//! (test-policy.md category 1).

mod support;

use git_forum::internal::id_alloc;
use git_forum::internal::legacy::event::ThreadKind;

#[test]
fn alloc_issue_id_has_correct_prefix() {
    let id = id_alloc::alloc_thread_id(
        ThreadKind::Issue,
        "human/alice",
        "Bug",
        "2026-01-01T00:00:00Z",
    );
    assert!(id.starts_with("ASK-"), "expected ASK- prefix, got: {id}");
    let token = &id[4..];
    assert_eq!(
        token.len(),
        8,
        "token length should be 8, got: {}",
        token.len()
    );
    assert!(
        token
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
        "token has invalid chars: {token}"
    );
}

#[test]
fn alloc_ids_are_unique_with_different_nonces() {
    let id1 = id_alloc::alloc_thread_id(
        ThreadKind::Rfc,
        "human/alice",
        "Title",
        "2026-01-01T00:00:00Z",
    );
    let id2 = id_alloc::alloc_thread_id(
        ThreadKind::Rfc,
        "human/alice",
        "Title",
        "2026-01-01T00:00:00Z",
    );
    assert_ne!(id1, id2, "two allocations with random nonces should differ");
}

#[test]
fn alloc_deterministic_with_nonce() {
    let id1 = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc,
        "human/alice",
        "Test",
        "2026-01-01",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    let id2 = id_alloc::alloc_thread_id_with_nonce(
        ThreadKind::Rfc,
        "human/alice",
        "Test",
        "2026-01-01",
        &[1, 2, 3, 4, 5, 6, 7, 8],
    );
    assert_eq!(id1, id2, "same inputs + nonce should produce same ID");
}

#[test]
fn is_valid_thread_id_both_formats() {
    assert!(id_alloc::is_valid_thread_id("RFC-0001"));
    assert!(id_alloc::is_valid_thread_id("ASK-0042"));
    assert!(id_alloc::is_valid_thread_id("RFC-a7f3b2x1"));
    assert!(id_alloc::is_valid_thread_id("JOB-x8n2q1d4"));
    assert!(!id_alloc::is_valid_thread_id("UNKNOWN-0001"));
    assert!(!id_alloc::is_valid_thread_id("garbage"));
}
