//! Git hook support for git-forum.
//!
//! Provides:
//! - A commit-msg hook that validates thread ID references in commit messages.
//! - A post-checkout hook that initializes git-forum in fresh worktrees.
//! - A `fix-index` subcommand that detects and re-hashes missing blobs
//!   (manual recovery; also invoked by `git-forum doctor`).
//!
//! Phase 2 slot 10b (RFC `7ymtc4b2`): the `Hook::*` arm body relocates
//! from `main.rs` to [`run_arm`] in this module. The lower-level
//! installer / scanner functions stay here as the hook subsystem
//! library; the new entry-point dispatches across the four sub-arms.

use std::fs;
use std::path::Path;

use crate::internal::actor;
use crate::internal::config::RepoPaths;

use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::init;
use super::super::refs;

// ── commit-msg hook ─────────────────────────────────────────────────

const COMMIT_MSG_HOOK_MARKER: &str = "# git-forum advisory commit-msg hook";

const COMMIT_MSG_HOOK_SCRIPT: &str = r#"#!/bin/sh
# git-forum advisory commit-msg hook
git-forum hook check-commit-msg "$1"
"#;

// ── post-checkout hook ──────────────────────────────────────────────

const POST_CHECKOUT_HOOK_MARKER: &str = "# git-forum post-checkout hook";

const POST_CHECKOUT_HOOK_SCRIPT: &str = r#"#!/bin/sh
# git-forum post-checkout hook
git-forum hook worktree-init
"#;

// ── arm dispatcher ──────────────────────────────────────────────────

/// Variants for [`run_arm`]. Mirrors the clap `HookCmd` enum in `main.rs`
/// 1:1 so the dispatcher can simply forward.
pub enum HookArm {
    Install { force: bool },
    Uninstall,
    CheckCommitMsg { file: std::path::PathBuf },
    FixIndex,
    WorktreeInit,
}

/// Uniform entry point for the `hook` subcommand cluster.
///
/// Each sub-arm operates on a `GitOps` only; we deliberately do not
/// emit the `git-forum is not initialized` warning, since the
/// post-checkout hook may run mid-clone before the forum is set up.
/// Use `Context::discover_quiet` at the call site.
pub fn run_arm(arm: HookArm, ctx: &super::context::Context) -> Result<(), ForumError> {
    match arm {
        HookArm::Install { force } => install_all_hooks(&ctx.git, force),
        HookArm::Uninstall => uninstall_all_hooks(&ctx.git),
        HookArm::CheckCommitMsg { file } => run_check_commit_msg(&ctx.git, &file),
        HookArm::FixIndex => run_fix_index(&ctx.git),
        HookArm::WorktreeInit => run_worktree_init(&ctx.git),
    }
}

fn run_check_commit_msg(git: &GitOps, file: &Path) -> Result<(), ForumError> {
    let raw = fs::read_to_string(file)?;
    let comment_char = get_comment_char(git);
    let cleaned = strip_comments(&raw, comment_char);
    let ids = extract_thread_ids(&cleaned);
    if ids.is_empty() {
        eprintln!("git-forum: warning: no thread ID referenced in commit message");
        return Ok(());
    }
    let result = check_thread_refs(git, &ids)?;
    if result.has_errors() {
        eprintln!("git-forum: commit message references non-existent thread(s):");
        for id in &result.missing_ids {
            eprintln!("  {id} — not found");
        }
        eprintln!(
            "hint: create the thread first, or remove the reference from the commit message."
        );
        std::process::exit(1);
    }
    Ok(())
}

fn run_fix_index(git: &GitOps) -> Result<(), ForumError> {
    let result = fix_index_blobs(git)?;
    for (path, sha) in &result.fixed {
        eprintln!("fix-index: re-hashed {path} (missing blob {sha})");
    }
    for (path, sha) in &result.warnings {
        eprintln!("fix-index: WARNING — {path} has missing blob {sha} and no working-tree copy");
    }
    if result.fixed.is_empty() && result.warnings.is_empty() {
        eprintln!("fix-index: all index blobs present");
    }
    Ok(())
}

