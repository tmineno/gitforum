//! Helpers shared across `commands::*` orchestration modules.
//!
//! These wrap small pieces that previously lived in `main.rs` and were used
//! by every `run_*` function: repo discovery with the init warning,
//! operation-check application, actor/thread-id resolution, plus the CLI
//! parsing helpers relocated by task `t8o3vnt6`. Kept here (not
//! re-introduced as a Service / DTO layer per thread `yjelk0s0` Out-of-scope) so
//! command modules don't need a back-reference to `main.rs`.

use crate::internal::actor;
use crate::internal::config::{self, RepoPaths};
use crate::internal::error::ForumError;
use crate::internal::git_ops::GitOps;
use crate::internal::operation_check;
use crate::internal::policy;
use crate::internal::thread;

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

/// Outcome of inspecting the positional args of a node-targeting command
/// (`resolve` / `retract` / `reopen` / `retype`). Per ticket `ycnxmj0y`:
///
/// - **Single positional** that resolves only as a node id: returns the
///   discovered owning thread plus that node id (`explicit_thread = false`).
/// - **Single positional** that resolves as a thread id: caller-specific
///   handling (e.g. `reopen <thread>` reopens the thread itself);
///   surfaces as `explicit_thread = true` with no node ids.
/// - **Two-or-more positionals** with the first arg resolving as a thread:
///   explicit-safety form. Returns the named thread and the remaining args
///   as node refs (`explicit_thread = true`). Per-node mismatches are
///   reported by the lifecycle loop with the
///   `node X is in thread Y, not <wrong-thread>` hint.
/// - **Two-or-more positionals** with no thread match on the first arg:
///   all args are treated as node refs and must share one owning thread.
pub struct NodeTargetSelection {
    pub thread_id: String,
    pub node_refs: Vec<String>,
    pub explicit_thread: bool,
}

/// Decide how to interpret the positional args of a node-targeting CLI
/// command (`resolve` / `retract` / `reopen` / `retype`). See
/// [`NodeTargetSelection`] for the resolution rules.
pub fn resolve_node_targets(
    git: &GitOps,
    args: &[String],
) -> Result<NodeTargetSelection, ForumError> {
    if args.is_empty() {
        return Err(ForumError::Repo("no thread or node id given".to_string()));
    }

    if let Ok(thread_id) = thread::resolve_thread_id(git, &args[0]) {
        return Ok(NodeTargetSelection {
            thread_id,
            node_refs: args[1..].to_vec(),
            explicit_thread: true,
        });
    }

    let index = thread::NodeIdIndex::build(git)?;
    let mut shared_thread: Option<String> = None;
    for arg in args {
        let (_, t) = index.resolve(arg).map_err(|inner| {
            ForumError::Repo(format!(
                "'{arg}' is not a known thread id or node id ({inner})"
            ))
        })?;
        match shared_thread.as_deref() {
            None => shared_thread = Some(t),
            Some(existing) if existing == t => {}
            Some(existing) => {
                return Err(ForumError::Repo(format!(
                    "node ids span multiple threads ({existing} and {t}); \
                     pass the thread id explicitly to disambiguate"
                )));
            }
        }
    }
    Ok(NodeTargetSelection {
        thread_id: shared_thread.expect("non-empty args"),
        node_refs: args.to_vec(),
        explicit_thread: false,
    })
}

// =============================================================================
// CLI parsing helpers — relocated from main.rs by task `t8o3vnt6`.
// =============================================================================

/// Parse a kind preset name (`rfc`, `dec`, `task`, `issue`, `bug`, plus
/// historical aliases) into the canonical preset name string used by
/// `policy::kind_label_for` for filtering. Used by `Ls`, `Shortlog`,
/// and the `--kind` filter on `state bulk`.
///
/// Routes through the 3.0-native `policy::preset_lookup` (returns a
/// `CategoryPreset`); the canonical name is the row's `name` field.
/// task `1v400j3l` replaced the typed `ThreadKind`
/// return shape with the canonical preset name string.
pub fn parse_thread_kind(kind: &str) -> Result<&'static str, ForumError> {
    let preset = policy::preset_lookup(kind).ok_or_else(|| {
        let valid: Vec<&str> = policy::presets().iter().map(|p| p.name).collect();
        ForumError::Config(format!(
            "unknown kind '{kind}'; valid presets: {}",
            valid.join(", "),
        ))
    })?;
    Ok(preset.name)
}

/// Optional-input variant of [`parse_thread_kind`]. Returns `Ok(None)`
/// when the caller passed `None` (no `--kind` flag).
pub fn parse_thread_kind_filter(kind: Option<&str>) -> Result<Option<&'static str>, ForumError> {
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
// at task `1hg98odf`.

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
