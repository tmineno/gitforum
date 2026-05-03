//! `git forum status <THREAD_ID>` orchestration.
//!
//! Phase 2 slot 7e (RFC `7ymtc4b2`): NEW module. The arm body in
//! `main.rs` becomes a thin dispatcher that hands `StatusArgs` to
//! [`run`] here. Internally this is a thin wrapper over
//! `commands::show::render_show(_, ShowMode::Status)` — the cutover
//! gives `status` its own orchestration entry-point so the module
//! layout matches the audit's per-arm 1:1 file map.

use super::super::error::ForumError;
use super::super::thread;
use super::context::Context;
use super::shared::resolve_tid;
use super::show::{render_show, ShowMode, ShowOptions};

/// Args for [`run`] — `git forum status`.
pub struct StatusArgs {
    pub thread_id: String,
}

/// Uniform entry point for the `status` subcommand.
pub fn run(args: StatusArgs, ctx: &Context) -> Result<(), ForumError> {
    let thread_id = resolve_tid(&ctx.git, &args.thread_id)?;
    let state = thread::replay_thread(&ctx.git, &thread_id)?;
    print!(
        "{}",
        render_show(
            &state,
            &ShowOptions {
                mode: ShowMode::Status,
                ..ShowOptions::default()
            }
        )
    );
    Ok(())
}
