//! Import GitHub issues into git-forum threads.
//!
//! Preconditions: `gh` CLI is installed and authenticated; git-forum is initialized.
//! Postconditions: new Issue threads created with nodes, evidence, and state.
//! Failure modes: ForumError::Git on `gh` subprocess failure; ForumError::Repo on dedup conflict.
//! Side effects: writes git objects, updates refs, network calls via `gh`.

use super::clock::Clock;
use super::create;
use super::error::{ForumError, ForumResult};
use super::event::{Event, EventType, NodeType, ThreadKind};
use super::evidence::{Evidence, EvidenceKind};
use super::git_ops::GitOps;
use super::github::{self, GhIssue};
use super::policy::Policy;
use super::say;
use super::state_change::{self, StateChangeOptions};
use super::thread;

/// Result of importing a single GitHub issue.
pub struct ImportResult {
    pub thread_id: String,
    pub github_url: String,
    pub comments_imported: usize,
    pub state_changed: bool,
}

/// What would be created (for --dry-run).
pub struct ImportPlan {
    pub title: String,
    pub body: Option<String>,
    pub github_url: String,
    pub comment_count: usize,
    pub would_close: bool,
    pub already_imported: Option<String>,
}

/// Check if a GitHub issue URL already has a matching thread.
///
/// Scans all threads' evidence_items for an External evidence with a matching URL.
pub fn find_existing_import(git: &GitOps, github_url: &str) -> ForumResult<Option<String>> {
    let thread_ids = thread::list_thread_ids(git)?;
    for id in &thread_ids {
        let state = thread::replay_thread(git, id)?;
        for ev in &state.evidence_items {
            if ev.kind == EvidenceKind::External && ev.ref_target == github_url {
                return Ok(Some(id.clone()));
            }
        }
    }
    Ok(None)
}

/// Build the issue body with metadata (labels, assignees, milestone) prepended.
fn format_issue_body(issue: &GhIssue) -> String {
    let mut meta = String::new();
    if !issue.labels.is_empty() {
        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        meta.push_str(&format!("**Labels:** {}\n", labels.join(", ")));
    }
    if !issue.assignees.is_empty() {
        let assignees: Vec<&str> = issue.assignees.iter().map(|a| a.login.as_str()).collect();
        meta.push_str(&format!("**Assignees:** {}\n", assignees.join(", ")));
    }
    if let Some(milestone) = &issue.milestone {
        meta.push_str(&format!("**Milestone:** {}\n", milestone.title));
    }

    let issue_body = issue.body.as_deref().unwrap_or("");
    if meta.is_empty() {
        return issue_body.to_string();
    }
    if issue_body.is_empty() {
        return meta;
    }
    format!("{meta}\n{issue_body}")
}

/// Plan an import (for dry-run).
pub fn plan_import(git: &GitOps, repo: &str, issue_number: u64) -> ForumResult<ImportPlan> {
    let github_url = format!("https://github.com/{repo}/issues/{issue_number}");
    let already = find_existing_import(git, &github_url)?;
    let issue = github::fetch_issue(repo, issue_number)?;
    let comments = github::fetch_issue_comments(repo, issue_number)?;
    Ok(ImportPlan {
        title: issue.title,
        body: issue.body,
        github_url,
        comment_count: comments.len(),
        would_close: issue.state == "CLOSED",
        already_imported: already,
    })
}

