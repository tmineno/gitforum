//! GitHub CLI (`gh`) subprocess wrapper for import/export operations.
//!
//! Preconditions: `gh` CLI is installed and authenticated.
//! Failure modes: ForumError::Git if `gh` is not found or returns an error.
//! Side effects: network calls to GitHub API via `gh`.

use super::error::{ForumError, ForumResult};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::process::Command;

/// A GitHub issue as returned by `gh issue view --json`.
#[derive(Debug, Deserialize)]
pub struct GhIssue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String, // "OPEN" or "CLOSED"
    #[serde(rename = "createdAt")]
    pub created_at: DateTime<Utc>,
    pub author: GhAuthor,
    #[serde(default)]
    pub labels: Vec<GhLabel>,
    #[serde(default)]
    pub assignees: Vec<GhAssignee>,
    pub milestone: Option<GhMilestone>,
}

#[derive(Debug, Deserialize)]
pub struct GhAuthor {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct GhLabel {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GhAssignee {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct GhMilestone {
    pub title: String,
}

/// A comment on a GitHub issue (from `gh api`).
#[derive(Debug, Deserialize)]
pub struct GhComment {
    pub id: u64,
    pub body: Option<String>,
    pub user: GhCommentUser,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct GhCommentUser {
    pub login: String,
}

/// An existing comment for re-export marker matching.
#[derive(Debug)]
pub struct GhExistingComment {
    pub id: u64,
    pub body: String,
}

fn run_gh(args: &[&str]) -> ForumResult<Vec<u8>> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| ForumError::Git(format!("failed to run gh: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ForumError::Git(format!(
            "gh {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        )));
    }
    Ok(output.stdout)
}

/// Fetch a single issue by number.
pub fn fetch_issue(repo: &str, issue_number: u64) -> ForumResult<GhIssue> {
    let num = issue_number.to_string();
    let stdout = run_gh(&[
        "issue",
        "view",
        &num,
        "--repo",
        repo,
        "--json",
        "number,title,body,state,createdAt,author,labels,assignees,milestone",
    ])?;
    serde_json::from_slice(&stdout).map_err(ForumError::Json)
}

/// Fetch all comments on an issue via the REST API.
pub fn fetch_issue_comments(repo: &str, issue_number: u64) -> ForumResult<Vec<GhComment>> {
    let endpoint = format!("repos/{repo}/issues/{issue_number}/comments?per_page=100");
    let stdout = run_gh(&["api", &endpoint, "--paginate"])?;
    // gh --paginate concatenates JSON arrays, producing invalid JSON.
    // Parse as a single array first; if that fails, try splitting.
    let text = String::from_utf8_lossy(&stdout);
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(vec![]);
    }
    // Try direct parse first (single page)
    if let Ok(comments) = serde_json::from_str::<Vec<GhComment>>(trimmed) {
        return Ok(comments);
    }
    // Multi-page: gh concatenates arrays like `[...][...]`, split and merge
    let mut all = Vec::new();
    for chunk in split_json_arrays(trimmed) {
        let page: Vec<GhComment> = serde_json::from_str(chunk).map_err(ForumError::Json)?;
        all.extend(page);
    }
    Ok(all)
}

/// List open issues in a repo.
pub fn list_issues(repo: &str) -> ForumResult<Vec<GhIssue>> {
    let stdout = run_gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--state",
        "all",
        "--limit",
        "1000",
        "--json",
        "number,title,body,state,createdAt,author,labels,assignees,milestone",
    ])?;
    serde_json::from_slice(&stdout).map_err(ForumError::Json)
}

/// Create a GitHub issue. Returns the issue number.
pub fn create_issue(repo: &str, title: &str, body: &str) -> ForumResult<u64> {
    let stdout = run_gh(&[
        "issue", "create", "--repo", repo, "--title", title, "--body", body,
    ])?;
    // gh issue create outputs the URL: https://github.com/owner/repo/issues/123
    let url = String::from_utf8_lossy(&stdout);
    let url = url.trim();
    url.rsplit('/')
        .next()
        .and_then(|n| n.parse::<u64>().ok())
        .ok_or_else(|| ForumError::Git(format!("cannot parse issue number from gh output: {url}")))
}

/// Add a comment to a GitHub issue.
pub fn add_comment(repo: &str, issue_number: u64, body: &str) -> ForumResult<()> {
    let num = issue_number.to_string();
    run_gh(&["issue", "comment", &num, "--repo", repo, "--body", body])?;
    Ok(())
}

/// Close a GitHub issue.
pub fn close_issue(repo: &str, issue_number: u64) -> ForumResult<()> {
    let num = issue_number.to_string();
    run_gh(&["issue", "close", &num, "--repo", repo])?;
    Ok(())
}

/// List comments on a GitHub issue (for re-export marker matching).
pub fn list_comments(repo: &str, issue_number: u64) -> ForumResult<Vec<GhExistingComment>> {
    let comments = fetch_issue_comments(repo, issue_number)?;
    Ok(comments
        .into_iter()
        .map(|c| GhExistingComment {
            id: c.id,
            body: c.body.unwrap_or_default(),
        })
        .collect())
}

/// Update an existing GitHub comment body.
pub fn update_comment(repo: &str, comment_id: u64, body: &str) -> ForumResult<()> {
    let endpoint = format!("repos/{repo}/issues/comments/{comment_id}");
    let field = format!("body={body}");
    run_gh(&["api", &endpoint, "--method", "PATCH", "--raw-field", &field])?;
    Ok(())
}

/// Split concatenated JSON arrays (gh --paginate output).
fn split_json_arrays(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0;
    let mut start = None;
    let mut in_string = false;
    let mut prev_backslash = false;

    for (i, ch) in input.char_indices() {
        if in_string {
            if ch == '\\' && !prev_backslash {
                prev_backslash = true;
                continue;
            }
            if ch == '"' && !prev_backslash {
                in_string = false;
            }
            prev_backslash = false;
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        result.push(&input[s..=i]);
                    }
                    start = None;
                }
            }
            _ => {}
        }
        prev_backslash = false;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_json_arrays_single() {
        let input = r#"[{"id":1}]"#;
        let result = split_json_arrays(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], r#"[{"id":1}]"#);
    }

    #[test]
    fn split_json_arrays_multiple() {
        let input = r#"[{"id":1}][{"id":2}]"#;
        let result = split_json_arrays(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], r#"[{"id":1}]"#);
        assert_eq!(result[1], r#"[{"id":2}]"#);
    }

    #[test]
    fn split_json_arrays_with_brackets_in_string() {
        let input = r#"[{"body":"[test]"}]"#;
        let result = split_json_arrays(input);
        assert_eq!(result.len(), 1);
    }
}
