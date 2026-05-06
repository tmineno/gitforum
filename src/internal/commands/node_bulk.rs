//! `git forum {retract|resolve|reopen}` orchestration over a node id list.
//!
//! task `1hg98odf`: node lifecycle changes update
//! `nodes/<id>.toml` `status` directly via
//! `snapshot::store::write_snapshot`. The legacy
//! `internal::write_ops::node_lifecycle` event-write path (which
//! emitted `EventType::Retract|Resolve|Reopen`) is no longer
//! invoked here; the `EventType` arg has been replaced by a typed
//! [`NodeLifecycleOp`] tag.

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::commands::show;
use crate::internal::error::ForumError;
use crate::internal::node::NodeStatus;
use crate::internal::snapshot::{self, store::write_snapshot};
use crate::internal::thread;

use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};

/// Lifecycle update applied to a list of nodes by `git forum
/// {retract|resolve|reopen}`. SPEC-3.0 §2.2: each maps to a
/// `NodeStatus` value (or, for `Reopen`, clears resolved/retracted/
/// incorporated → `Open`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLifecycleOp {
    Retract,
    Resolve,
    Reopen,
}

impl NodeLifecycleOp {
    fn target_status(self) -> NodeStatus {
        match self {
            NodeLifecycleOp::Retract => NodeStatus::Retracted,
            NodeLifecycleOp::Resolve => NodeStatus::Resolved,
            NodeLifecycleOp::Reopen => NodeStatus::Open,
        }
    }
}

/// Args for `commands::node_bulk::run`.
pub struct NodeBulkArgs {
    pub thread_id: String,
    pub node_ids: Vec<String>,
    pub as_actor: Option<String>,
    pub op: NodeLifecycleOp,
    pub label: String,
}

/// Uniform entry point per task `t8o3vnt6`.
pub fn run(args: NodeBulkArgs, ctx: &Context) -> Result<(), ForumError> {
    run_node_lifecycle_bulk(
        &args.thread_id,
        &args.node_ids,
        args.as_actor,
        args.op,
        &args.label,
        ctx.clock.as_ref(),
    )
}

pub fn run_node_lifecycle_bulk(
    thread_id: &str,
    node_ids: &[String],
    as_actor: Option<String>,
    op: NodeLifecycleOp,
    label: &str,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let actor = resolve_actor(as_actor, &git);

    let mut doc = snapshot::read_snapshot(&git, thread_id)?;

    let now = clock.now();
    let target = op.target_status();
    let mut failures = 0usize;
    let mut applied: Vec<String> = Vec::new();

    for node_id in node_ids {
        // Resolve via `replay_thread` so the existing prefix / collision
        // semantics carry through. `replay_thread` is mixed-chain aware.
        let resolved = match thread::resolve_node_id_in_thread(&git, thread_id, node_id) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {node_id}: {e}");
                failures += 1;
                continue;
            }
        };
        if let Some(node) = doc.nodes.iter_mut().find(|n| n.record.id == resolved) {
            node.record.status = target;
            node.record.updated_at = Some(now);
            node.record.updated_by = Some(actor.clone());
            applied.push(resolved);
        } else {
            eprintln!(
                "error: {}: node not found in snapshot",
                show::short_oid(&resolved)
            );
            failures += 1;
        }
    }

    if !applied.is_empty() {
        doc.snapshot.updated_at = now;
        doc.snapshot.updated_by = actor.clone();
        write_snapshot(
            &git,
            thread_id,
            &doc,
            &format!(
                "node lifecycle: {} ({})",
                label.to_lowercase(),
                applied.len()
            ),
        )?;
        for resolved in &applied {
            println!("{label} {}", show::short_oid(resolved));
        }
    }

    if failures > 0 {
        std::process::exit(1);
    }
    if matches!(op, NodeLifecycleOp::Retract) {
        eprintln!("note: retract is a soft-delete — the original content remains in git history");
    }
    Ok(())
}
