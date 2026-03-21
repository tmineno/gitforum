//! Export git-forum issue threads to GitHub Issues.
//!
//! Preconditions: `gh` CLI is installed and authenticated; git-forum is initialized.
//! Postconditions: GitHub issue created/updated; external evidence stored on thread for dedup.
//! Failure modes: ForumError::Git on `gh` subprocess failure; ForumError::Repo on parse failure.
//! Side effects: creates GitHub issue/comments via `gh`; writes git objects and updates refs.

use super::clock::Clock;
use super::error::{ForumError, ForumResult};
use super::evidence::EvidenceKind;
use super::evidence_ops;
use super::git_ops::GitOps;
use super::github;
use super::node::Node;
use super::thread::{self, ThreadState};

/// Result of exporting a thread to GitHub.
pub struct ExportResult {
    pub github_issue_number: u64,
    pub github_url: String,
    pub comments_created: usize,
    pub comments_updated: usize,
    pub comments_skipped: usize,
    pub was_closed: bool,
}

/// What would be created (for --dry-run).
pub struct ExportPlan {
    pub thread_id: String,
    pub title: String,
    pub body: Option<String>,
    pub node_count: usize,
    pub would_close: bool,
    pub already_exported: bool,
    pub existing_github_url: Option<String>,
}

/// Format a node for GitHub comment body, including the git-forum marker.
pub fn format_node_as_comment(node: &Node) -> String {
    format!(
        "<!-- git-forum:{} -->\n**[{}]** by {}\n\n{}",
        node.node_id, node.node_type, node.actor, node.body
    )
}

/// Extract git-forum node ID from a GitHub comment body.
pub fn extract_marker(comment_body: &str) -> Option<&str> {
    let start = comment_body.find("<!-- git-forum:")?;
    let rest = &comment_body[start + 15..];
    let end = rest.find(" -->")?;
    Some(&rest[..end])
}

/// Check if thread already has an external evidence pointing to a GitHub issue URL.
pub fn find_existing_export(state: &ThreadState) -> Option<String> {
    state
        .evidence_items
        .iter()
        .find(|e| {
            e.kind == EvidenceKind::External
                && e.ref_target.starts_with("https://github.com/")
                && e.ref_target.contains("/issues/")
        })
        .map(|e| e.ref_target.clone())
}

/// Extract repo and issue number from a GitHub issue URL.
pub fn parse_github_issue_url(url: &str) -> Option<(String, u64)> {
    let path = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.splitn(4, '/').collect();
    if parts.len() == 4 && parts[2] == "issues" {
        let number = parts[3].parse::<u64>().ok()?;
        let repo = format!("{}/{}", parts[0], parts[1]);
        Some((repo, number))
    } else {
        None
    }
}

/// Format the GitHub issue body, including a git-forum traceability marker.
fn format_issue_body(state: &ThreadState) -> String {
    let body = state.body.as_deref().unwrap_or("");
    format!(
        "{body}\n\n---\n*Exported from git-forum thread `{}`*",
        state.id
    )
}

/// Plan an export (for dry-run).
pub fn plan_export(git: &GitOps, thread_id: &str) -> ForumResult<ExportPlan> {
    let state = thread::replay_thread(git, thread_id)?;
    let existing = find_existing_export(&state);
    let non_retracted_count = state.nodes.iter().filter(|n| !n.retracted).count();
    let would_close = matches!(state.status.as_str(), "closed" | "rejected");
    Ok(ExportPlan {
        thread_id: thread_id.to_string(),
        title: state.title.clone(),
        body: state.body.clone(),
        node_count: non_retracted_count,
        would_close,
        already_exported: existing.is_some(),
        existing_github_url: existing,
    })
}

