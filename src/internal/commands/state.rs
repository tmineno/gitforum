//! `git forum close|accept|propose|pend|reject|withdraw|deprecate <ID>`
//! orchestration — the §9.3 shorthand verbs, plus the canonical
//! `git forum state <ID> <state>` form.
//!
//! Owns the clap subcommand enum `StateCmd` (moved from `main.rs` by
//! task `t8o3vnt6`).
//!
//! Phase 2 slot 3 (RFC `7ymtc4b2`): the write path updates
//! `thread.toml`'s `status` field directly via
//! `snapshot::store::write_snapshot` per SPEC-3.0 §3.1 and adds any
//! companion side-effects (comment node, approval nodes, link
//! entries, action resolution) into the same snapshot commit. The
//! legacy `state_change::change_state` event-write path is no longer
//! invoked here.

use chrono::Utc;
use clap::Subcommand;

use super::context::Context;
use crate::internal::clock::Clock;
use crate::internal::commands::show;
use crate::internal::error::ForumError;
use crate::internal::event::Lifecycle;
use crate::internal::id_alloc;
use crate::internal::node::{NodeKind, NodeRecord, NodeStatus};
use crate::internal::policy::Policy;
use crate::internal::snapshot::{self, store::write_snapshot, Link, NodeWithBody, ThreadDocument};
use crate::internal::thread;
use crate::internal::workflow::SPEC;

use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};
use super::shorthand_say::migrate_legacy_to_snapshot;

