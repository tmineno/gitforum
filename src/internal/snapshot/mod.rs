//! SPEC-3.0 snapshot subsystem.
//!
//! The snapshot tree at each `refs/forum/threads/<id>` tip carries:
//!
//! ```text
//! thread.toml
//! body.md
//! nodes/
//!   <node-id>.toml
//!   <node-id>.md
//! links.toml
//! evidence.toml
//! legacy/
//!   events.ndjson    # migration archive — 3.0 reads MUST ignore it
//! ```
//!
//! `thread.toml` is required; the others MAY be absent when empty
//! (SPEC-3.0 §4.2).
//!
//! Modules:
//! - [`link`] — `links.toml` model + serde.
//! - [`store`] — read tip → `ThreadDocument`; write `ThreadDocument`
//!   → tree → commit → CAS. Owns the SPEC-3.0 §4 schema boundary.
//! - [`history`] — git-history view of the snapshot ref per
//!   SPEC-3.0 §5.4. Replaces the v2 domain-event timeline
//!   (RFC `7ymtc4b2`, task `913c4s9v`).
//! - [`list`] — `for-each-ref` walk + per-ref snapshot read. Replaces
//!   the SQLite-backed thread listing for the TUI and `commands::ls`
//!   (task `913c4s9v`).
//!
//! RFC `7ymtc4b2`, task `qa8u71j9`: this subsystem is additive;
//! production commands do not call into it yet. task `1hg98odf` cuts
//! each command over.

pub mod history;
pub mod link;
pub mod list;
pub mod store;

pub use link::{Link, Links};
pub use store::{
    read_snapshot, read_snapshot_at, write_snapshot, write_snapshot_with_archive,
    write_snapshot_with_archive_pinned, NodeWithBody, ThreadDocument,
};
