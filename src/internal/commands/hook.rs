//! Git hook support for git-forum.
//!
//! Provides:
//! - A commit-msg hook that validates the thread IDs on `Refs:` trailers
//!   (SPEC-3.0 §2.5 (3), ADR-013).
//! - A post-checkout hook that initializes git-forum in fresh worktrees.
//! - A `fix-index` subcommand that detects and re-hashes missing blobs
//!   (manual recovery; also invoked by `git-forum doctor`).
//!
//! task `1hg98odf`: the `Hook::*` arm body relocates
//! from `main.rs` to [`run_arm`] in this module. The lower-level
//! installer / scanner functions stay here as the hook subsystem
//! library; the new entry-point dispatches across the four sub-arms.

use std::fs;
use std::path::Path;

use crate::internal::actor;
use crate::internal::config::RepoPaths;
use crate::internal::id_alloc;

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
    let ids = read_refs_trailer_ids(git, &cleaned)?;
    if ids.is_empty() {
        // The suggestion is best-effort: a failure to list threads must
        // not turn an advisory warning into a refused commit.
        let suggested = suggest_refs(git, &cleaned).unwrap_or_default();
        eprint!("{}", render_no_refs_warning(&suggested));
        return Ok(());
    }
    let result = check_thread_refs(git, &ids)?;
    if result.has_errors() {
        eprintln!("git-forum: commit message references non-existent thread(s):");
        for id in &result.missing_ids {
            eprintln!("  {id} — not found");
        }
        eprintln!("hint: create the thread first, or remove it from the `Refs:` trailer.");
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
    // Per task `96u6zxmc`: worktree-init writes only .git/forum/ local state.
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
    let _ = init::ensure_forum_refspecs(git, init::InitMode::default());
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

/// Extract thread IDs from the `Refs:` trailers of a commit message
/// (SPEC-3.0 §2.5 (3), ADR-013).
///
/// `parsed_trailers` is the output of `git interpret-trailers --parse`:
/// one `Key: value` line per trailer, continuation lines already folded.
/// The key matches case-insensitively. A value may list several IDs
/// separated by commas or whitespace, each optionally written with a
/// leading `@`, which is stripped. Tokens without thread-ID shape (`#123`,
/// URLs) are ignored. Returns deduplicated IDs in trailer order.
pub fn refs_trailer_ids(parsed_trailers: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for line in parsed_trailers.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("refs") {
            continue;
        }
        for token in value.split(|c: char| c == ',' || c.is_whitespace()) {
            let token = token.strip_prefix('@').unwrap_or(token);
            if id_alloc::is_valid_thread_id(token) && !ids.iter().any(|id| id == token) {
                ids.push(token.to_string());
            }
        }
    }
    ids
}

/// Read the `Refs:` trailer IDs of a commit message through Git's own
/// trailer parser, so "trailer" means exactly what it means to Git.
fn read_refs_trailer_ids(git: &GitOps, message: &str) -> ForumResult<Vec<String>> {
    let parsed = git.run_with_stdin(
        &["interpret-trailers", "--parse", "--no-divider"],
        message.as_bytes(),
    )?;
    Ok(refs_trailer_ids(&parsed))
}

/// Thread-ID-shaped tokens anywhere in a commit message, deduplicated in
/// order. Only feeds the suggestion in the no-trailer warning; these are
/// never treated as references (SPEC-3.0 §2.5 (3)).
pub fn id_shaped_tokens(message: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        if id_alloc::is_valid_thread_id(s) && !ids.iter().any(|id| id == s) {
            ids.push(s.to_string());
        }
    };
    for word in message.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if id_alloc::is_valid_thread_id(word) {
            push(word);
        } else {
            word.split('-').for_each(&mut push);
        }
    }
    ids
}

/// Existing threads named by ID-shaped tokens elsewhere in the message —
/// the values the no-trailer warning offers for the `Refs:` line. Lists the
/// thread refs once instead of resolving every English 8-letter word.
fn suggest_refs(git: &GitOps, message: &str) -> ForumResult<Vec<String>> {
    let tokens = id_shaped_tokens(message);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }
    let known = crate::internal::thread::list_thread_ids(git)?;
    let mut found = Vec::new();
    for token in tokens {
        let exists = if token.contains('-') {
            git.resolve_ref(&super::migrate::alias_ref(&token))?
                .is_some()
        } else {
            known.contains(&token)
        };
        if exists {
            found.push(token);
        }
    }
    Ok(found)
}

/// The warning for a commit message whose trailers name no thread
/// (ADR-013). The first line is unchanged from earlier releases; the rest
/// shows the form that is read and how to attach the commit afterwards.
/// `suggested` fills in the IDs when the message names threads elsewhere.
pub fn render_no_refs_warning(suggested: &[String]) -> String {
    let mut out = String::from("git-forum: warning: no thread ID referenced in commit message\n");
    out.push_str("  expected a trailer as the last paragraph of the message:\n");
    if suggested.is_empty() {
        out.push_str("    Refs: <thread-id>\n");
        out.push_str("  (subject tags such as [<thread-id>] and IDs in body text are not read)\n");
    } else {
        let listed = suggested.join(", ");
        out.push_str(&format!("    Refs: {listed}\n"));
        out.push_str(&format!(
            "  ({listed} appears in the subject or body, which are not read)\n"
        ));
    }
    let id = match suggested {
        [only] => only.as_str(),
        _ => "<thread-id>",
    };
    out.push_str("  to attach this commit as evidence after committing:\n");
    out.push_str(&format!(
        "    git forum evidence add {id} --kind commit --ref HEAD\n"
    ));
    out
}

