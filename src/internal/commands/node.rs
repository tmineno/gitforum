//! `git forum node show <NODE_ID>` and `git forum node add` orchestration.
//!
//! task `1hg98odf`: NEW module. The two `Node::*` arms
//! relocate from `main.rs` to [`run_show`] and [`run_add`] here.
//! `Node::Add` continues to share its body with the typed shorthands
//! (`comment` / `objection` / `action`) via [`super::shorthand_say`];
//! the only difference at the CLI surface is that `node add` accepts
//! an explicit `--type`.

use std::path::PathBuf;
use std::str::FromStr;

use super::super::clock::Clock;
use super::super::error::ForumError;
use super::super::node::NodeKind;
use super::super::refs::thread_ref;
use super::super::snapshot::history;
use super::super::thread;
use super::context::Context;
use super::shorthand_say::run_shorthand_say;
use super::show::{render_node_show, ShowOptions};

/// Args for [`run_show`] — `git forum node show <NODE_ID>`.
pub struct NodeShowArgs {
    pub node_id: String,
}

/// Args for [`run_add`] — `git forum node add <THREAD> --type <TYPE> ...`.
///
/// `node_type` is the raw CLI string (e.g. `"comment"`, `"objection"`).
/// Parsing into a [`NodeKind`] happens inside [`run_add`] so main.rs
/// stays free of `internal::event` / `internal::node` imports.
pub struct NodeAddArgs {
    pub thread_id: String,
    pub node_type: String,
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
/// Bug `bvdk2w48`: resolves the node id through the cheap
/// [`thread::NodeIdIndex`] cached on `Context` (one
/// `ls-tree --name-only nodes/` per thread, no body reads) and
/// then replays only the owning thread, instead of the historical
/// "replay every thread twice" path. `render_node_show` is the
/// shared canonical formatter.
pub fn run_show(args: NodeShowArgs, ctx: &Context) -> Result<(), ForumError> {
    let lookup = thread::find_node_with_index(&ctx.git, &args.node_id, ctx.node_index()?)?;
    // SPEC-3.0 §5.4: per-node history is the slice of the thread's
    // snapshot ref log that touched `nodes/<id>.{toml,md}`. The
    // renderer does the path filtering — we just hand it the full log.
    let timeline_entries = history::read_log(&ctx.git, &thread_ref(&lookup.thread_id)).ok();
    print!(
        "{}",
        render_node_show(
            &lookup,
            &ShowOptions {
                timeline_entries,
                ..ShowOptions::default()
            }
        )
    );
    Ok(())
}

/// Add a typed node to a thread. Forwards to `run_shorthand_say`
/// with the explicit `--type` from the CLI; the `force` and edit
/// flags pass through unchanged.
pub fn run_add(args: NodeAddArgs, clock: &dyn Clock) -> Result<(), ForumError> {
    // Parse `--type` directly into the SPEC-3.0 NodeKind. Legacy 1.x
    // rhetorical types (claim/question/summary/etc.) are not valid for
    // 3.0 native writes — the user gets a typed error pointing at the
    // canonical four. `run_shorthand_say` performs its own
    // discover/init-warning, so we don't construct a Context here.
    let kind = NodeKind::from_str(&args.node_type)
        .map_err(|e| ForumError::Config(format!("invalid --type: {e}")))?;
    run_shorthand_say(
        &args.thread_id,
        args.body_positional,
        args.body_flag,
        args.body_file,
        args.edit,
        args.reply_to,
        args.as_actor,
        kind,
        args.force,
        clock,
    )
}