fn run_worktree_init(git: &GitOps) -> Result<(), ForumError> {
    let git_dir = git.git_dir()?;
    let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
    if paths.git_forum.join("logs").is_dir() {
        return Ok(());
    }
    // Per ADR-007: worktree-init writes only .git/forum/ local state.
    // Tracked .forum/ content arrives via checkout, never via this hook.
    init::init_forum_local(&paths)?;
    let local_toml_path = paths.git_forum.join("local.toml");
    if !local_toml_path.exists() {
        let default_actor = actor::actor_from_git_config(git);
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
        fs::write(&local_toml_path, content)?;
    }
    let _ = init::ensure_forum_refspecs(git);
    install_all_hooks(git, false)?;
    eprintln!(
        "git-forum: initialized worktree at {}",
        git.root().display()
    );
    Ok(())
}

// ── fix-index result ────────────────────────────────────────────────

/// Result of running fix-index-blobs.
pub struct FixIndexResult {
    /// (path, old_sha) pairs that were re-hashed from the working tree.
    pub fixed: Vec<(String, String)>,
    /// (path, sha) pairs where the blob is missing AND no working-tree copy exists.
    pub warnings: Vec<(String, String)>,
}

/// Repair missing blob references in the git index AND in HEAD's tree.
///
/// Two passes:
/// 1. Iterate the staged index via `git ls-files --stage` and re-hash any
///    entry whose blob is missing (using the working-tree copy).
/// 2. Iterate HEAD's tree via `git ls-tree -r HEAD` and stage a re-add for
///    any entry whose blob is missing. This handles the case where HEAD
///    itself references a pruned blob — the next commit will then carry
///    the repair into a new tree.
///
/// Defense-in-depth recovery; see ADR-008. Invoked manually via
/// `git-forum hook fix-index` and as part of `git-forum doctor`.
///
/// Also runs `git worktree prune` first to clean up stale worktree metadata
/// that could cause GC to skip dead worktree indices.
pub fn fix_index_blobs(git: &GitOps) -> ForumResult<FixIndexResult> {
    // Prune stale worktrees so GC doesn't skip dead indices
    let _ = git.run(&["worktree", "prune"]);

    let mut fixed = Vec::new();
    let mut warnings = Vec::new();

    // Pass 1: index entries
    let output = git.run(&["ls-files", "--stage"])?;
    for line in output.lines() {
        if line.is_empty() {
            continue;
        }
        // Format: "100644 <sha> <stage>\t<path>"
        let Some((mode_sha_stage, path)) = line.split_once('\t') else {
            continue;
        };
        let fields: Vec<&str> = mode_sha_stage.split_whitespace().collect();
        if fields.len() < 2 {
            continue;
        }
        let sha = fields[1];

        if git.run(&["cat-file", "-e", sha]).is_err() {
            let full_path = git.root().join(path);
            if full_path.is_file() {
                git.run(&["update-index", "--force-remove", path])?;
                git.run(&["add", path])?;
                fixed.push((path.to_string(), sha.to_string()));
            } else {
                warnings.push((path.to_string(), sha.to_string()));
            }
        }
    }

    // Pass 2: HEAD-tree entries (only if HEAD exists)
    if git.run(&["rev-parse", "--verify", "HEAD"]).is_ok() {
        let head_tree = git.run(&["ls-tree", "-r", "HEAD"]).unwrap_or_default();
        for line in head_tree.lines() {
            if line.is_empty() {
                continue;
            }
            // Format: "100644 blob <sha>\t<path>"
            let Some((mode_type_sha, path)) = line.split_once('\t') else {
                continue;
            };
            let fields: Vec<&str> = mode_type_sha.split_whitespace().collect();
            if fields.len() < 3 || fields[1] != "blob" {
                continue;
            }
            let sha = fields[2];

            if git.run(&["cat-file", "-e", sha]).is_err() {
                // Skip if pass 1 already staged a repair for this path.
                if fixed.iter().any(|(p, _)| p == path) {
                    continue;
                }
                let full_path = git.root().join(path);
                if full_path.is_file() {
                    let _ = git.run(&["update-index", "--force-remove", path]);
                    git.run(&["add", path])?;
                    fixed.push((path.to_string(), sha.to_string()));
                } else {
                    warnings.push((path.to_string(), sha.to_string()));
                }
            }
        }
    }

    Ok(FixIndexResult { fixed, warnings })
}

