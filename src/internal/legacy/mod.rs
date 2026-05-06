//! 1.x → 2.0 compatibility codec.
//!
//! Per RFC 915yuegd P1: every read-time / load-time rule that exists
//! solely to keep the v1.x storage shape projecting cleanly onto the
//! 2.0 domain model lives here. Domain code (`event.rs`, `thread.rs`,
//! `policy.rs`, `main.rs`) calls into [`v1`] rather than embedding the
//! legacy rules inline.
//!
//! The five candidates currently in scope:
//!   1. 1.x state alias normalisation ([`v1::normalize_state_name`],
//!      [`v1::migrate_legacy_state`]).
//!   2. 1.x `ThreadKind` → lifecycle auto-derive used by replay when
//!      a thread has no `facet_set` event ([`v1::lifecycle_for_legacy_kind`]).
//!   3. 1.x `policy.toml` shape rewrites — kind-keyed creation rules,
//!      kind-prefixed guard scopes, and the removed
//!      `at_least_one_summary` predicate ([`v1::rewrite_legacy_policy`]).
//!   4. 1.x `NodeType` → 2.0 canonical projection
//!      ([`v1::canonical_node_type`], [`v1::legacy_subtype_label`]).
//!   5. `Event.legacy_subtype` preservation rule consumed by both the
//!      migration tool and native 2.0 write paths
//!      ([`v1::legacy_subtype_for_node_type`]).
//!
//! Out of scope per the parent RFC: `lifecycle_explicit` is a 2.0-native
//! invariant (SPEC-2.0 §7.3 first-lifecycle-wins) and is set by domain
//! replay, not by this module.

pub mod v1;

// task `913c4s9v`: the v2 event +
// workflow modules relocated into this tree per task `913c4s9v`
// ("v1/v2-format-aware Rust code that survives v3.0.0 lives under
// src/internal/legacy/"). Non-migrate callers MUST NOT reach here
// — `tests/legacy_gate_test.rs` enforces the import-graph invariant.
pub mod event;
pub mod workflow;

// task `1v400j3l`: the v1/v2
// event-chain replay machinery (formerly in `internal::thread`)
// relocated here. The 3.0-native `thread::replay_thread` reads
// snapshot tips only; the legacy reader lives here and is reachable
// solely from `commands::migrate` (the only ALLOW-listed
// non-legacy consumer). See `tests/legacy_gate_test.rs`.
pub mod chain_replay;
