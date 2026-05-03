//! `git forum {retract|resolve|reopen}` orchestration over a node id list.

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::event::EventType;
use crate::internal::show;
use crate::internal::thread;
use crate::internal::write_ops;

use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};

/// Args for `commands::node_bulk::run`.
pub struct NodeBulkArgs {
    pub thread_id: String,
    pub node_ids: Vec<String>,
    pub as_actor: Option<String>,
    pub event_type: EventType,
    pub label: String,
}

/// Uniform entry point per task `t8o3vnt6`.
pub fn run(args: NodeBulkArgs, ctx: &Context) -> Result<(), ForumError> {
    run_node_lifecycle_bulk(
        &args.thread_id,
        &args.node_ids,
        args.as_actor,
        args.event_type,
        &args.label,
        ctx.clock.as_ref(),
    )
}

pub fn run_node_lifecycle_bulk(
    thread_id: &str,
    node_ids: &[String],
    as_actor: Option<String>,
    event_type: EventType,
    label: &str,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let actor = resolve_actor(as_actor, &git);
    let mut failures = 0usize;
    for node_id in node_ids {
        let resolved = match thread::resolve_node_id_in_thread(&git, thread_id, node_id) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {node_id}: {e}");
                failures += 1;
                continue;
            }
        };
        match write_ops::node_lifecycle(&git, thread_id, &resolved, &actor, clock, event_type) {
            Ok(()) => println!("{label} {}", show::short_oid(&resolved)),
            Err(e) => {
                eprintln!("error: {}: {e}", show::short_oid(&resolved));
                failures += 1;
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
    if event_type == EventType::Retract {
        eprintln!("note: retract is a soft-delete — the original content remains in git history");
    }
    Ok(())
}