/// Known v2 thread ID prefixes (must match the prefix table in
/// `id_alloc::KNOWN_THREAD_PREFIXES` and the legacy
/// `ThreadKind::id_prefix` mapping).
const KNOWN_PREFIXES: &[&str] = &["ASK", "ISSUE", "RFC", "DEC", "JOB", "TASK"];

/// Result of checking a commit message for thread references.
pub struct HookCheckResult {
    pub found_ids: Vec<String>,
    pub missing_ids: Vec<String>,
}

impl HookCheckResult {
    pub fn has_errors(&self) -> bool {
        !self.missing_ids.is_empty()
    }
}

/// Query the effective Git comment character (respects `core.commentChar`).
pub fn get_comment_char(git: &GitOps) -> char {
    git.run(&["config", "--get", "core.commentChar"])
        .ok()
        .and_then(|s| s.trim().chars().next())
        .unwrap_or('#')
}

/// Strip Git comment lines and scissors sections from a commit message.
pub fn strip_comments(message: &str, comment_char: char) -> String {
    let scissors = format!("{comment_char} --- >8 ---");
    let mut lines = Vec::new();
    for line in message.lines() {
        if line.starts_with(&scissors) {
            break;
        }
        if !line.starts_with(comment_char) {
            lines.push(line);
        }
    }
    lines.join("\n")
}

/// Extract git-forum thread IDs from a commit message.
///
/// Matches three forms (SPEC-2.0 §6.1):
/// - 2.0 display form: `@<8 base36 chars>` (e.g. `@e216r3on`)
/// - Legacy opaque: `KIND-<8 base36>` (e.g. `RFC-a7f3b2x1`)
/// - Legacy sequential: `KIND-NNNN` (e.g. `ISSUE-0001`)
///
/// Returns deduplicated results in match order. The leading `@` is stripped
/// so callers can resolve uniformly via `refs/forum/threads/<token>` or the
/// alias namespace.
pub fn extract_thread_ids(message: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let chars: Vec<char> = message.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Check word boundary: start of string or previous char is not alphanumeric.
        // The `@` marker is not alphanumeric, so it preserves the boundary.
        if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i += 1;
            continue;
        }

        // 2.0 display form: `@<8-char base36 token>`.
        if chars[i] == '@' {
            let token_start = i + 1;
            let mut token_len = 0;
            while token_start + token_len < len
                && token_len < 8
                && (chars[token_start + token_len].is_ascii_digit()
                    || chars[token_start + token_len].is_ascii_lowercase())
            {
                token_len += 1;
            }
            let end = token_start + token_len;
            let trailing_word = end < len && (chars[end].is_alphanumeric() || chars[end] == '_');
            let is_bare_token = token_len == 8
                && !chars[token_start..end].iter().all(|c| c.is_ascii_digit())
                && !trailing_word;
            if is_bare_token {
                let id: String = chars[token_start..end].iter().collect();
                if !ids.contains(&id) {
                    ids.push(id);
                }
                i = end;
                continue;
            }
        }

        for prefix in KNOWN_PREFIXES {
            let prefix_chars: Vec<char> = prefix.chars().collect();
            let prefix_len = prefix_chars.len();

            // Need at least prefix + '-' + 4 chars (minimum token length)
            if i + prefix_len + 1 + 4 > len {
                continue;
            }

            let mut matched = true;
            for (j, &pc) in prefix_chars.iter().enumerate() {
                if chars[i + j] != pc {
                    matched = false;
                    break;
                }
            }
            if !matched {
                continue;
            }

            // Check for '-' after prefix
            if chars[i + prefix_len] != '-' {
                continue;
            }

            // Collect the token: digits and lowercase letters after the dash
            let token_start = i + prefix_len + 1;
            let mut token_len = 0;
            while token_start + token_len < len
                && (chars[token_start + token_len].is_ascii_digit()
                    || chars[token_start + token_len].is_ascii_lowercase())
            {
                token_len += 1;
            }

            // Check trailing word boundary
            let end = token_start + token_len;
            if end < len && (chars[end].is_alphanumeric() || chars[end] == '_') {
                continue;
            }

            // Match legacy sequential: exactly 4 digits
            let is_sequential =
                token_len == 4 && chars[token_start..end].iter().all(|c| c.is_ascii_digit());
            // Match opaque: exactly 8 base36 chars (not all digits)
            let is_opaque = token_len == 8
                && chars[token_start..end]
                    .iter()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                && !chars[token_start..end].iter().all(|c| c.is_ascii_digit());

            if is_sequential || is_opaque {
                let id: String = chars[i..end].iter().collect();
                if !ids.contains(&id) {
                    ids.push(id);
                }
                break;
            }
        }

        i += 1;
    }

    ids
}

