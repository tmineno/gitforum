//! Git-history view of a snapshot ref (SPEC-3.0 §5.4).
//!
//! Phase 4 Step 1a (RFC `7ymtc4b2`, task `913c4s9v`): replaces the v2
//! domain-event timeline with a Git-log walk over `refs/forum/threads/<id>`.
//! Each commit's subject is parsed against the operation-shaped messages
//! from SPEC-3.0 §5.3 (`thread-create`, `node-add`, `node-resolve`,
//! `state`, `link-add`); subjects that don't match are still surfaced
//! verbatim per §5.4 ("Commits whose messages are not recognized MUST
//! still be shown as Git commits").
//!
//! Consumers: `commands::show::render_full` / `render_node_show`
//! (this commit), and `tui::render` per-thread timeline (Phase 4 Step 1d).
//! Per-node history (`render_node_show`) filters the same entry list to
//! commits whose tree changed `nodes/<id>.{toml,md}`.

use chrono::{DateTime, Utc};

use super::super::error::ForumResult;
use super::super::git_ops::GitOps;

/// A single entry in the git-history-derived timeline view.
#[derive(Debug, Clone)]
pub struct SnapshotLogEntry {
    /// Full commit OID.
    pub sha: String,
    /// Author timestamp (RFC 3339, UTC).
    pub timestamp: DateTime<Utc>,
    /// Commit author display name (from `%an`).
    pub author: String,
    /// First line of the commit message (subject), verbatim.
    pub subject: String,
    /// Recognized SPEC-3.0 §5.3 operation, or `Other` for free-form
    /// commit messages (§5.4 says these must still be shown).
    pub op: SnapshotOp,
    /// Tree paths changed by this commit, relative to the snapshot
    /// root (e.g. `nodes/<id>.toml`, `body.md`, `links.toml`).
    /// Empty for the initial commit (no parent to diff against).
    pub changed_paths: Vec<String>,
}

/// Recognized operation-shaped commit messages (SPEC-3.0 §5.3).
///
/// Variants store only the parsed argument tokens — formatting is the
/// renderer's job. Free-form subjects map to [`SnapshotOp::Other`];
/// the renderer falls back to the raw subject in that case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotOp {
    ThreadCreate {
        thread: String,
    },
    NodeAdd {
        thread: String,
        node: String,
    },
    NodeResolve {
        thread: String,
        node: String,
    },
    State {
        thread: String,
        from: String,
        to: String,
    },
    LinkAdd {
        thread: String,
        target: String,
        rel: String,
    },
    /// Subject did not match any §5.3 pattern. Renderer shows the raw
    /// `subject` instead.
    Other,
}

const PREFIX: &str = "[git-forum] ";

/// Parse a single commit subject into a [`SnapshotOp`].
///
/// Recognizes the SPEC-3.0 §5.3 patterns; everything else returns
/// `Other`. Whitespace is normalized; missing/extra tokens fall through
/// to `Other` rather than guessing.
pub fn parse_subject(subject: &str) -> SnapshotOp {
    let Some(rest) = subject.strip_prefix(PREFIX) else {
        return SnapshotOp::Other;
    };
    let mut parts = rest.split_whitespace();
    let op = parts.next().unwrap_or("");
    match op {
        "thread-create" => match parts.next() {
            Some(thread) if parts.next().is_none() => SnapshotOp::ThreadCreate {
                thread: thread.to_string(),
            },
            _ => SnapshotOp::Other,
        },
        "node-add" => match (parts.next(), parts.next()) {
            (Some(thread), Some(node)) if parts.next().is_none() => SnapshotOp::NodeAdd {
                thread: thread.to_string(),
                node: node.to_string(),
            },
            _ => SnapshotOp::Other,
        },
        "node-resolve" => match (parts.next(), parts.next()) {
            (Some(thread), Some(node)) if parts.next().is_none() => SnapshotOp::NodeResolve {
                thread: thread.to_string(),
                node: node.to_string(),
            },
            _ => SnapshotOp::Other,
        },
        "state" => match (parts.next(), parts.next()) {
            // Argument shape: `<id> <from>-><to>`
            (Some(thread), Some(transition)) if parts.next().is_none() => {
                if let Some((from, to)) = transition.split_once("->") {
                    SnapshotOp::State {
                        thread: thread.to_string(),
                        from: from.to_string(),
                        to: to.to_string(),
                    }
                } else {
                    SnapshotOp::Other
                }
            }
            _ => SnapshotOp::Other,
        },
        "link-add" => match (parts.next(), parts.next(), parts.next()) {
            (Some(thread), Some(target), Some(rel)) if parts.next().is_none() => {
                SnapshotOp::LinkAdd {
                    thread: thread.to_string(),
                    target: target.to_string(),
                    rel: rel.to_string(),
                }
            }
            _ => SnapshotOp::Other,
        },
        _ => SnapshotOp::Other,
    }
}

