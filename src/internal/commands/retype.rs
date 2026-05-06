//! `git forum retype <ID> <NODE_ID> <NEW_TYPE>` orchestration.
//!
//! task `1hg98odf`: rewrites `nodes/<id>.toml`'s
//! `type` field directly via `snapshot::store::write_snapshot`. The
//! legacy `internal::write_ops::retype_node` event-write path is no
//! longer invoked here.

use crate::internal::clock::Clock;
use crate::internal::commands::show;
use crate::internal::error::ForumError;
use crate::internal::node::NodeKind;
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::snapshot::{self, store::write_snapshot};
use crate::internal::thread;

use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};

#[allow(clippy::too_many_arguments)]
pub fn run_retype(
    thread_id: &str,
    node_ref: &str,
    new_type: &str,
    as_actor: Option<String>,
    force: bool,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = resolve_tid(&git, thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);

    let new_kind = parse_new_kind(new_type)?;

    let state = thread::replay_thread(&git, &thread_id)?;
    let category = crate::internal::policy::category_for_state(&state);
    let violations = operation_check::check_revise(&policy, category, state.status.as_str(), false);
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, node_ref)?;
    let mut doc = snapshot::read_snapshot(&git, &thread_id)?;
    let now = clock.now();
    let node = doc
        .nodes
        .iter_mut()
        .find(|n| n.record.id == resolved)
        .ok_or_else(|| {
            ForumError::Repo(format!(
                "node '{resolved}' not found in snapshot for thread '{thread_id}'"
            ))
        })?;
    node.record.kind = new_kind;
    node.record.updated_at = Some(now);
    node.record.updated_by = Some(actor.clone());
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.clone();

    write_snapshot(
        &git,
        &thread_id,
        &doc,
        &format!("retype node {resolved} -> {new_type}"),
    )?;
    println!("Retyped {} -> {new_type}", show::short_oid(&resolved));
    Ok(())
}

/// Parse a CLI `--type` value into a SPEC-3.0 [`NodeKind`]. The
/// legacy rhetorical labels (`claim`/`question`/`summary`/`risk`/
/// `review`/`alternative`/`assumption`/`evidence`) are rejected with
/// a redirect to the canonical four kinds — task `1hg98odf`
/// already removed the inbound CLI surfaces for them.
fn parse_new_kind(raw: &str) -> Result<NodeKind, ForumError> {
    match raw {
        "comment" => Ok(NodeKind::Comment),
        "approval" => Ok(NodeKind::Approval),
        "objection" => Ok(NodeKind::Objection),
        "action" => Ok(NodeKind::Action),
        other => Err(ForumError::Config(format!(
            "node type '{other}' is not a SPEC-3.0 NodeKind \
             (canonical: comment, approval, objection, action)"
        ))),
    }
}