/// `git forum state` sub-commands.
#[derive(Subcommand)]
pub enum StateCmd {
    /// Apply the same transition to multiple threads
    Bulk {
        #[arg(long = "to", value_name = "STATE")]
        new_state: String,
        thread_ids: Vec<String>,
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

/// Args for `commands::state::run` shorthand path
/// (close/accept/propose/pend/reject/withdraw/deprecate).
pub struct StateShorthandArgs {
    pub thread_id: String,
    pub new_state: String,
    pub approve: Vec<String>,
    pub as_actor: Option<String>,
    pub resolve_open_actions: bool,
    pub link_to: Vec<String>,
    pub rel: Option<String>,
    pub comment: Option<String>,
    pub fast_track: bool,
    pub force: bool,
}

/// Uniform shorthand entry point per task `t8o3vnt6`.
pub fn run(args: StateShorthandArgs, ctx: &Context) -> Result<(), ForumError> {
    run_state_shorthand(
        &args.thread_id,
        &args.new_state,
        &args.approve,
        args.as_actor,
        args.resolve_open_actions,
        &args.link_to,
        args.rel.as_deref(),
        args.comment.as_deref(),
        args.fast_track,
        args.force,
        ctx.clock.as_ref(),
    )
}

/// Resolve a state-change shorthand to a concrete target state for the
/// thread's current lifecycle, per SPEC-2.0 §9.3.
pub fn shorthand_target_for_lifecycle(
    shorthand: &str,
    lifecycle: Lifecycle,
) -> Result<&'static str, ForumError> {
    use crate::internal::workflow::ShorthandResolution::*;
    match SPEC.shorthand_target(shorthand, lifecycle) {
        Target(t) => Ok(t),
        NotApplicable(hint) => Err(ForumError::Config(format!("{hint} (SPEC-2.0 §9.3)"))),
        Unknown => Err(ForumError::Config(format!(
            "unknown state-change shorthand '{shorthand}'",
        ))),
    }
}

/// Resolve `new_state` as either a §9.3 shorthand verb (`close`,
/// `accept`, etc.) or a canonical 2.0 state name (`done`, `accepted`,
/// `open`, …). Used by both the shorthand arms and the canonical
/// `git forum state <ID> <STATE>` form.
fn resolve_target_state(new_state: &str, lifecycle: Lifecycle) -> Result<&'static str, ForumError> {
    use crate::internal::workflow::ShorthandResolution::*;
    match SPEC.shorthand_target(new_state, lifecycle) {
        Target(t) => Ok(t),
        NotApplicable(hint) => Err(ForumError::Config(format!("{hint} (SPEC-2.0 §9.3)"))),
        Unknown => {
            // Try as a canonical state name. The legacy 1.x → 2.0
            // alias fold (`closed` → `done`, `proposed` → `open`,
            // etc.) lives in `event::ThreadStatus::parse_lenient`.
            crate::internal::event::ThreadStatus::parse_lenient(new_state)
                .map(|s| s.as_str())
                .ok_or_else(|| {
                    ForumError::Config(format!(
                        "unknown state '{new_state}' for {lifecycle} lifecycle"
                    ))
                })
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_state_shorthand(
    thread_id: &str,
    new_state: &str,
    approve: &[String],
    as_actor: Option<String>,
    resolve_open_actions: bool,
    link_to: &[String],
    rel: Option<&str>,
    comment: Option<&str>,
    fast_track: bool,
    _force: bool,
    clock: &dyn Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
    let actor = resolve_actor(as_actor, &git);

    // Replay once up front to resolve the lifecycle facet — the §9.3
    // table is keyed on lifecycle, not on the legacy `kind` field.
    let pre_state = thread::replay_thread(&git, thread_id)?;
    let target = resolve_target_state(new_state, pre_state.lifecycle)?;

    let mut doc = match snapshot::read_snapshot(&git, thread_id) {
        Ok(doc) => doc,
        Err(ForumError::LegacyEventChain) => migrate_legacy_to_snapshot(&git, thread_id)?,
        Err(other) => return Err(other),
    };
    let from = doc.snapshot.status.clone();
    let now = clock.now();

    if from == target {
        // No-op (already in target state). Honour `--comment` by
        // attaching a standalone Comment node so operators can leave
        // a note even on a no-op.
        if let Some(text) = comment {
            doc.nodes.push(comment_node(&actor, text, now));
            doc.snapshot.updated_at = now;
            doc.snapshot.updated_by = actor.clone();
            write_snapshot(
                &git,
                thread_id,
                &doc,
                &format!("state no-op (comment recorded) on {thread_id}"),
            )?;
            println!(
                "note: {thread_id} is already in state '{from}'; no transition recorded (comment attached as a standalone node)"
            );
        } else {
            println!("note: {thread_id} is already in state '{from}'; no transition recorded");
        }
        emit_action_hint(&git, thread_id, &policy);
        return Ok(());
    }

    // Validate the transition. With `fast_track`, find_path walks
    // intermediate states; otherwise the direct edge must be legal.
    let walked: Vec<&'static str> = if fast_track {
        SPEC.find_path(pre_state.lifecycle, &from, target)
            .ok_or_else(|| {
                ForumError::Config(format!(
                    "no legal path from '{from}' to '{target}' for {} lifecycle",
                    pre_state.lifecycle
                ))
            })?
    } else if SPEC.is_valid_transition(pre_state.lifecycle, &from, target) {
        vec![target]
    } else {
        return Err(ForumError::Config(format!(
            "transition '{from}' → '{target}' is not allowed for {} lifecycle",
            pre_state.lifecycle
        )));
    };

    // Update status to the final hop. The snapshot model carries
    // the cumulative state, so we write one snapshot landing on
    // the final target — the intermediate hops are still validated
    // against the workflow graph by `find_path`.
    let final_state = walked.last().expect("walked is non-empty after validation");
    doc.snapshot.status = (*final_state).to_string();
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.clone();

    if resolve_open_actions {
        for n in doc.nodes.iter_mut() {
            if matches!(n.record.kind, NodeKind::Action)
                && matches!(n.record.status, NodeStatus::Open)
            {
                n.record.status = NodeStatus::Resolved;
                n.record.updated_at = Some(now);
                n.record.updated_by = Some(actor.clone());
            }
        }
    }

    for approver in approve {
        doc.nodes.push(approval_node(approver, now));
    }

    if let Some(text) = comment {
        doc.nodes.push(comment_node(&actor, text, now));
    }

    if !link_to.is_empty() {
        let rel = rel
            .ok_or_else(|| ForumError::Config("--rel is required when --link-to is used".into()))?;
        for target_id in link_to {
            let resolved_target = resolve_tid(&git, target_id)?;
            doc.links.entries.push(Link {
                target: resolved_target,
                rel: rel.into(),
                created_at: now,
                created_by: actor.clone(),
            });
        }
    }

    write_snapshot(
        &git,
        thread_id,
        &doc,
        &format!("state {} -> {}", from, final_state),
    )?;

    if fast_track {
        for (i, step) in walked.iter().enumerate() {
            let is_final = i == walked.len() - 1;
            if is_final {
                println!("{thread_id} -> {step}");
            } else {
                eprintln!("  {thread_id}: -> {step}");
            }
        }
    } else {
        println!("{thread_id} -> {target}");
    }

    emit_action_hint(&git, thread_id, &policy);
    Ok(())
}

/// Snapshot-native state change applied to one thread.
///
/// Mirrors the single-thread shorthand path (resolve target,
/// validate transition, mutate `thread.toml.status` + companion
/// fields, write one snapshot commit) but without the surrounding
/// stdout / action-hint chatter — the bulk caller owns its own
/// reporting. Returns the resolved `(from, to)` pair on success
/// (with an empty Vec when the thread is already in the target
/// state).
#[allow(clippy::too_many_arguments)]
pub fn apply_state_change_snapshot(
    git: &crate::internal::git_ops::GitOps,
    policy: &Policy,
    thread_id: &str,
    new_state: &str,
    approve: &[String],
    actor: &str,
    clock: &dyn Clock,
    resolve_open_actions: bool,
) -> Result<(String, &'static str), ForumError> {
    let pre_state = thread::replay_thread(git, thread_id)?;
    let target = resolve_target_state(new_state, pre_state.lifecycle)?;

    let mut doc = match snapshot::read_snapshot(git, thread_id) {
        Ok(doc) => doc,
        Err(ForumError::LegacyEventChain) => migrate_legacy_to_snapshot(git, thread_id)?,
        Err(other) => return Err(other),
    };
    let from = doc.snapshot.status.clone();
    if from == target {
        return Ok((from, target));
    }

    if !SPEC.is_valid_transition(pre_state.lifecycle, &from, target) {
        return Err(ForumError::Config(format!(
            "transition '{from}' → '{target}' is not allowed for {} lifecycle",
            pre_state.lifecycle
        )));
    }

    // Project the post-transition state so policy guards see the
    // synthetic Approval nodes / resolved actions the transition is
    // about to write — `--resolve-open-actions` cures `no_open_actions`,
    // and explicit `--approve <ACTOR>` cures `one_human_approval`.
    let mut projected = pre_state.clone();
    projected.status =
        crate::internal::event::ThreadStatus::parse_lenient(target).unwrap_or(projected.status);
    if resolve_open_actions {
        for n in projected.nodes.iter_mut() {
            if n.node_type == crate::internal::event::NodeType::Action && n.is_open() {
                n.resolved = true;
            }
        }
    }
    for approver in approve {
        projected.nodes.push(crate::internal::node::Node {
            node_id: format!("approval-{approver}"),
            node_type: crate::internal::event::NodeType::Approval,
            actor: approver.clone(),
            ..Default::default()
        });
    }
    let guard_violations = crate::internal::policy::check_guards(policy, &projected, &from, target);
    if !guard_violations.is_empty() {
        let parts: Vec<String> = guard_violations
            .iter()
            .map(|v| format!("[{}] {}", v.rule, v.reason))
            .collect();
        return Err(ForumError::Policy(parts.join("; ")));
    }

    let now = clock.now();
    doc.snapshot.status = target.into();
    doc.snapshot.updated_at = now;
    doc.snapshot.updated_by = actor.into();

    if resolve_open_actions {
        for n in doc.nodes.iter_mut() {
            if matches!(n.record.kind, NodeKind::Action)
                && matches!(n.record.status, NodeStatus::Open)
            {
                n.record.status = NodeStatus::Resolved;
                n.record.updated_at = Some(now);
                n.record.updated_by = Some(actor.into());
            }
        }
    }

    for approver in approve {
        doc.nodes.push(approval_node(approver, now));
    }

    write_snapshot(git, thread_id, &doc, &format!("state {from} -> {target}"))?;

    Ok((from, target))
}

fn comment_node(actor: &str, body: &str, now: chrono::DateTime<Utc>) -> NodeWithBody {
    NodeWithBody {
        record: NodeRecord {
            id: id_alloc::alloc_bare_thread_id(actor, body, &now.to_rfc3339()),
            kind: NodeKind::Comment,
            status: NodeStatus::Open,
            created_at: now,
            created_by: actor.into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: body.into(),
    }
}

fn approval_node(approver: &str, now: chrono::DateTime<Utc>) -> NodeWithBody {
    NodeWithBody {
        record: NodeRecord {
            id: id_alloc::alloc_bare_thread_id(approver, "approval", &now.to_rfc3339()),
            kind: NodeKind::Approval,
            status: NodeStatus::Open,
            created_at: now,
            created_by: approver.into(),
            updated_at: None,
            updated_by: None,
            reply_to: None,
            legacy_label: None,
        },
        body: String::new(),
    }
}

fn emit_action_hint(git: &crate::internal::git_ops::GitOps, thread_id: &str, policy: &Policy) {
    if let Ok(state) = thread::replay_thread(git, thread_id) {
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
}

// `ThreadDocument` is referenced through the snapshot module; the
// re-export here keeps later slots' helpers inside `commands::state`
// from needing the full path.
#[allow(dead_code)]
pub(crate) type Doc = ThreadDocument;