/// Op-kind label for the rendered table (the `op` column).
fn op_label(op: &SnapshotOp) -> &'static str {
    match op {
        SnapshotOp::ThreadCreate { .. } => "thread-create",
        SnapshotOp::NodeAdd { .. } => "node-add",
        SnapshotOp::NodeResolve { .. } => "node-resolve",
        SnapshotOp::State { .. } => "state",
        SnapshotOp::LinkAdd { .. } => "link-add",
        SnapshotOp::Other => "(commit)",
    }
}

/// Per-row detail column derived from the parsed operation.
fn op_detail(entry: &SnapshotLogEntry) -> String {
    match &entry.op {
        SnapshotOp::ThreadCreate { thread } => thread.clone(),
        SnapshotOp::NodeAdd { node, .. } | SnapshotOp::NodeResolve { node, .. } => node.clone(),
        SnapshotOp::State { from, to, .. } => format!("{from} -> {to}"),
        SnapshotOp::LinkAdd { target, rel, .. } => format!("{target} ({rel})"),
        SnapshotOp::Other => entry.subject.clone(),
    }
}

/// Walk `refname`'s commit history newest-first and return one
/// [`SnapshotLogEntry`] per commit.
///
/// Uses `git log --name-only` so each entry carries its changed-paths
/// list — required by [`entries_touching`] for per-node filtering.
pub fn read_log(git: &GitOps, refname: &str) -> ForumResult<Vec<SnapshotLogEntry>> {
    // Custom format with a unique record separator (`\x1e`, RS) and
    // field separator (`\x1f`, US) so we can parse without splitting
    // on whitespace inside subjects.
    let fmt = "%x1e%H%x1f%aI%x1f%an%x1f%s";
    let raw = git.run(&["log", &format!("--format={fmt}"), "--name-only", refname])?;
    let mut entries = Vec::new();
    for record in raw.split('\x1e').filter(|r| !r.is_empty()) {
        // record = "<sha>\x1f<iso>\x1f<author>\x1f<subject>\n[paths\n...]"
        let mut header_and_paths = record.splitn(2, '\n');
        let header = header_and_paths.next().unwrap_or("");
        let paths_block = header_and_paths.next().unwrap_or("");
        let mut fields = header.splitn(4, '\x1f');
        let sha = fields.next().unwrap_or("").to_string();
        let iso = fields.next().unwrap_or("");
        let author = fields.next().unwrap_or("").to_string();
        let subject = fields.next().unwrap_or("").to_string();
        if sha.is_empty() {
            continue;
        }
        let timestamp = DateTime::parse_from_rfc3339(iso)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let changed_paths = paths_block
            .lines()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let op = parse_subject(&subject);
        entries.push(SnapshotLogEntry {
            sha,
            timestamp,
            author,
            subject,
            op,
            changed_paths,
        });
    }
    Ok(entries)
}

/// Subset of `entries` whose `changed_paths` touched any of `path_prefixes`.
///
/// Used by `commands::show::render_node_show` to surface the per-node
/// history slice (`nodes/<id>.toml`, `nodes/<id>.md`).
pub fn entries_touching<'a>(
    entries: &'a [SnapshotLogEntry],
    path_prefixes: &[&str],
) -> Vec<&'a SnapshotLogEntry> {
    entries
        .iter()
        .filter(|e| {
            e.changed_paths
                .iter()
                .any(|p| path_prefixes.iter().any(|pre| p.starts_with(pre)))
        })
        .collect()
}

/// Render the canonical 3.0 timeline table per SPEC-3.0 §5.4. Columns:
///
/// | date | sha | author | op | detail |
///
/// `date` is the commit author timestamp; `sha` is the first 8 chars of
/// the commit OID; `op` is the recognized operation kind (or `(commit)`);
/// `detail` is the parsed argument or the raw subject.
pub fn render_markdown(entries: &[SnapshotLogEntry]) -> Vec<String> {
    render_markdown_refs(&entries.iter().collect::<Vec<_>>())
}

