//! `git forum comment | objection | action | claim | question | summary | risk | review`
//! orchestration. All collapse onto [`run_shorthand_say`]; the rhetorical
//! 1.x labels (`claim` / `question` / `summary` / `risk` / `review`) are
//! aliased to `comment` per ADR-006 with the user's chosen label preserved
//! on the event as `legacy_subtype`.

use std::path::PathBuf;

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::event::NodeType;
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::show;
use crate::internal::thread;
use crate::internal::write_ops;

use super::revise::resolve_reply_to;
use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};
use super::thread_new::resolve_body_required;

/// Args for `commands::shorthand_say::run` — the shared field set used
/// by `comment` / `objection` / `action` / `claim` / `question` /
/// `summary` / `risk` / `review` / `node add`.
pub struct ShorthandSayArgs {
    pub thread_id: String,
    pub body_positional: Option<String>,
    pub body_flag: Option<String>,
    pub body_file: Option<PathBuf>,
    pub edit: bool,
    pub reply_to: Option<String>,
    pub as_actor: Option<String>,
    pub node_type: NodeType,
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
        args.node_type,
        args.force,
        ctx.clock.as_ref(),
    )
}

/// Print a deprecation warning for legacy node-type shorthand commands.
///
/// Per SPEC-2.0 §9.3 / ADR-006, the rhetorical-only shorthands
/// `claim` / `question` / `summary` / `risk` / `review` are aliased to
/// `comment` for one minor release and removed in 3.0. The user's chosen
/// label is preserved on the event as `legacy_subtype`.
pub fn warn_legacy_node_shorthand(name: &str) {
    eprintln!(
        "warning: `git forum {name}` is deprecated; the canonical 2.0 form is `git forum comment` \
         (the rhetorical label is preserved). This shorthand will be removed in 3.0."
    );
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
    node_type: NodeType,
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
        &format!("Compose a {node_type} node"),
    )?;

    // Operation check: is this node type allowed in the current state?
    let state = thread::replay_thread(&git, thread_id)?;
    let violations = operation_check::check_say(&policy, state.status.as_str(), node_type);
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let resolved_reply = resolve_reply_to(&git, thread_id, reply_to.as_deref())?;
    let node_id = write_ops::say_node(
        &git,
        thread_id,
        node_type,
        &body_text,
        &actor,
        clock,
        resolved_reply.as_deref(),
    )?;
    println!("Added {node_type} {}", show::short_oid(&node_id));
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