/// Check which thread IDs exist as git-forum refs.
///
/// Resolution order (per SPEC-3.0 §2.5 (3), §4.1):
/// 1. Canonical thread ref under `refs/forum/threads/<id>`.
/// 2. Published mirror under `refs/forum/published/<id>` — the only ref a
///    public-only clone has for a thread.
/// 3. Post-migration alias under `refs/forum/aliases/<id>` — covers legacy
///    kind-prefixed IDs (`RFC-0001`, `JOB-e216r3on`, etc.) that were
///    rewritten to bare tokens by `git forum migrate`.
pub fn check_thread_refs(git: &GitOps, ids: &[String]) -> ForumResult<HookCheckResult> {
    let mut found_ids = Vec::new();
    let mut missing_ids = Vec::new();

    for id in ids {
        if git.resolve_ref(&refs::thread_ref(id))?.is_some()
            || git.resolve_ref(&refs::published_ref(id))?.is_some()
            || git.resolve_ref(&super::migrate::alias_ref(id))?.is_some()
        {
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

    // ── refs_trailer_ids (input: `git interpret-trailers --parse` output) ──

    #[test]
    fn trailer_bare_id() {
        assert_eq!(refs_trailer_ids("Refs: a7f3b2x1"), vec!["a7f3b2x1"]);
    }

    #[test]
    fn trailer_at_marker_is_stripped() {
        assert_eq!(refs_trailer_ids("Refs: @a7f3b2x1"), vec!["a7f3b2x1"]);
    }

    #[test]
    fn trailer_list_and_repeated_keys() {
        assert_eq!(
            refs_trailer_ids("Refs: a7f3b2x1, @e216r3on\nRefs: q59k5a38 a7f3b2x1"),
            vec!["a7f3b2x1", "e216r3on", "q59k5a38"]
        );
    }

    #[test]
    fn trailer_key_is_case_insensitive() {
        assert_eq!(refs_trailer_ids("refs: a7f3b2x1"), vec!["a7f3b2x1"]);
        assert_eq!(refs_trailer_ids("REFS : a7f3b2x1"), vec!["a7f3b2x1"]);
    }

    #[test]
    fn trailer_other_keys_are_ignored() {
        assert!(refs_trailer_ids("Signed-off-by: a7f3b2x1 <a@example.com>").is_empty());
        assert!(refs_trailer_ids("Closes: a7f3b2x1").is_empty());
    }

    #[test]
    fn trailer_non_id_tokens_are_ignored() {
        // Another tracker's convention on the same key.
        assert!(refs_trailer_ids("Refs: #123").is_empty());
        assert!(refs_trailer_ids("Refs: https://example.com/x").is_empty());
        // Wrong length, all digits, uppercase.
        assert!(refs_trailer_ids("Refs: a7f3 a7f3b2x1z 12345678 A7F3B2X1").is_empty());
        assert_eq!(refs_trailer_ids("Refs: #123, a7f3b2x1"), vec!["a7f3b2x1"]);
    }

    #[test]
    fn trailer_legacy_ids() {
        assert_eq!(
            refs_trailer_ids("Refs: RFC-0001, ASK-a7f3b2x1"),
            vec!["RFC-0001", "ASK-a7f3b2x1"]
        );
    }

    // ── id_shaped_tokens (warning suggestion only) ──

    #[test]
    fn shaped_tokens_subject_tag_and_marker() {
        assert_eq!(
            id_shaped_tokens("docs: foo [a7f3b2x1] see @e216r3on"),
            vec!["a7f3b2x1", "e216r3on"]
        );
    }

    #[test]
    fn shaped_tokens_path_and_hyphen_suffix() {
        assert_eq!(
            id_shaped_tokens("branch issue/a7f3b2x1, and e216r3on-fix"),
            vec!["a7f3b2x1", "e216r3on"]
        );
    }

    #[test]
    fn shaped_tokens_legacy_and_rejects() {
        assert_eq!(id_shaped_tokens("fix RFC-0001"), vec!["RFC-0001"]);
        assert!(id_shaped_tokens("12345678 a7f3 a7f3b2x1z").is_empty());
    }

    // ── render_no_refs_warning ──

    #[test]
    fn warning_keeps_first_line_and_shows_placeholder() {
        let w = render_no_refs_warning(&[]);
        assert!(w.starts_with("git-forum: warning: no thread ID referenced in commit message\n"));
        assert!(w.contains("    Refs: <thread-id>\n"));
        assert!(w.contains("git forum evidence add <thread-id> --kind commit --ref HEAD"));
    }

    #[test]
    fn warning_fills_in_single_suggestion() {
        let w = render_no_refs_warning(&["a7f3b2x1".to_string()]);
        assert!(w.contains("    Refs: a7f3b2x1\n"));
        assert!(w.contains("git forum evidence add a7f3b2x1 --kind commit --ref HEAD"));
    }

    #[test]
    fn warning_lists_several_suggestions_without_picking_one() {
        let w = render_no_refs_warning(&["a7f3b2x1".to_string(), "e216r3on".to_string()]);
        assert!(w.contains("    Refs: a7f3b2x1, e216r3on\n"));
        assert!(w.contains("git forum evidence add <thread-id> --kind commit --ref HEAD"));
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
