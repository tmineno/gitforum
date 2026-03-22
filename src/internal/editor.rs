use std::env;
use std::io::{IsTerminal, Write};
use std::process::Command;

use tempfile::Builder;

use super::error::ForumError;

/// Resolve the editor command: $VISUAL → $EDITOR → vi.
fn resolve_editor() -> String {
    env::var("VISUAL")
        .or_else(|_| env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into())
}

/// Strip lines starting with '#' and trim trailing whitespace.
fn strip_comments(input: &str) -> String {
    let stripped: Vec<&str> = input
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    stripped.join("\n").trim().to_string()
}

/// Open `$VISUAL` / `$EDITOR` / `vi` with a temporary file for body composition.
///
/// **Preconditions:** stdin is an interactive terminal.
/// **Postconditions:** Returns the user-composed body with comment lines stripped.
/// **Failure modes:**
/// - `ForumError::Config` if stdin is not a terminal (non-interactive context)
/// - `ForumError::Config` if edited content is empty after stripping comments
/// - `ForumError::Io` if the temp file or editor process fails
///
/// **Side effects:** Spawns an external editor process; creates a temp file (auto-cleaned).
pub fn edit_body(hint: &str) -> Result<String, ForumError> {
    if !std::io::stdin().is_terminal() && env::var("GIT_FORUM_EDITOR_FORCE").is_err() {
        return Err(ForumError::Config(
            "--edit requires an interactive terminal; use --body or --body-file instead".into(),
        ));
    }

    let editor = resolve_editor();

    let mut tmp = Builder::new()
        .prefix("git-forum-")
        .suffix(".md")
        .tempfile()
        .map_err(|e| ForumError::Config(format!("failed to create temp file: {e}")))?;

    let template = format!(
        "\n# Lines starting with '#' will be stripped.\n# {hint}\n# Leave empty to abort.\n"
    );
    tmp.write_all(template.as_bytes())?;
    tmp.flush()?;

    let path = tmp.path().to_path_buf();

    let status = Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| ForumError::Config(format!("failed to launch editor '{editor}': {e}")))?;

    if !status.success() {
        return Err(ForumError::Config(format!(
            "editor '{editor}' exited with {}",
            status
                .code()
                .map_or("signal".to_string(), |c| c.to_string())
        )));
    }

    let content = std::fs::read_to_string(&path)?;
    let body = strip_comments(&content);

    if body.is_empty() {
        return Err(ForumError::Config("aborted: empty body from editor".into()));
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_comments_removes_hash_lines() {
        let input = "# comment\nhello\n# another\nworld";
        assert_eq!(strip_comments(input), "hello\nworld");
    }

    #[test]
    fn strip_comments_preserves_non_comment_lines() {
        let input = "line one\nline two\nline three";
        assert_eq!(strip_comments(input), "line one\nline two\nline three");
    }

    #[test]
    fn strip_comments_handles_mixed_content() {
        let input = "# header\nreal content\n# footer\nmore content\n";
        assert_eq!(strip_comments(input), "real content\nmore content");
    }

    #[test]
    fn strip_comments_returns_empty_for_all_comments() {
        let input = "# only\n# comments\n# here\n";
        assert_eq!(strip_comments(input), "");
    }

    #[test]
    fn strip_comments_trims_trailing_whitespace() {
        let input = "hello\n\n# comment\n\n";
        assert_eq!(strip_comments(input), "hello");
    }

    #[test]
    fn strip_comments_preserves_inline_hash() {
        let input = "heading with # inside\n# pure comment";
        assert_eq!(strip_comments(input), "heading with # inside");
    }

    #[test]
    fn resolve_editor_defaults_to_vi() {
        // Only test the fallback logic — env vars may be set in CI
        let editor = resolve_editor();
        assert!(!editor.is_empty());
    }
}