/// Check which thread IDs exist as git-forum refs.
///
/// Resolution order (per SPEC-2.0 §6.1.1 / §10.1):
/// 1. Canonical thread ref under `refs/forum/threads/<id>`.
/// 2. Post-migration alias under `refs/forum/aliases/<id>` — covers legacy
///    kind-prefixed IDs (`RFC-0001`, `JOB-e216r3on`, etc.) that were
///    rewritten to bare tokens by `git forum migrate`.
pub fn check_thread_refs(git: &GitOps, ids: &[String]) -> ForumResult<HookCheckResult> {
    let mut found_ids = Vec::new();
    let mut missing_ids = Vec::new();

    for id in ids {
        if git.resolve_ref(&refs::thread_ref(id))?.is_some() {
            found_ids.push(id.clone());
            continue;
        }
        if git.resolve_ref(&super::migrate::alias_ref(id))?.is_some() {
            found_ids.push(id.clone());
            continue;
        }
        missing_ids.push(id.clone());
    }

    Ok(HookCheckResult {
        found_ids,
        missing_ids,
    })
}

// ── hook path resolution ────────────────────────────────────────────

/// Resolve the file path for a named git hook using `git rev-parse --git-path`.
///
/// Works correctly in both normal repos and worktrees.
pub fn resolve_hook_path(git: &GitOps, hook_name: &str) -> ForumResult<std::path::PathBuf> {
    let git_path_arg = format!("hooks/{hook_name}");
    let path_str = git.run(&["rev-parse", "--git-path", &git_path_arg])?;
    let path = Path::new(path_str.trim());
    if path.is_relative() {
        Ok(git.root().join(path))
    } else {
        Ok(path.to_path_buf())
    }
}

// ── generic install / uninstall ─────────────────────────────────────

/// Install a git-forum managed hook.
fn install_hook_generic(
    hook_path: &Path,
    hook_name: &str,
    marker: &str,
    script: &str,
    force: bool,
) -> ForumResult<()> {
    if hook_path.exists() {
        let content = fs::read_to_string(hook_path)?;
        if content.contains(marker) {
            eprintln!("git-forum: {hook_name} hook already installed");
            return Ok(());
        }
        if !force {
            return Err(ForumError::Config(format!(
                "{hook_name} hook already exists; use --force to overwrite"
            )));
        }
    }

    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(hook_path, script)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(hook_path, perms)?;
    }

    eprintln!("git-forum: {hook_name} hook installed");
    Ok(())
}

/// Uninstall a git-forum managed hook (only if it matches the marker).
fn uninstall_hook_generic(hook_path: &Path, hook_name: &str, marker: &str) -> ForumResult<()> {
    if !hook_path.exists() {
        eprintln!("git-forum: no {hook_name} hook installed");
        return Ok(());
    }

    let content = fs::read_to_string(hook_path)?;
    if !content.contains(marker) {
        return Err(ForumError::Config(format!(
            "{hook_name} hook was not installed by git-forum; refusing to remove"
        )));
    }

    fs::remove_file(hook_path)?;
    eprintln!("git-forum: {hook_name} hook removed");
    Ok(())
}

