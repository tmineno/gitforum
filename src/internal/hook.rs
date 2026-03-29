//! Git hook support for git-forum.
//!
//! Provides a commit-msg hook that validates thread ID references
//! in commit messages against existing git-forum threads.

use std::fs;
use std::path::Path;

use super::error::{ForumError, ForumResult};
use super::git_ops::GitOps;
use super::refs;

const HOOK_MARKER: &str = "# git-forum advisory commit-msg hook";

const HOOK_SCRIPT: &str = r#"#!/bin/sh
# git-forum advisory commit-msg hook
git-forum hook check-commit-msg "$1"
"#;

/// Known thread ID prefixes (must match ThreadKind::id_prefix values).
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
/// Matches `ISSUE-NNNN` and `RFC-NNNN` patterns with word boundaries.
/// Returns deduplicated results.
pub fn extract_thread_ids(message: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let chars: Vec<char> = message.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Check word boundary: start of string or previous char is not alphanumeric
        if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
            i += 1;
            continue;
        }

        for prefix in KNOWN_PREFIXES {
            let prefix_chars: Vec<char> = prefix.chars().collect();
            let prefix_len = prefix_chars.len();

            // Check if prefix matches at position i
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

            // Check for exactly 4 digits
            let digit_start = i + prefix_len + 1;
            let mut digit_count = 0;
            while digit_start + digit_count < len
                && chars[digit_start + digit_count].is_ascii_digit()
            {
                digit_count += 1;
            }
            if digit_count != 4 {
                continue;
            }

            // Check trailing word boundary
            let end = digit_start + 4;
            if end < len && (chars[end].is_alphanumeric() || chars[end] == '_') {
                continue;
            }

            let id: String = chars[i..end].iter().collect();
            if !ids.contains(&id) {
                ids.push(id);
            }
            break;
        }

        i += 1;
    }

    ids
}

/// Check which thread IDs exist as git-forum refs.
pub fn check_thread_refs(git: &GitOps, ids: &[String]) -> ForumResult<HookCheckResult> {
    let mut found_ids = Vec::new();
    let mut missing_ids = Vec::new();

    for id in ids {
        let refname = refs::thread_ref(id);
        match git.resolve_ref(&refname)? {
            Some(_) => found_ids.push(id.clone()),
            None => missing_ids.push(id.clone()),
        }
    }

    Ok(HookCheckResult {
        found_ids,
        missing_ids,
    })
}

/// Resolve the hook file path using `git rev-parse --git-path`.
pub fn resolve_hook_path(git: &GitOps) -> ForumResult<std::path::PathBuf> {
    let path_str = git.run(&["rev-parse", "--git-path", "hooks/commit-msg"])?;
    let path = Path::new(path_str.trim());
    if path.is_relative() {
        Ok(git.root().join(path))
    } else {
        Ok(path.to_path_buf())
    }
}

/// Install the commit-msg hook.
pub fn install_hook(hook_path: &Path, force: bool) -> ForumResult<()> {
    if hook_path.exists() {
        let content = fs::read_to_string(hook_path)?;
        if content.contains(HOOK_MARKER) {
            eprintln!("git-forum: commit-msg hook already installed");
            return Ok(());
        }
        if !force {
            return Err(ForumError::Config(
                "commit-msg hook already exists; use --force to overwrite".into(),
            ));
        }
    }

    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(hook_path, HOOK_SCRIPT)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        fs::set_permissions(hook_path, perms)?;
    }

    eprintln!("git-forum: commit-msg hook installed");
    Ok(())
}

/// Uninstall the commit-msg hook (only if it matches the git-forum template).
pub fn uninstall_hook(hook_path: &Path) -> ForumResult<()> {
    if !hook_path.exists() {
        eprintln!("git-forum: no commit-msg hook installed");
        return Ok(());
    }

    let content = fs::read_to_string(hook_path)?;
    if !content.contains(HOOK_MARKER) {
        return Err(ForumError::Config(
            "commit-msg hook was not installed by git-forum; refusing to remove".into(),
        ));
    }

    fs::remove_file(hook_path)?;
    eprintln!("git-forum: commit-msg hook removed");
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
}
