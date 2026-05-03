//! `git forum revise` orchestration — body and node revisions.
//!
//! Owns the clap subcommand enum `ReviseCmd` (moved from `main.rs` by
//! task `t8o3vnt6`) so the command's full surface — args, dispatch,
//! orchestration — lives in one module.
//!
//! Phase 2 slot 5 (RFC `7ymtc4b2`): body and node revisions overwrite
//! `body.md` / `nodes/<id>.md` directly via
//! `snapshot::store::write_snapshot`. SPEC-3.0 §4.2 has no
//! `body_revision_count` field — revision history is `git log` over
//! the snapshot ref. The legacy `internal::write_ops::revise_*`
//! event-write paths are no longer invoked here.

use std::path::PathBuf;

use clap::Subcommand;

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::commands::show;
use crate::internal::error::ForumError;
use crate::internal::node::NodeStatus;
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::snapshot::{self, store::write_snapshot};
use crate::internal::thread;

use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};
use super::shorthand_say::migrate_legacy_to_snapshot;
use super::thread_new::resolve_body_required;

/// Args for the top-level `revise` arm — captures the optional thread_id +
/// shared body fields that apply when no explicit `body` / `node`
/// sub-command is given (default-shorthand path), plus the optional
/// `cmd` for explicit `revise body` / `revise node` dispatch.
pub struct ReviseArgs {
    pub thread_id: Option<String>,
    pub body: Option<String>,
    pub body_file: Option<PathBuf>,
    pub edit: bool,
    pub incorporates: Vec<String>,
    pub as_actor: Option<String>,
    pub force: bool,
    pub cmd: Option<ReviseCmd>,
}

/// Uniform entry point per task `t8o3vnt6` — dispatches to the body /
/// node revise impls based on the parsed sub-command.
pub fn run(args: ReviseArgs, ctx: &Context) -> Result<(), ForumError> {
    match args.cmd {
        Some(ReviseCmd::Body {
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
        }) => run_revise_body(
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
            ctx.clock.as_ref(),
        ),
        Some(ReviseCmd::Node {
            thread_id,
            node_id,
            body,
            body_file,
            edit,
            as_actor,
            force,
        }) => run_revise_node(
            thread_id,
            node_id,
            body,
            body_file,
            edit,
            as_actor,
            force,
            ctx.clock.as_ref(),
        ),
        None => {
            let thread_id = args.thread_id.ok_or_else(|| {
                ForumError::Config(
                    "usage: git forum revise <THREAD_ID> --body <TEXT> | --body-file <PATH> | --edit".into(),
                )
            })?;
            run_revise_body(
                thread_id,
                args.body,
                args.body_file,
                args.edit,
                args.incorporates,
                args.as_actor,
                args.force,
                ctx.clock.as_ref(),
            )
        }
    }
}

/// `git forum revise` sub-commands.
#[derive(Subcommand)]
pub enum ReviseCmd {
    /// Revise the body of a thread
    Body {
        thread_id: String,
        /// New thread body text (use "-" to read from stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read new thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        /// Node IDs to mark as incorporated into this body revision
        #[arg(long = "incorporates", alias = "incorporate", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Revise the body of an existing node
    Node {
        thread_id: String,
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_id: String,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read revised body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
}

/// Body revision: rewrite `body.md` in the snapshot tree.
///
/// SPEC-3.0 §4.2 has no body_revision_count — revision history is
/// `git log` over the snapshot ref. `--incorporates` flips the named
/// nodes to `NodeStatus::Incorporated` in the same snapshot commit.
#[allow(clippy::too_many_arguments)]
pub fn run_revise_body(
    thread_id: String,
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    incorporates: Vec<String>,
    as_actor: Option<String>,
    force: bool,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = resolve_tid(&git, &thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);
    let body_text = resolve_body_required(
        body,
        body_file,
        edit,
        &format!("Revise body for {thread_id}"),
    )?;

    let state = thread::replay_thread(&git, &thread_id)?;
    let violations = operation_check::check_revise(&policy, state.status.as_str(), true);
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let mut doc = match snapshot::read_snapshot(&git, &thread_id) {
        Ok(doc) => doc,
        Err(ForumError::LegacyEventChain) => migrate_legacy_to_snapshot(&git, &thread_id)?,
        Err(other) => return Err(other),
    };
    let now = clock.now();
    doc.body = Some(body_text);
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.clone();

    for nref in &incorporates {
        let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, nref)?;
        if let Some(n) = doc.nodes.iter_mut().find(|n| n.record.id == resolved) {
            n.record.status = NodeStatus::Incorporated;
            n.record.updated_at = Some(now);
            n.record.updated_by = Some(actor.clone());
        }
    }

    write_snapshot(
        &git,
        &thread_id,
        &doc,
        &format!("revise body of {thread_id}"),
    )?;
    println!("Body revised for {thread_id}");
    Ok(())
}

/// Node revision: rewrite a single node's body in place.
#[allow(clippy::too_many_arguments)]
pub fn run_revise_node(
    thread_id: String,
    node_id: String,
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    as_actor: Option<String>,
    force: bool,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = resolve_tid(&git, &thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);
    let body_text = resolve_body_required(
        body,
        body_file,
        edit,
        &format!("Revise node {node_id} in {thread_id}"),
    )?;

    let state = thread::replay_thread(&git, &thread_id)?;
    let violations = operation_check::check_revise(&policy, state.status.as_str(), false);
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
    let mut doc = match snapshot::read_snapshot(&git, &thread_id) {
        Ok(doc) => doc,
        Err(ForumError::LegacyEventChain) => migrate_legacy_to_snapshot(&git, &thread_id)?,
        Err(other) => return Err(other),
    };
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
    node.body = body_text;
    node.record.updated_at = Some(now);
    node.record.updated_by = Some(actor.clone());
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.clone();

    write_snapshot(
        &git,
        &thread_id,
        &doc,
        &format!("revise node {resolved} in {thread_id}"),
    )?;
    println!("Revised {}", show::short_oid(&resolved));
    Ok(())
}

/// Resolve `reply_to` (a node ref or `None`) to a full node ID inside `thread_id`.
pub fn resolve_reply_to(
    git: &crate::internal::git_ops::GitOps,
    thread_id: &str,
    reply_to: Option<&str>,
) -> Result<Option<String>, ForumError> {
    match reply_to {
        Some(node_ref) => {
            let resolved = thread::resolve_node_id_in_thread(git, thread_id, node_ref)?;
            Ok(Some(resolved))
        }
        None => Ok(None),
    }
}
