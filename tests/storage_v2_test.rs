//! v2.x storage-shape regression tests (task `4w8hm98j`).
//!
//! These tests assert the **event-chain** storage layout at
//! `refs/forum/threads/<id>` produced by the v2 implementation. They
//! are intentionally version-pinned: the matching task `1hg98odf` cutover
//! commit per command removes the corresponding `v2_*` test and adds
//! its `tests/storage_v3_test.rs` counterpart that asserts the
//! snapshot-tree shape.
//!
//! See `doc/internal/cli-coverage-audit.md` and
//! `doc/internal/main-rs-audit.md` for the task `1hg98odf`
//! cutover map.
//!
//! Slot 1 (`thread_new`) was the only originally-locked entry; it was
//! removed at task `1hg98odf`. The matching v3
//! invariant is in
//! `tests/storage_v3_test.rs::v3_cli_thread_new_writes_thread_toml`.
//! Subsequent slots may add new `v2_*` entries here as their own
//! pre-cutover bridges.
