pub mod actor;
pub mod clock;
pub mod commands;
pub mod config;
pub mod create;
pub mod editor;
pub mod error;
// Phase 4 Step 2b (RFC `7ymtc4b2`, task `913c4s9v`): `event` relocated
// to `internal::legacy::event` per ADR-011 Decision 1. Importers
// should reach for the new path; non-migrate code is blocked by
// `tests/legacy_gate_test.rs`.
pub mod evidence;
pub mod git_ops;
pub mod github;
pub mod github_export;
pub mod github_import;
pub mod help;
pub mod id;
pub mod id_alloc;
pub mod index;
pub mod init;
pub mod legacy;
pub mod lint_emit;
pub mod node;
pub mod operation_check;
pub mod policy;
pub mod prune;
pub mod purge;
pub mod refs;
pub mod reindex;
pub mod repair;
pub mod repair_workflow;
pub mod snapshot;
pub mod state_change;
pub mod thread;
pub mod timeline;
pub mod tui;
pub mod validate;
// Phase 4 Step 2b: `workflow` relocated to `internal::legacy::workflow`
// alongside `event`.
pub mod write_ops;
