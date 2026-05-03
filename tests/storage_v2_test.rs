//! v2.x storage-shape regression tests (task `4w8hm98j`).
//!
//! These tests assert the **event-chain** storage layout at
//! `refs/forum/threads/<id>` produced by the v2 implementation. They
//! are intentionally version-pinned: the matching Phase 2 cutover
//! commit per command removes the corresponding `v2_*` test and adds
//! its `tests/storage_v3_test.rs` counterpart that asserts the
//! snapshot-tree shape.
//!
//! See `doc/internal/cli-coverage-audit.md` for the cutover discipline
//! and `doc/internal/main-rs-audit.md` for per-command Phase 2 slots.
//!
//! Slot 1 (`thread_new`) was the only originally-locked entry; it was
//! removed at Phase 2 slot 1 (RFC `7ymtc4b2`). The matching v3
//! invariant is in
//! `tests/storage_v3_test.rs::v3_cli_thread_new_writes_thread_toml`.
//! Subsequent slots may add new `v2_*` entries here as their own
//! pre-cutover bridges.
