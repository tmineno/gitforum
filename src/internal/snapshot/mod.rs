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
//!
//! Phase 1 of RFC `7ymtc4b2`: this subsystem is additive; production
//! commands do not call into it yet. Phase 2 cuts each command over.

pub mod link;
pub mod store;

pub use link::{Link, Links};
pub use store::{read_snapshot, write_snapshot, NodeWithBody, ThreadDocument};
