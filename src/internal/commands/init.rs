//! `git forum init` orchestration.
//!
//! Phase 2 slot 10a (RFC `7ymtc4b2`): NEW module owning the `Init`
//! arm. Library code (`internal::init::*`) stays peer-level — it is
//! reused by the post-checkout hook (`hook worktree-init`) and by
//! tests. This module is the CLI-only handler.
//!
//! Phase 2 slot 11 already removed the SQLite reindex bootstrap; init
//! now stops after refspec setup + fetch + hook install (ADR-011
//! Decision 6: no index in v3.0.0).

use crate::internal::actor;
use crate::internal::error::ForumError;
use crate::internal::init;

use super::context::Context;
use super::hook;

/// Uniform entry point for the `init` subcommand. Takes no args.
///
/// Doubles as the post-clone bootstrap: when `.forum/policy.toml` is
/// already tracked, init skips re-seeding the shared config and instead
/// announces that it discovered an existing forum, then proceeds to
/// register the fetch refspec, fetch `refs/forum/*`, and create the
/// per-clone `.git/forum/` state. See README "Cloning a repo that
/// already uses git-forum".
pub fn run(ctx: &Context) -> Result<(), ForumError> {
    // Detect "fresh clone" before init_forum runs — `.forum/policy.toml`
    // tracked in the cloned tree is the signal that this repo already
    // uses git-forum and we just need to wire up local state + fetch.
    let policy_existed = ctx.paths.dot_forum.join("policy.toml").is_file();
    if policy_existed {
        println!("Detected existing forum config (.forum/policy.toml).");
    }

    init::init_forum(&ctx.paths)?;

    // Generate local.toml with default_actor if it doesn't exist.
    let local_toml_path = ctx.paths.git_forum.join("local.toml");
    if !local_toml_path.exists() {
        let default_actor = actor::actor_from_git_config(&ctx.git);
        let content = format!(
            "# git-forum local config (per-clone, not committed)\n\
             \n\
             # Default actor ID for this clone.\n\
             # Override per-command with --as or GIT_FORUM_ACTOR env var.\n\
             default_actor = \"{default_actor}\"\n\
             \n\
             # Override git commit author/committer on forum commits.\n\
             # Uncomment to use a pseudonym instead of git config user.name/email.\n\
             # [commit_identity]\n\
             # name = \"pseudonym\"\n\
             # email = \"pseudonym@example.com\"\n"
        );
        std::fs::write(&local_toml_path, content)?;
        println!("Default actor: {default_actor}");
        eprintln!("hint: edit .git/forum/local.toml to change your actor ID or commit identity");
    }

    // Configure fetch refspecs for forum refs on all remotes.
    match init::ensure_forum_refspecs(&ctx.git) {
        Ok(modified) => {
            for remote in &modified {
                eprintln!("Added forum fetch refspec for remote '{remote}'");
            }
        }
        Err(e) => {
            eprintln!("warning: could not configure forum fetch refspecs: {e}");
        }
    }

    // Fetch forum refs from all remotes. We diff the local ref set
    // before/after each fetch so users see how many threads they just
    // pulled — important UX cue for the post-clone path where the
    // generic "Fetched forum refs" line gives no signal that anything
    // actually arrived.
    if let Ok(remotes_output) = ctx.git.run(&["remote"]) {
        for remote in remotes_output.lines() {
            let remote = remote.trim();
            if remote.is_empty() {
                continue;
            }
            let before = ctx
                .git
                .list_refs("refs/forum/threads/")
                .map(|v| v.len())
                .unwrap_or(0);
            match ctx.git.run(&["fetch", remote, init::FORUM_REFSPEC]) {
                Ok(_) => {
                    let after = ctx
                        .git
                        .list_refs("refs/forum/threads/")
                        .map(|v| v.len())
                        .unwrap_or(0);
                    let new = after.saturating_sub(before);
                    if new > 0 {
                        let s = if new == 1 { "" } else { "s" };
                        eprintln!("Fetched {new} forum thread ref{s} from '{remote}'");
                    } else if after > 0 {
                        eprintln!("Fetched forum refs from '{remote}' (no new threads)");
                    } else {
                        eprintln!("Fetched forum refs from '{remote}' (remote has no threads)");
                    }
                }
                Err(e) => {
                    eprintln!("warning: could not fetch forum refs from '{remote}': {e}");
                }
            }
        }
    }

    let dir_name = ctx
        .git
        .root()
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ".".to_string());
    if policy_existed {
        println!("Forum ready in {dir_name}.");
    } else {
        println!("Initialized git-forum in {dir_name}");
    }
    eprintln!("note: actor IDs (--as) are claimed identities, not authenticated. Approvals are recorded, not cryptographically verified.");
    hook::install_all_hooks(&ctx.git, false)?;
    Ok(())
}
