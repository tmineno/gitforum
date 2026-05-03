//! `git forum revise` orchestration — body and node revisions.
//!
//! The clap subcommand enum `ReviseCmd` stays in `main.rs` (per #yjelk0s0
//! out-of-scope: no clap restructuring); main.rs destructures the variants
//! and calls the appropriate function below.

use std::path::PathBuf;

use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::operation_check;
use crate::internal::policy::Policy;
use crate::internal::show;
use crate::internal::thread;
use crate::internal::write_ops;

use super::shared::{
    apply_operation_checks, discover_repo_with_init_warning, resolve_actor, resolve_tid,
};
use super::thread_new::resolve_body_required;

/// Body revision: rewrite the thread body and bump `body_revision_count`.
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

    write_ops::revise_body(&git, &thread_id, &body_text, &incorporates, &actor, clock)?;
    let revision = state.body_revision_count + 1;
    println!("Body revised for {thread_id} (revision {revision})");
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
    write_ops::revise_node(&git, &thread_id, &resolved, &body_text, &actor, clock)?;
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
