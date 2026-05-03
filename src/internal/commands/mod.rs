//! CLI orchestration entry points.
//!
//! Each submodule owns the orchestration glue for one cluster of related
//! `Commands::*` arms. `main.rs` keeps clap parsing + dispatch only;
//! everything else (replay, policy load, write events, render output) lives
//! here. Per #yjelk0s0 (P1 main.rs function extraction).
//!
//! No new types or vocabulary are introduced — this module is mechanical
//! relocation of functions that already exist; signatures are preserved.

pub mod bulk;
pub mod context;
pub mod node_bulk;
pub mod repair_workflow;
pub mod revise;
pub mod shared;
pub mod shorthand_say;
pub mod state;
pub mod thread_new;

pub use context::Context;
