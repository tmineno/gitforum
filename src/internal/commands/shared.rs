//! Helpers shared across `commands::*` orchestration modules.
//!
//! These wrap small pieces that previously lived in `main.rs` and were used
//! by every `run_*` function: repo discovery with the init warning,
//! operation-check application, actor/thread-id resolution, plus the CLI
//! parsing helpers relocated by task `t8o3vnt6`. Kept here (not
//! re-introduced as a Service / DTO layer per #yjelk0s0 Out-of-scope) so
//! command modules don't need a back-reference to `main.rs`.

use crate::internal::actor;
use crate::internal::config::{self, RepoPaths};
use crate::internal::error::ForumError;
use crate::internal::git_ops::GitOps;
use crate::internal::operation_check;
use crate::internal::policy;
use crate::internal::thread;
use crate::internal::thread::ThreadKind;

/// Apply the result of an operation-check pass: print violations to stderr,
/// and return a `Policy` error if any are blocking. `force` and `strict`
/// flow from the CLI flags / policy config.
pub fn apply_operation_checks(
    violations: &[operation_check::OperationViolation],
    force: bool,
    strict: bool,
) -> Result<(), ForumError> {
    if violations.is_empty() {
        return Ok(());
    }
    let (has_errors, output) = operation_check::evaluate_violations(violations, force, strict);
    eprint!("{output}");
    if has_errors {
        Err(ForumError::Policy(
            "operation blocked by check violations".into(),
        ))
    } else {
        Ok(())
    }
}

/// Discover the surrounding git repo and `.forum/` paths, warning the user
/// when the repo has not been initialised yet. Returns the `GitOps` handle
/// (with commit identity / default actor pre-loaded from local config) and
/// the resolved `RepoPaths`.
pub fn discover_repo_with_init_warning() -> Result<(GitOps, RepoPaths), ForumError> {
    let mut git = GitOps::discover()?;
    let git_dir = git.git_dir()?;
    let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
    if !is_forum_initialized(&paths, &git) {
        eprintln!(
            "warning: git-forum is not initialized in this repository; run `git forum init` first"
        );
    }
    let local_cfg = config::load_local_config(&paths).unwrap_or_default();
    if let Some(identity) = local_cfg.commit_identity {
        git.set_commit_identity(identity);
    }
    if let Some(default_actor) = local_cfg.default_actor {
        git.set_default_actor(default_actor);
    }
    Ok((git, paths))
}

fn is_forum_initialized(paths: &RepoPaths, git: &GitOps) -> bool {
    if paths.dot_forum.join("policy.toml").is_file() && paths.git_forum.join("logs").is_dir() {
        return true;
    }
    git.list_refs("refs/forum/threads/")
        .map(|refs| !refs.is_empty())
        .unwrap_or(false)
}

/// Resolve the effective actor for a CLI write: prefer `--as`, otherwise
/// fall back to the local actor config (defaulted from git identity).
pub fn resolve_actor(as_actor: Option<String>, git: &GitOps) -> String {
    as_actor.unwrap_or_else(|| actor::current_actor(git, git.default_actor()))
}

/// Resolve a user-supplied thread reference to its canonical full ID.
/// Wraps `thread::resolve_thread_id` for use from CLI command handlers.
pub fn resolve_tid(git: &GitOps, user_input: &str) -> Result<String, ForumError> {
    thread::resolve_thread_id(git, user_input)
}

// =============================================================================
// CLI parsing helpers — relocated from main.rs by task `t8o3vnt6`.
// =============================================================================