// ── public install / uninstall per hook type ────────────────────────

/// Install the commit-msg hook.
pub fn install_commit_msg_hook(hook_path: &Path, force: bool) -> ForumResult<()> {
    install_hook_generic(
        hook_path,
        "commit-msg",
        COMMIT_MSG_HOOK_MARKER,
        COMMIT_MSG_HOOK_SCRIPT,
        force,
    )
}

/// Uninstall the commit-msg hook.
pub fn uninstall_commit_msg_hook(hook_path: &Path) -> ForumResult<()> {
    uninstall_hook_generic(hook_path, "commit-msg", COMMIT_MSG_HOOK_MARKER)
}

/// Install the post-checkout hook.
pub fn install_post_checkout_hook(hook_path: &Path, force: bool) -> ForumResult<()> {
    install_hook_generic(
        hook_path,
        "post-checkout",
        POST_CHECKOUT_HOOK_MARKER,
        POST_CHECKOUT_HOOK_SCRIPT,
        force,
    )
}

/// Uninstall the post-checkout hook.
pub fn uninstall_post_checkout_hook(hook_path: &Path) -> ForumResult<()> {
    uninstall_hook_generic(hook_path, "post-checkout", POST_CHECKOUT_HOOK_MARKER)
}

/// Install all git-forum hooks (commit-msg + post-checkout).
pub fn install_all_hooks(git: &GitOps, force: bool) -> ForumResult<()> {
    let commit_msg_path = resolve_hook_path(git, "commit-msg")?;
    install_commit_msg_hook(&commit_msg_path, force)?;

    let post_checkout_path = resolve_hook_path(git, "post-checkout")?;
    install_post_checkout_hook(&post_checkout_path, force)?;

    Ok(())
}

