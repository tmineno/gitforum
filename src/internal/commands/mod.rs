//! CLI orchestration entry points.
//!
//! Each submodule owns the orchestration glue for one cluster of related
//! `Commands::*` arms. `main.rs` keeps clap parsing + dispatch only;
//! everything else (replay, policy load, write events, render output) lives
//! here. Per #yjelk0s0 (P1 main.rs function extraction) and the
//! Phase 0 relocation of peer-file CLI handlers (task `9tof5nre`).

pub mod branch;
pub mod brief;
pub mod bulk;
pub mod context;
pub mod diff;
pub mod doctor;
pub mod hook;
pub mod ls;
pub mod migrate;
pub mod node_bulk;
pub mod repair_workflow;
pub mod revise;
pub mod shared;
pub mod shorthand_say;
pub mod show;
pub mod state;
pub mod thread_new;
pub mod verify;

pub use context::Context;
