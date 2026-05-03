//! `git forum node show <NODE_ID>` and `git forum node add` orchestration.
//!
//! Phase 2 slot 7f (RFC `7ymtc4b2`): NEW module. The two `Node::*` arms
//! relocate from `main.rs` to [`run_show`] and [`run_add`] here.
//! `Node::Add` continues to share its body with the typed shorthands
//! (`comment` / `objection` / `action`) via [`super::shorthand_say`];
//! the only difference at the CLI surface is that `node add` accepts
//! an explicit `--type`.

use std::path::PathBuf;

use super::super::clock::Clock;
use super::super::error::ForumError;
use super::super::event::NodeType;
use super::super::thread;
use super::context::Context;
use super::shorthand_say::run_shorthand_say;
use super::show::{render_node_show, ShowOptions};

/// Args for [`run_show`] — `git forum node show <NODE_ID>`.
pub struct NodeShowArgs {
    pub node_id: String,
}

/// Args for [`run_add`] — `git forum node add <THREAD> --type <TYPE> ...`.
pub struct NodeAddArgs {
    pub thread_id: String,
    pub node_type: NodeType,
    pub body_positional: Option<String>,
    pub body_flag: Option<String>,
    pub body_file: Option<PathBuf>,
    pub edit: bool,
    pub reply_to: Option<String>,
    pub as_actor: Option<String>,
    pub force: bool,
}

/// Render `git forum node show <NODE_ID>` for a single node.
///
/// `find_node` does the snapshot tree lookup; `render_node_show` is
/// the shared canonical formatter.
pub fn run_show(args: NodeShowArgs, ctx: &Context) -> Result<(), ForumError> {
    let lookup = thread::find_node(&ctx.git, &args.node_id)?;
    print!("{}", render_node_show(&lookup, &ShowOptions::default()));
    Ok(())
}

/// Add a typed node to a thread. Forwards to `run_shorthand_say`
/// with the explicit `--type` from the CLI; the `force` and edit
/// flags pass through unchanged.
pub fn run_add(args: NodeAddArgs, clock: &dyn Clock) -> Result<(), ForumError> {
    // `run_shorthand_say` performs its own discover/init-warning, so
    // we don't construct a Context here; forwarding via Context would
    // mean double-discovery against the same repo.
    run_shorthand_say(
        &args.thread_id,
        args.body_positional,
        args.body_flag,
        args.body_file,
        args.edit,
        args.reply_to,
        args.as_actor,
        args.node_type,
        args.force,
        clock,
    )
}
