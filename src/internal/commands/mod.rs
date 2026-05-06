//! CLI orchestration entry points.
//!
//! Each submodule owns the orchestration glue for one cluster of related
//! `Commands::*` arms. `main.rs` keeps clap parsing + dispatch only;
//! everything else (replay, policy load, write events, render output) lives
//! here. Per task `yjelk0s0` (main.rs function extraction) and the
//! peer-file CLI handler relocation tracked by task `9tof5nre`.

pub mod branch;
pub mod brief;
pub mod bulk;
pub mod context;
pub mod diff;
pub mod doctor;
pub mod evidence;
pub mod help;
pub mod hook;
pub mod init;
pub mod link;
pub mod ls;
pub mod migrate;
pub mod node;
pub mod node_bulk;
pub mod policy;
pub mod push;
// task `913c4s9v`:
// `commands::repair_workflow` git rm'd alongside the v2 event-runtime.
pub mod retype;
pub mod revise;
pub mod shared;
pub mod shorthand_say;
pub mod shortlog;
pub mod show;
pub mod state;
pub mod status;
pub mod thread_new;
pub mod verify;
pub mod visibility;

pub use context::Context;
