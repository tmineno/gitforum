//! `git forum comment | objection | action` orchestration.
//!
//! SPEC-3.0 §2.2 / ADR-006 keeps four canonical NodeKinds: Comment,
//! Approval, Objection, Action. The v1.x rhetorical shorthands
//! (`claim`/`question`/`summary`/`risk`/`review`) were removed at
//! Phase 2 slot 2 (RFC `7ymtc4b2`); their CLI arms, the `Commands::*`
//! enum variants, and the `warn_legacy_node_shorthand` helper are no
//! longer in tree.
//!
//! Phase 2 slot 2: `run_shorthand_say` writes a `NodeRecord` + body
//! through `internal::snapshot::store::write_snapshot`. The legacy
//! `internal::write_ops::say_node` event-write path is no longer
//! invoked here.

use std::path::PathBuf;

use chrono::Utc;

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::commands::show;
use crate::internal::error::ForumError;
use crate::internal::id_alloc;
use crate::internal::node::{NodeKind, NodeRecord, NodeStatus, NodeType};
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::snapshot::{self, store::write_snapshot, NodeWithBody};
use crate::internal::thread;

use super::revise::resolve_reply_to;
use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};
use super::thread_new::resolve_body_required;

// Note on `internal::event::NodeType`: imported only for
// `operation_check::check_say` (policy is keyed on NodeType) — the
// import does not bridge legacy event-chain reads/writes. ADR-011
// Decision 3 forbids non-migrate paths from consuming legacy event
// chains; this module honors that by bailing on `LegacyEventChain`
// instead of migrating-on-write.

/// Args for `commands::shorthand_say::run` — shared field set used by
/// `comment` / `objection` / `action` (and `node add` after slot 7f).
pub struct ShorthandSayArgs {
    pub thread_id: String,
    pub body_positional: Option<String>,
    pub body_flag: Option<String>,
    pub body_file: Option<PathBuf>,
    pub edit: bool,
    pub reply_to: Option<String>,
    pub as_actor: Option<String>,
    pub kind: NodeKind,
    pub force: bool,
}

/// Uniform entry point per task `t8o3vnt6`.
pub fn run(args: ShorthandSayArgs, ctx: &Context) -> Result<(), ForumError> {
    run_shorthand_say(
        &args.thread_id,
        args.body_positional,
        args.body_flag,
        args.body_file,
        args.edit,
        args.reply_to,
        args.as_actor,
        args.kind,
        args.force,
        ctx.clock.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_shorthand_say(
    thread_id: &str,
    body_positional: Option<String>,
    body_flag: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    reply_to: Option<String>,
    as_actor: Option<String>,
    kind: NodeKind,
    force: bool,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);
    let body = body_positional.or(body_flag);
    let body_text = resolve_body_required(
        body,
        body_file,
        edit,
        &format!("Compose a {} node", node_kind_label(kind)),
    )?;

    // Operation check is still keyed on the v2 ThreadStatus + NodeType.
    // Project the SPEC-3.0 NodeKind back into the four canonical NodeType
    // variants for policy lookup; the conversion is internal to this
    // helper so main.rs stays free of `internal::event` imports.
    let policy_node_type = node_kind_to_policy_type(kind);
    let state = thread::replay_thread(&git, thread_id)?;
    let category = crate::internal::policy::category_for_state(&state);
    let violations =
        operation_check::check_say(&policy, category, state.status.as_str(), policy_node_type);
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let resolved_reply = resolve_reply_to(&git, thread_id, reply_to.as_deref())?;
    let now = clock.now();
    let node_id = id_alloc::alloc_bare_thread_id(&actor, &body_text, &now.to_rfc3339());

    write_node_to_snapshot(
        &git,
        thread_id,
        NodeWithBody {
            record: NodeRecord {
                id: node_id.clone(),
                kind,
                status: NodeStatus::Open,
                created_at: now,
                created_by: actor.clone(),
                updated_at: None,
                updated_by: None,
                reply_to: resolved_reply.clone(),
                legacy_label: None,
            },
            body: body_text.clone(),
        },
        &actor,
        now,
    )?;

    println!(
        "Added {} {}",
        node_kind_label(kind),
        show::short_oid(&node_id)
    );
    if let Ok(state) = thread::replay_thread(&git, thread_id) {
        eprintln!(
            "{}",
            show::render_show(
                &state,
                &show::ShowOptions {
                    mode: show::ShowMode::ActionHint,
                    policy: Some(policy.clone()),
                    ..show::ShowOptions::default()
                }
            )
        );
    }
    Ok(())
}

/// Append `node` to the thread's snapshot and write a new snapshot
/// commit. ADR-011 Decision 3: non-migrate paths must NOT consume
/// legacy event chains. If the source is still on the legacy chain,
/// bail with `LegacyEventChain` so the user runs `git forum migrate`
/// before mutating.
fn write_node_to_snapshot(
    git: &crate::internal::git_ops::GitOps,
    thread_id: &str,
    node: NodeWithBody,
    actor: &str,
    now: chrono::DateTime<Utc>,
) -> Result<(), ForumError> {
    let mut doc = snapshot::read_snapshot(git, thread_id)?;
    doc.nodes.push(node);
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();
    write_snapshot(git, thread_id, &doc, "node add")?;
    Ok(())
}

/// Project a SPEC-3.0 [`NodeKind`] back to the v2 [`NodeType`]
/// vocabulary used by `policy.toml`. Phase 2 keeps the policy schema
/// keyed on NodeType; the conversion is total because the four
/// canonical NodeKind variants each map 1:1 to the four canonical
/// NodeType variants. This is the only consumer of `internal::event`
/// in this module, and it stays internal so main.rs no longer imports
/// `event::NodeType` (audit grep contract).
fn node_kind_to_policy_type(kind: NodeKind) -> NodeType {
    match kind {
        NodeKind::Comment => NodeType::Comment,
        NodeKind::Approval => NodeType::Approval,
        NodeKind::Objection => NodeType::Objection,
        NodeKind::Action => NodeType::Action,
    }
}

fn node_kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Comment => "comment",
        NodeKind::Approval => "approval",
        NodeKind::Objection => "objection",
        NodeKind::Action => "action",
    }
}