/// Export a git-forum thread to GitHub Issues.
pub fn export_issue(
    git: &GitOps,
    thread_id: &str,
    target_repo: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<ExportResult> {
    let state = thread::replay_thread(git, thread_id)?;
    let non_retracted: Vec<&Node> = state.nodes.iter().filter(|n| !n.retracted).collect();
    let would_close = matches!(state.status.as_str(), "closed" | "rejected");

    // Check for existing export
    if let Some(ref url) = find_existing_export(&state) {
        let (repo, issue_number) = parse_github_issue_url(url)
            .ok_or_else(|| ForumError::Repo(format!("cannot parse GitHub URL: {url}")))?;
        if repo != target_repo {
            return Err(ForumError::Repo(format!(
                "thread already exported to {repo}, not {target_repo}"
            )));
        }
        return re_export_issue(
            &state,
            &non_retracted,
            issue_number,
            target_repo,
            would_close,
        );
    }

    // First export
    let body_text = format_issue_body(&state);
    let issue_number = github::create_issue(target_repo, &state.title, &body_text)?;
    let github_url = format!("https://github.com/{target_repo}/issues/{issue_number}");

    // Add each non-retracted node as a comment
    let mut comments_created = 0;
    for node in &non_retracted {
        let comment_body = format_node_as_comment(node);
        github::add_comment(target_repo, issue_number, &comment_body)?;
        comments_created += 1;
    }

    // Close if needed
    if would_close {
        github::close_issue(target_repo, issue_number)?;
    }

    // Store external evidence on thread for dedup
    evidence_ops::add_evidence(
        git,
        thread_id,
        EvidenceKind::External,
        &github_url,
        None,
        actor,
        clock,
    )?;

    Ok(ExportResult {
        github_issue_number: issue_number,
        github_url,
        comments_created,
        comments_updated: 0,
        comments_skipped: 0,
        was_closed: would_close,
    })
}

/// Re-export: match existing comments by marker, skip or update.
fn re_export_issue(
    _state: &ThreadState,
    non_retracted: &[&Node],
    issue_number: u64,
    repo: &str,
    would_close: bool,
) -> ForumResult<ExportResult> {
    let existing_comments = github::list_comments(repo, issue_number)?;

    // Build marker→comment_id map
    let mut marker_map: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for comment in &existing_comments {
        if let Some(node_id) = extract_marker(&comment.body) {
            marker_map.insert(node_id.to_string(), comment.id);
        }
    }

    let mut created = 0;
    let mut updated = 0;
    let mut skipped = 0;

    for node in non_retracted {
        let new_body = format_node_as_comment(node);
        if let Some(&comment_id) = marker_map.get(&node.node_id) {
            let existing = existing_comments.iter().find(|c| c.id == comment_id);
            if let Some(existing) = existing {
                if existing.body == new_body {
                    skipped += 1;
                } else {
                    github::update_comment(repo, comment_id, &new_body)?;
                    updated += 1;
                }
            }
        } else {
            github::add_comment(repo, issue_number, &new_body)?;
            created += 1;
        }
    }

    if would_close {
        github::close_issue(repo, issue_number)?;
    }

    let github_url = format!("https://github.com/{repo}/issues/{issue_number}");
    Ok(ExportResult {
        github_issue_number: issue_number,
        github_url,
        comments_created: created,
        comments_updated: updated,
        comments_skipped: skipped,
        was_closed: would_close,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::event::NodeType;
    use chrono::TimeZone;

    fn make_node(node_id: &str, node_type: NodeType, body: &str) -> Node {
        Node {
            node_id: node_id.to_string(),
            node_type,
            body: body.to_string(),
            actor: "human/alice".to_string(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            resolved: false,
            retracted: false,
            incorporated: false,
            reply_to: None,
        }
    }

    #[test]
    fn format_node_includes_marker() {
        let node = make_node("abc123", NodeType::Claim, "hello world");
        let body = format_node_as_comment(&node);
        assert!(body.starts_with("<!-- git-forum:abc123 -->"));
        assert!(body.contains("**[claim]** by human/alice"));
        assert!(body.contains("hello world"));
    }

    #[test]
    fn extract_marker_roundtrip() {
        let node = make_node("abc123def456", NodeType::Objection, "some text");
        let body = format_node_as_comment(&node);
        let extracted = extract_marker(&body);
        assert_eq!(extracted, Some("abc123def456"));
    }

    #[test]
    fn extract_marker_no_marker() {
        assert_eq!(extract_marker("just a regular comment"), None);
    }

    #[test]
    fn parse_url_valid() {
        let result = parse_github_issue_url("https://github.com/owner/repo/issues/42");
        assert_eq!(result, Some(("owner/repo".to_string(), 42)));
    }

    #[test]
    fn parse_url_invalid() {
        assert_eq!(
            parse_github_issue_url("https://github.com/owner/repo/pulls/1"),
            None
        );
        assert_eq!(parse_github_issue_url("https://example.com/issues/1"), None);
    }
}