/// Parse a kind preset name (`rfc`, `dec`, `task`, `issue`, `bug`, plus
/// historical aliases) into the canonical `ThreadKind`. Used by `Ls`,
/// `Shortlog`, and the `--kind` filter on `state bulk`.
///
/// Routes through the 3.0-native `policy::preset_lookup` (returns a
/// `CategoryPreset`) and maps the preset's canonical name to the v2
/// `ThreadKind` enum locally. The whole helper retires in v3.1 step 3h
/// when `ThreadKind` is dropped in favour of category strings.
pub fn parse_thread_kind(kind: &str) -> Result<ThreadKind, ForumError> {
    let preset = policy::preset_lookup(kind).ok_or_else(|| {
        let valid: Vec<&str> = policy::presets().iter().map(|p| p.name).collect();
        ForumError::Config(format!(
            "unknown kind '{kind}'; valid presets: {}",
            valid.join(", "),
        ))
    })?;
    Ok(match preset.name {
        "rfc" => ThreadKind::Rfc,
        "dec" => ThreadKind::Dec,
        "task" => ThreadKind::Task,
        "issue" => ThreadKind::Issue,
        // policy::presets() is closed over the four built-in preset
        // names; any new preset must add a ThreadKind mapping here
        // (or, post-3h, the whole helper goes away).
        other => unreachable!("unmapped preset name: {other}"),
    })
}

/// Optional-input variant of [`parse_thread_kind`]. Returns `Ok(None)`
/// when the caller passed `None` (no `--kind` flag).
pub fn parse_thread_kind_filter(kind: Option<&str>) -> Result<Option<ThreadKind>, ForumError> {
    kind.map(parse_thread_kind).transpose()
}

/// Parse a `--since` value (ISO date, RFC 3339, or git revision) into
/// a UTC instant. Used by `Log` and `Shortlog`.
pub fn parse_since_date(
    since: &str,
    git: &GitOps,
) -> Result<chrono::DateTime<chrono::Utc>, ForumError> {
    use chrono::{DateTime, NaiveDate, Utc};
    if let Ok(naive) = NaiveDate::parse_from_str(since, "%Y-%m-%d") {
        return Ok(naive.and_hms_opt(0, 0, 0).unwrap().and_utc());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(since) {
        return Ok(dt.with_timezone(&Utc));
    }
    git.commit_timestamp(since)
}

// `terminal_state_date` relocated to `commands::shortlog::terminal_state_date`
// at Phase 2 slot 7b (RFC `7ymtc4b2`).

// =============================================================================
// Clap-error hint helpers — relocated from main.rs by task `t8o3vnt6`.
// =============================================================================

/// Extract the subcommand name from a clap "unrecognized subcommand"
/// error message: `"error: unrecognized subcommand 'foo'"` → `Some("foo")`.
pub fn parse_unrecognized_subcommand(msg: &str) -> Option<String> {
    let marker = "unrecognized subcommand '";
    let start = msg.find(marker)? + marker.len();
    let end = msg[start..].find('\'')?;
    Some(msg[start..start + end].to_string())
}

/// Return a custom hint for known unrecognized subcommands.
///
/// SPEC-2.0 §10.2: kind-prefixed subcommand groupings (`git forum rfc new`,
/// `git forum issue close`, etc.) are removed in 2.0. Invoking them
/// prints a hard error pointing at the top-level form.
pub fn subcommand_hint(sub: &str) -> Option<&'static str> {
    match sub {
        "rfc" | "issue" | "ask" | "dec" | "task" | "job" => Some(
            "kind-prefixed subcommand groupings (`git forum rfc new`, \
             `git forum issue close`, etc.) were removed in 2.0 (SPEC-2.0 §10.2). \
             Use the top-level form:\n  \
             git forum new <kind> \"title\"      (create — kinds: rfc, dec, task, issue, bug)\n  \
             git forum close|accept|propose|pend|reject|withdraw|deprecate <ID>\n  \
             git forum thread new --lifecycle <X> --tag <Y> ...   (canonical / scripts)",
        ),
        "say" => Some(
            "\"say\" is an internal module, not a CLI command. \
             Use node shorthands instead:\n  \
             git forum comment, objection, action  (canonical 2.0)\n  \
             or: git forum node add <THREAD> --type <TYPE> \"body\"",
        ),
        "revise-body" => Some(
            "use `git forum revise <THREAD_ID>` to revise a thread body, \
             or `git forum revise node <NODE_ID> <THREAD_ID>` to revise a node",
        ),
        "create" => Some("use `git forum new <kind> \"title\"` to create a thread"),
        "add" => Some("use `git forum node add <THREAD> --type <TYPE> \"body\"` to add a node"),
        _ => None,
    }
}
