pub mod actor;
pub mod clock;
pub mod commands;
pub mod config;
pub mod editor;
pub mod error;
pub mod evidence;
pub mod git_ops;
pub mod help;
pub mod id;
pub mod id_alloc;
pub mod init;
pub mod legacy;
pub mod lint_emit;
pub mod node;
pub mod operation_check;
pub mod policy;
pub mod refs;
pub mod snapshot;
pub mod thread;
pub mod tui;
pub mod validate;

// Phase 4 (RFC `7ymtc4b2`, task `913c4s9v`):
// - `event`, `workflow` relocated into `internal::legacy/` in Step 2b
//   (ADR-011 Decision 1; non-migrate access blocked by
//   `tests/legacy_gate_test.rs`).
// - `state_change`, `write_ops`, `create`, `repair`, `repair_workflow`,
//   `prune`, `purge`, `timeline`, `index`, `reindex`, `github`,
//   `github_import`, `github_export`, and `commands::repair_workflow`
//   were `git rm`'d in Step 3 per the DELETE list in
//   `doc/internal/3.0-removal-plan.md`.