/// Uninstall all git-forum hooks.
pub fn uninstall_all_hooks(git: &GitOps) -> ForumResult<()> {
    let commit_msg_path = resolve_hook_path(git, "commit-msg")?;
    uninstall_commit_msg_hook(&commit_msg_path)?;

    let post_checkout_path = resolve_hook_path(git, "post-checkout")?;
    uninstall_post_checkout_hook(&post_checkout_path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_no_ids() {
        assert!(extract_thread_ids("fix typo in README").is_empty());
    }

    #[test]
    fn extract_single_issue() {
        assert_eq!(extract_thread_ids("fix ISSUE-0001 bug"), vec!["ISSUE-0001"]);
    }

    #[test]
    fn extract_single_rfc() {
        assert_eq!(
            extract_thread_ids("implement RFC-0042 design"),
            vec!["RFC-0042"]
        );
    }

    #[test]
    fn extract_multiple_ids() {
        assert_eq!(
            extract_thread_ids("address RFC-0001, closes ISSUE-0042"),
            vec!["RFC-0001", "ISSUE-0042"]
        );
    }

    #[test]
    fn extract_dedup() {
        assert_eq!(
            extract_thread_ids("ISSUE-0001 and ISSUE-0001 again"),
            vec!["ISSUE-0001"]
        );
    }

    #[test]
    fn extract_at_start_and_end() {
        assert_eq!(extract_thread_ids("ISSUE-0001"), vec!["ISSUE-0001"]);
        assert_eq!(extract_thread_ids("fix for RFC-0001"), vec!["RFC-0001"]);
    }

    #[test]
    fn extract_ignores_unknown_prefix() {
        assert!(extract_thread_ids("fix JIRA-1234 ticket").is_empty());
        assert!(extract_thread_ids("see PROJ-0001").is_empty());
    }

    #[test]
    fn extract_ignores_wrong_digit_count() {
        assert!(extract_thread_ids("ISSUE-001 too few").is_empty());
        assert!(extract_thread_ids("ISSUE-00001 too many").is_empty());
    }

    #[test]
    fn extract_respects_word_boundary() {
        assert!(extract_thread_ids("XISSUE-0001 not a match").is_empty());
        assert!(extract_thread_ids("ISSUE-0001x not a match").is_empty());
    }

    #[test]
    fn extract_id_after_punctuation() {
        assert_eq!(extract_thread_ids("(ISSUE-0001)"), vec!["ISSUE-0001"]);
        assert_eq!(extract_thread_ids("[RFC-0001]"), vec!["RFC-0001"]);
    }

    #[test]
    fn extract_id_on_newline() {
        assert_eq!(
            extract_thread_ids("subject line\n\nISSUE-0001"),
            vec!["ISSUE-0001"]
        );
    }

    // Tests for opaque content-addressed IDs
    #[test]
    fn extract_opaque_id() {
        assert_eq!(
            extract_thread_ids("implement RFC-a7f3b2x1 design"),
            vec!["RFC-a7f3b2x1"]
        );
    }

    #[test]
    fn extract_opaque_ask_id() {
        assert_eq!(
            extract_thread_ids("fix ASK-0a1b2c3d bug"),
            vec!["ASK-0a1b2c3d"]
        );
    }

    #[test]
    fn extract_mixed_legacy_and_opaque() {
        assert_eq!(
            extract_thread_ids("RFC-0001 and RFC-a7f3b2x1"),
            vec!["RFC-0001", "RFC-a7f3b2x1"]
        );
    }

    #[test]
    fn extract_opaque_respects_word_boundary() {
        assert!(extract_thread_ids("XRFC-a7f3b2x1").is_empty());
        assert!(extract_thread_ids("RFC-a7f3b2x1z").is_empty());
    }

    #[test]
    fn extract_opaque_rejects_all_digits() {
        // 8 digits is neither sequential (4 digits) nor opaque (must have letters)
        assert!(extract_thread_ids("RFC-12345678").is_empty());
    }

    #[test]
    fn extract_opaque_after_punctuation() {
        assert_eq!(extract_thread_ids("(JOB-x8n2q1d4)"), vec!["JOB-x8n2q1d4"]);
    }

    // SPEC-2.0 §6.1 `@<token>` display form.
    #[test]
    fn extract_at_marker_form() {
        assert_eq!(
            extract_thread_ids("see @e216r3on for details"),
            vec!["e216r3on"]
        );
    }

    #[test]
    fn extract_at_marker_in_parens() {
        assert_eq!(extract_thread_ids("(@a7f3b2x1)"), vec!["a7f3b2x1"]);
    }

    #[test]
    fn extract_at_marker_dedup_and_mixed_with_legacy() {
        // @e216r3on and the legacy alias JOB-e216r3on both surface; the bare
        // form is deduplicated separately because the alias path resolves
        // it via `migrate::alias_ref`.
        let out = extract_thread_ids("Closes JOB-e216r3on (aka @e216r3on)");
        assert_eq!(out, vec!["JOB-e216r3on", "e216r3on"]);
    }

    #[test]
    fn extract_at_marker_word_boundary() {
        // No bare bare-letters before/after the token.
        assert!(extract_thread_ids("foo@e216r3on").is_empty());
        assert!(extract_thread_ids("@e216r3on0extra").is_empty());
    }

    #[test]
    fn extract_at_marker_rejects_short_or_all_digits() {
        // Too short.
        assert!(extract_thread_ids("@a7f3").is_empty());
        // All-digit token is reserved by the bare-token grammar (id_alloc).
        assert!(extract_thread_ids("@12345678").is_empty());
        // Uppercase rejected.
        assert!(extract_thread_ids("@E216R3ON").is_empty());
    }

    #[test]
    fn strip_comments_default() {
        let msg = "fix bug\n# This is a comment\nISSUE-0001";
        assert_eq!(strip_comments(msg, '#'), "fix bug\nISSUE-0001");
    }

    #[test]
    fn strip_comments_scissors() {
        let msg = "fix bug\nISSUE-0001\n# --- >8 ---\ndiff --git a/foo";
        assert_eq!(strip_comments(msg, '#'), "fix bug\nISSUE-0001");
    }

    #[test]
    fn strip_comments_custom_char() {
        let msg = "fix bug\n; This is a comment\nISSUE-0001";
        assert_eq!(strip_comments(msg, ';'), "fix bug\nISSUE-0001");
    }

    #[test]
    fn strip_comments_preserves_non_comment_hash() {
        let msg = "fix #123 issue\nISSUE-0001";
        assert_eq!(strip_comments(msg, '#'), "fix #123 issue\nISSUE-0001");
    }

    // ── hook install / uninstall tests ──────────────────────────────

    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn make_hook_dir() -> TempDir {
        TempDir::new().expect("create temp dir")
    }

    #[test]
    fn install_commit_msg_hook_creates_executable() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("commit-msg");
        install_commit_msg_hook(&hook_path, false).unwrap();
        assert!(hook_path.exists());
        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(COMMIT_MSG_HOOK_MARKER));
        let perms = fs::metadata(&hook_path).unwrap().permissions();
        assert_ne!(perms.mode() & 0o111, 0, "hook must be executable");
    }

    #[test]
    fn install_post_checkout_hook_creates_executable() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("post-checkout");
        install_post_checkout_hook(&hook_path, false).unwrap();
        assert!(hook_path.exists());
        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(POST_CHECKOUT_HOOK_MARKER));
        assert!(content.contains("git-forum hook worktree-init"));
    }

    #[test]
    fn install_hook_refuses_overwrite_without_force() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("post-checkout");
        fs::write(&hook_path, "#!/bin/sh\necho existing").unwrap();
        let result = install_post_checkout_hook(&hook_path, false);
        assert!(result.is_err());
    }

    #[test]
    fn install_hook_overwrites_with_force() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("post-checkout");
        fs::write(&hook_path, "#!/bin/sh\necho existing").unwrap();
        install_post_checkout_hook(&hook_path, true).unwrap();
        let content = fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(POST_CHECKOUT_HOOK_MARKER));
    }

    #[test]
    fn install_hook_is_idempotent() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("commit-msg");
        install_commit_msg_hook(&hook_path, false).unwrap();
        // Second install should succeed (already installed)
        install_commit_msg_hook(&hook_path, false).unwrap();
    }

    #[test]
    fn uninstall_commit_msg_hook_removes_file() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("commit-msg");
        install_commit_msg_hook(&hook_path, false).unwrap();
        uninstall_commit_msg_hook(&hook_path).unwrap();
        assert!(!hook_path.exists());
    }

    #[test]
    fn uninstall_refuses_foreign_hook() {
        let dir = make_hook_dir();
        let hook_path = dir.path().join("commit-msg");
        fs::write(&hook_path, "#!/bin/sh\necho foreign").unwrap();
        let result = uninstall_commit_msg_hook(&hook_path);
        assert!(result.is_err());
        assert!(hook_path.exists(), "foreign hook must not be deleted");
    }

    // ── fix_index_blobs tests ───────────────────────────────────────

    /// Create a git Command with all GIT_* env vars removed so tests
    /// work correctly when invoked from pre-commit hooks.
    fn git_cmd(dir: &Path) -> std::process::Command {
        let mut cmd = std::process::Command::new("git");
        cmd.current_dir(dir);
        for (key, _) in std::env::vars() {
            if key.starts_with("GIT_") {
                cmd.env_remove(&key);
            }
        }
        cmd
    }

    fn init_test_repo() -> (TempDir, GitOps) {
        let dir = TempDir::new().expect("create temp dir");
        let root = dir.path().to_path_buf();
        git_cmd(&root)
            .args(["init"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git init");
        git_cmd(&root)
            .args(["config", "user.name", "Test"])
            .output()
            .expect("config name");
        git_cmd(&root)
            .args(["config", "user.email", "test@test.com"])
            .output()
            .expect("config email");
        let git = GitOps::new(root);
        (dir, git)
    }

    #[test]
    fn fix_index_no_staged_files() {
        let (_dir, git) = init_test_repo();
        let result = fix_index_blobs(&git).unwrap();
        assert!(result.fixed.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn fix_index_healthy_repo() {
        let (dir, git) = init_test_repo();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "hello").unwrap();
        git_cmd(dir.path())
            .args(["add", "hello.txt"])
            .output()
            .expect("git add");
        let result = fix_index_blobs(&git).unwrap();
        assert!(result.fixed.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn fix_index_repairs_missing_blob() {
        let (dir, git) = init_test_repo();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "hello").unwrap();
        git_cmd(dir.path())
            .args(["add", "hello.txt"])
            .output()
            .expect("git add");

        // Get the blob SHA for hello.txt
        let output = git_cmd(dir.path())
            .args(["ls-files", "--stage", "hello.txt"])
            .output()
            .expect("ls-files");
        let ls_line = String::from_utf8_lossy(&output.stdout);
        let sha: &str = ls_line.split_whitespace().nth(1).unwrap();

        // Delete the blob object file to simulate a missing blob
        let obj_dir = dir.path().join(".git/objects").join(&sha[..2]);
        let obj_file = obj_dir.join(&sha[2..]);
        assert!(
            obj_file.exists(),
            "blob object file must exist before deletion"
        );
        fs::remove_file(&obj_file).unwrap();

        // fix_index_blobs should detect and repair
        let result = fix_index_blobs(&git).unwrap();
        assert_eq!(result.fixed.len(), 1);
        assert_eq!(result.fixed[0].0, "hello.txt");
        assert!(result.warnings.is_empty());

        // Verify the blob is now accessible again
        assert!(
            git.run(&["cat-file", "-e", sha]).is_ok() || {
                // SHA may have changed after re-hash; just verify git status is clean
                let status = git.run(&["status", "--porcelain"]).unwrap_or_default();
                !status.contains("hello.txt")
            }
        );
    }

    #[test]
    fn fix_index_repairs_missing_head_tree_blob() {
        // The pre-commit framework's startup probe (`git diff
        // --diff-filter=A --name-only -z`) crashes if HEAD's tree references
        // a pruned blob, killing the commit before any user hook can repair
        // it. fix_index_blobs handles this by re-staging the affected path
        // so the next commit lands a fresh blob.
        let (dir, git) = init_test_repo();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "v1").unwrap();
        git_cmd(dir.path())
            .args(["add", "hello.txt"])
            .output()
            .expect("git add");
        git_cmd(dir.path())
            .args(["commit", "-m", "v1"])
            .output()
            .expect("git commit");

        let output = git_cmd(dir.path())
            .args(["ls-tree", "-r", "HEAD"])
            .output()
            .expect("ls-tree");
        let ls_line = String::from_utf8_lossy(&output.stdout);
        let head_sha: &str = ls_line.split_whitespace().nth(2).unwrap();

        // Prune the blob from HEAD's tree (working file unchanged).
        let obj_file = dir
            .path()
            .join(".git/objects")
            .join(&head_sha[..2])
            .join(&head_sha[2..]);
        fs::remove_file(&obj_file).unwrap();

        // Confirm the corruption: ls-tree against HEAD now fails to read the blob.
        assert!(git.run(&["cat-file", "-e", head_sha]).is_err());

        let result = fix_index_blobs(&git).unwrap();
        assert_eq!(result.fixed.len(), 1, "expected one HEAD-tree repair");
        assert_eq!(result.fixed[0].0, "hello.txt");
        assert!(result.warnings.is_empty());

        // The repair stages a fresh blob so a follow-up commit will heal HEAD.
        let staged = git.run(&["ls-files", "--stage", "hello.txt"]).unwrap();
        let staged_sha: &str = staged.split_whitespace().nth(1).unwrap();
        assert!(
            git.run(&["cat-file", "-e", staged_sha]).is_ok(),
            "staged blob must exist after repair"
        );
    }
}