/// Reference-slice variant for callers that already filtered (e.g.
/// per-node history via [`entries_touching`]).
pub fn render_markdown_refs(entries: &[&SnapshotLogEntry]) -> Vec<String> {
    let mut lines = Vec::with_capacity(entries.len() + 2);
    lines.push("| date | sha | author | op | detail |".into());
    lines.push("|------|-----|--------|-----|--------|".into());
    for entry in entries {
        lines.push(format!(
            "| {} | {} | {} | {} | {} |",
            entry.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
            short_sha(&entry.sha),
            entry.author,
            op_label(&entry.op),
            op_detail(entry),
        ));
    }
    lines
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_thread_create() {
        let op = parse_subject("[git-forum] thread-create fg61bcmp");
        assert_eq!(
            op,
            SnapshotOp::ThreadCreate {
                thread: "fg61bcmp".into()
            }
        );
    }

    #[test]
    fn parse_node_add() {
        let op = parse_subject("[git-forum] node-add fg61bcmp 7eaded74");
        assert_eq!(
            op,
            SnapshotOp::NodeAdd {
                thread: "fg61bcmp".into(),
                node: "7eaded74".into()
            }
        );
    }

    #[test]
    fn parse_state_transition() {
        let op = parse_subject("[git-forum] state fg61bcmp draft->open");
        assert_eq!(
            op,
            SnapshotOp::State {
                thread: "fg61bcmp".into(),
                from: "draft".into(),
                to: "open".into()
            }
        );
    }

    #[test]
    fn parse_link_add() {
        let op = parse_subject("[git-forum] link-add fg61bcmp 7ymtc4b2 implements");
        assert_eq!(
            op,
            SnapshotOp::LinkAdd {
                thread: "fg61bcmp".into(),
                target: "7ymtc4b2".into(),
                rel: "implements".into()
            }
        );
    }

    #[test]
    fn unrecognized_subject_falls_through() {
        assert_eq!(parse_subject("merge branch foo"), SnapshotOp::Other);
        assert_eq!(
            parse_subject("[git-forum] unknown-op id"),
            SnapshotOp::Other
        );
        assert_eq!(
            parse_subject("[git-forum] state id badtransition"),
            SnapshotOp::Other
        );
        // Right prefix, no op token.
        assert_eq!(parse_subject("[git-forum] "), SnapshotOp::Other);
    }

    #[test]
    fn render_table_has_header_and_one_row_per_entry() {
        let entries = vec![
            SnapshotLogEntry {
                sha: "0123456789abcdef".into(),
                timestamp: DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                author: "human/alice".into(),
                subject: "[git-forum] node-add fg61bcmp 7eaded74".into(),
                op: SnapshotOp::NodeAdd {
                    thread: "fg61bcmp".into(),
                    node: "7eaded74".into(),
                },
                changed_paths: vec!["nodes/7eaded74.toml".into()],
            },
            SnapshotLogEntry {
                sha: "fedcba9876543210".into(),
                timestamp: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
                author: "human/bob".into(),
                subject: "[git-forum] thread-create fg61bcmp".into(),
                op: SnapshotOp::ThreadCreate {
                    thread: "fg61bcmp".into(),
                },
                changed_paths: vec!["thread.toml".into(), "body.md".into()],
            },
        ];
        let lines = render_markdown(&entries);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "| date | sha | author | op | detail |");
        assert_eq!(lines[1], "|------|-----|--------|-----|--------|");
        assert!(lines[2].contains("01234567"));
        assert!(lines[2].contains("node-add"));
        assert!(lines[2].contains("7eaded74"));
        assert!(lines[3].contains("fedcba98"));
        assert!(lines[3].contains("thread-create"));
        assert!(lines[3].contains("fg61bcmp"));
    }

    #[test]
    fn entries_touching_filters_by_path_prefix() {
        let entries = vec![
            SnapshotLogEntry {
                sha: "a".into(),
                timestamp: Utc::now(),
                author: "x".into(),
                subject: "x".into(),
                op: SnapshotOp::Other,
                changed_paths: vec!["nodes/abc.toml".into()],
            },
            SnapshotLogEntry {
                sha: "b".into(),
                timestamp: Utc::now(),
                author: "x".into(),
                subject: "x".into(),
                op: SnapshotOp::Other,
                changed_paths: vec!["thread.toml".into()],
            },
            SnapshotLogEntry {
                sha: "c".into(),
                timestamp: Utc::now(),
                author: "x".into(),
                subject: "x".into(),
                op: SnapshotOp::Other,
                changed_paths: vec!["nodes/abc.md".into()],
            },
        ];
        let touching = entries_touching(&entries, &["nodes/abc."]);
        assert_eq!(touching.len(), 2);
        assert_eq!(touching[0].sha, "a");
        assert_eq!(touching[1].sha, "c");
    }

    #[test]
    fn raw_log_format_round_trips() {
        // Hand-construct what `git log --format=...` would emit, then
        // verify the parser's record/field splitting works.
        let raw = "\x1e\
            sha1\x1f2026-01-01T00:00:00Z\x1fhuman/alice\x1f[git-forum] thread-create fg61bcmp\n\
            thread.toml\nbody.md\n\
            \x1e\
            sha2\x1f2026-01-02T00:00:00Z\x1fhuman/bob\x1f[git-forum] node-add fg61bcmp 7eaded74\n\
            nodes/7eaded74.toml\nnodes/7eaded74.md\n";
        // We test the parsing pipeline by inlining it (the live read_log
        // shells out to git; this exercises the splitting+parse logic
        // separately).
        let mut parsed = Vec::new();
        for record in raw.split('\x1e').filter(|r| !r.is_empty()) {
            let mut header_and_paths = record.splitn(2, '\n');
            let header = header_and_paths.next().unwrap_or("");
            let paths_block = header_and_paths.next().unwrap_or("");
            let mut fields = header.splitn(4, '\x1f');
            let sha = fields.next().unwrap_or("").to_string();
            let _iso = fields.next().unwrap_or("");
            let _author = fields.next().unwrap_or("");
            let subject = fields.next().unwrap_or("").to_string();
            let paths: Vec<String> = paths_block
                .lines()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            parsed.push((sha, subject, paths));
        }
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, "sha1");
        assert_eq!(parsed[0].2, vec!["thread.toml", "body.md"]);
        assert_eq!(
            parsed[1].2,
            vec!["nodes/7eaded74.toml", "nodes/7eaded74.md"]
        );
    }
}