/// Import a single GitHub issue into git-forum.
pub fn import_issue(
    git: &GitOps,
    repo: &str,
    issue_number: u64,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<ImportResult> {
    let github_url = format!("https://github.com/{repo}/issues/{issue_number}");

    // Dedup check
    if let Some(existing_id) = find_existing_import(git, &github_url)? {
        return Err(ForumError::Repo(format!(
            "GitHub issue {github_url} already imported as {existing_id}"
        )));
    }

    // Fetch from GitHub
    let issue = github::fetch_issue(repo, issue_number)?;
    let comments = github::fetch_issue_comments(repo, issue_number)?;

    // 1. Create thread (with original timestamp)
    let body_text = format_issue_body(&issue);
    let body_opt = if body_text.is_empty() {
        None
    } else {
        Some(body_text.as_str())
    };
    let thread_id = create::create_thread_with_timestamp(
        git,
        ThreadKind::Issue,
        &issue.title,
        body_opt,
        None,
        actor,
        clock,
        issue.created_at,
    )?;

    // 2. Add each comment as a Claim node (with original timestamp)
    let mut comments_imported = 0;
    for comment in &comments {
        let body = comment.body.as_deref().unwrap_or("");
        let comment_body = format!(
            "**@{}** ({})\n\n{}",
            comment.user.login,
            comment.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
            body
        );
        say::say_node_with_timestamp(
            git,
            &thread_id,
            NodeType::Claim,
            &comment_body,
            actor,
            clock,
            None,
            comment.created_at,
        )?;
        comments_imported += 1;
    }

    // 4. Add external evidence linking to GitHub
    let evidence = Evidence {
        evidence_id: String::new(),
        kind: EvidenceKind::External,
        ref_target: github_url.clone(),
        locator: None,
    };
    let ev = Event::base(&thread_id, EventType::Link, actor, clock).with_evidence(evidence);
    super::event::write_event(git, &ev)?;

    // 5. If closed, transition state (using default policy to avoid guard conflicts)
    let state_changed = if issue.state == "CLOSED" {
        state_change::change_state(
            git,
            &thread_id,
            "closed",
            &[],
            actor,
            clock,
            &Policy::default(),
            StateChangeOptions::default(),
        )?;
        true
    } else {
        false
    };

    Ok(ImportResult {
        thread_id,
        github_url,
        comments_imported,
        state_changed,
    })
}

/// Import all issues from a repo.
pub fn import_all(
    git: &GitOps,
    repo: &str,
    actor: &str,
    clock: &dyn Clock,
) -> ForumResult<Vec<Result<ImportResult, (u64, ForumError)>>> {
    let issues = github::list_issues(repo)?;
    let mut results = Vec::new();
    for issue in &issues {
        match import_issue(git, repo, issue.number, actor, clock) {
            Ok(result) => results.push(Ok(result)),
            Err(e) => results.push(Err((issue.number, e))),
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::github::{GhAssignee, GhAuthor, GhLabel, GhMilestone};
    use chrono::TimeZone;

    fn make_issue(labels: Vec<&str>, assignees: Vec<&str>, milestone: Option<&str>) -> GhIssue {
        GhIssue {
            number: 1,
            title: "Test".into(),
            body: Some("Issue body".into()),
            state: "OPEN".into(),
            created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            author: GhAuthor {
                login: "alice".into(),
            },
            labels: labels
                .into_iter()
                .map(|n| GhLabel { name: n.into() })
                .collect(),
            assignees: assignees
                .into_iter()
                .map(|l| GhAssignee { login: l.into() })
                .collect(),
            milestone: milestone.map(|t| GhMilestone { title: t.into() }),
        }
    }

    #[test]
    fn format_body_plain() {
        let issue = make_issue(vec![], vec![], None);
        assert_eq!(format_issue_body(&issue), "Issue body");
    }

    #[test]
    fn format_body_with_labels() {
        let issue = make_issue(vec!["bug", "urgent"], vec![], None);
        let body = format_issue_body(&issue);
        assert!(body.starts_with("**Labels:** bug, urgent\n"));
        assert!(body.contains("Issue body"));
    }

    #[test]
    fn format_body_with_all_metadata() {
        let issue = make_issue(vec!["bug"], vec!["alice", "bob"], Some("v1.0"));
        let body = format_issue_body(&issue);
        assert!(body.contains("**Labels:** bug"));
        assert!(body.contains("**Assignees:** alice, bob"));
        assert!(body.contains("**Milestone:** v1.0"));
        assert!(body.contains("Issue body"));
    }

    #[test]
    fn format_body_no_body() {
        let mut issue = make_issue(vec!["bug"], vec![], None);
        issue.body = None;
        let body = format_issue_body(&issue);
        assert_eq!(body, "**Labels:** bug\n");
    }
}
