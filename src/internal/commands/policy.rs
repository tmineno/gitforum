//! `git forum policy {show, lint, check}` orchestration.
//!
//! task `1hg98odf`: NEW module. task `qa8u71j9` already rewrote
//! `internal::policy` to the SPEC-3.0 category registry
//! (`CategoryRegistry`); this slot exposes the rewritten library at
//! the CLI surface. The arm body relocates from `main.rs` to
//! [`run_arm`] here.

use crate::internal::error::ForumError;
use crate::internal::policy::{self, Policy};
use crate::internal::thread;

use super::context::Context;
use super::shared::resolve_tid;

/// Variants for [`run_arm`]. Mirrors the clap `PolicyCmd` enum.
pub enum PolicyArm {
    Show,
    Lint,
    Check {
        thread_id: String,
        transition: String,
    },
}

/// Uniform entry point for the `policy` subcommand cluster.
pub fn run_arm(arm: PolicyArm, ctx: &Context) -> Result<(), ForumError> {
    match arm {
        PolicyArm::Show => run_show(ctx),
        PolicyArm::Lint => run_lint(ctx),
        PolicyArm::Check {
            thread_id,
            transition,
        } => run_check(ctx, &thread_id, &transition),
    }
}

fn run_show(ctx: &Context) -> Result<(), ForumError> {
    let policy = Policy::load(&ctx.paths.dot_forum.join("policy.toml"))?;
    print!("{}", policy::render_policy_show(&policy));
    Ok(())
}

fn run_lint(ctx: &Context) -> Result<(), ForumError> {
    let policy = Policy::load(&ctx.paths.dot_forum.join("policy.toml"))?;
    let diags = policy::lint_policy(&policy);
    if diags.is_empty() {
        println!("policy ok");
    } else {
        for d in &diags {
            println!("{d}");
        }
    }
    Ok(())
}

fn run_check(ctx: &Context, thread_id: &str, transition: &str) -> Result<(), ForumError> {
    let thread_id = resolve_tid(&ctx.git, thread_id)?;
    let policy = Policy::load(&ctx.paths.dot_forum.join("policy.toml"))?;
    let state = thread::replay_thread(&ctx.git, &thread_id)?;
    let parts: Vec<&str> = transition.splitn(2, "->").collect();
    if parts.len() != 2 {
        eprintln!("error: --transition must be 'from->to'");
        std::process::exit(1);
    }
    let violations = policy::check_guards(&policy, &state, parts[0], parts[1]);
    if violations.is_empty() {
        println!("transition {transition}: ok");
    } else {
        for v in &violations {
            println!("FAIL [{}] {}", v.rule, v.reason);
        }
        std::process::exit(1);
    }
    Ok(())
}
