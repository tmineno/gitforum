//! `git forum close|accept|propose|pend|reject|withdraw|deprecate <ID>`
//! orchestration — the §9.3 shorthand verbs.
//!
//! The shorthand→target mapping itself lives in
//! [`WorkflowSpec`](crate::internal::workflow::WorkflowSpec) (#34ith16h);
//! this module only handles the I/O and state-change wiring.

use crate::internal::clock::Clock;
use crate::internal::error::ForumError;
use crate::internal::event::Lifecycle;
use crate::internal::evidence;
use crate::internal::policy::Policy;
use crate::internal::show;
use crate::internal::state_change;
use crate::internal::thread;
use crate::internal::workflow::SPEC;

use super::shared::{discover_repo_with_init_warning, resolve_actor, resolve_tid};

/// Resolve a state-change shorthand to a concrete target state for the
/// thread's current lifecycle, per SPEC-2.0 §9.3.
///
/// Thin wrapper over
/// [`SPEC::shorthand_target`](crate::internal::workflow::WorkflowSpec::shorthand_target);
/// the §9.3 table itself lives in `workflow.rs`. This wrapper turns the
/// typed [`ShorthandResolution`](crate::internal::workflow::ShorthandResolution)
/// into the CLI-shaped `ForumError`.
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
    // Replay once up front to resolve the lifecycle facet — the §9.3 table
    // is keyed on lifecycle, not the legacy `kind` field.
    let pre_state = thread::replay_thread(&git, thread_id)?;
    let target = shorthand_target_for_lifecycle(new_state, pre_state.lifecycle)?;
    let options = state_change::StateChangeOptions {
        resolve_open_actions,
        comment: comment.map(|s| s.to_string()),
    };
    if fast_track {
        let walked = state_change::fast_track_state(
            &git, thread_id, target, approve, &actor, clock, &policy, options,
        )?;
        for (i, step) in walked.iter().enumerate() {
            let is_final = i == walked.len() - 1;
            if is_final {
                println!("{thread_id} -> {step}");
            } else {
                eprintln!("  {thread_id}: -> {step}");
            }
        }
    } else {
        let outcome = state_change::change_state(
            &git, thread_id, target, approve, &actor, clock, &policy, options,
        )?;
        match outcome {
            state_change::StateChangeOutcome::Applied { .. } => {
                println!("{thread_id} -> {target}");
            }
            state_change::StateChangeOutcome::NoOp {
                state,
                comment_recorded,
            } => {
                if comment_recorded {
                    println!(
                        "note: {thread_id} is already in state '{state}'; no transition recorded (comment attached as a standalone node)"
                    );
                } else {
                    println!(
                        "note: {thread_id} is already in state '{state}'; no transition recorded"
                    );
                }
            }
        }
    }
    if !link_to.is_empty() {
        let rel = rel
            .ok_or_else(|| ForumError::Config("--rel is required when --link-to is used".into()))?;
        for target in link_to {
            let resolved_target = resolve_tid(&git, target)?;
            evidence::add_thread_link(&git, thread_id, &resolved_target, rel, &actor, clock)?;
        }
    }
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
