use std::io::Write;

use tempfile::NamedTempFile;

use super::error::{ForumError, ForumResult};
use super::event::EventType;
use super::git_ops::GitOps;
use super::thread::ThreadState;

/// A body revision extracted from the event history.
struct BodyRevision {
    body: String,
}

/// Collect body revisions from a thread's events.
///
/// Revision 0 is the body from the Create event (empty string if absent).
/// Revisions 1..N correspond to each ReviseBody event in timeline order.
fn collect_revisions(state: &ThreadState) -> Vec<BodyRevision> {
    let mut revisions = Vec::new();

    for event in &state.events {
        match event.event_type {
            EventType::Create => {
                revisions.push(BodyRevision {
                    body: event.body.clone().unwrap_or_default(),
                });
            }
            EventType::ReviseBody => {
                if let Some(ref body) = event.body {
                    revisions.push(BodyRevision { body: body.clone() });
                }
            }
            _ => {}
        }
    }

    revisions
}

/// Parse a `--rev` argument into a (from, to) pair of revision indices.
///
/// Accepted formats:
/// - `N` → diff between revision N-1 and N
/// - `N..M` → diff between revision N and M
pub fn parse_rev_arg(rev: &str, max_rev: usize) -> ForumResult<(usize, usize)> {
    if let Some((left, right)) = rev.split_once("..") {
        let from: usize = left.parse().map_err(|_| {
            ForumError::Config(format!("invalid revision number '{left}' in --rev {rev}"))
        })?;
        let to: usize = right.parse().map_err(|_| {
            ForumError::Config(format!("invalid revision number '{right}' in --rev {rev}"))
        })?;
        if from > max_rev {
            return Err(ForumError::Config(format!(
                "revision {from} does not exist; latest revision is {max_rev}"
            )));
        }
        if to > max_rev {
            return Err(ForumError::Config(format!(
                "revision {to} does not exist; latest revision is {max_rev}"
            )));
        }
        if from == to {
            return Err(ForumError::Config(format!(
                "cannot diff revision {from} against itself"
            )));
        }
        Ok((from, to))
    } else {
        let n: usize = rev
            .parse()
            .map_err(|_| ForumError::Config(format!("invalid revision number '{rev}' in --rev")))?;
        if n == 0 {
            return Err(ForumError::Config(
                "revision 0 has no previous revision to diff against; use --rev 0..N".into(),
            ));
        }
        if n > max_rev {
            return Err(ForumError::Config(format!(
                "revision {n} does not exist; latest revision is {max_rev}"
            )));
        }
        Ok((n - 1, n))
    }
}

/// Produce a unified diff between two body revisions of a thread.
///
/// If `rev_spec` is None, diffs the latest revision against the previous one.
/// If `rev_spec` is Some, parses it as a revision specifier (see `parse_rev_arg`).
///
/// Returns the diff output as a string, or an informative message if there
/// are no revisions to diff.
pub fn diff_body(git: &GitOps, state: &ThreadState, rev_spec: Option<&str>) -> ForumResult<String> {
    let revisions = collect_revisions(state);
    let max_rev = revisions.len().saturating_sub(1);

    if revisions.len() < 2 {
        return Ok(format!(
            "{}: no body revisions to diff (body revision count: {})",
            state.id, state.body_revision_count
        ));
    }

    let (from, to) = match rev_spec {
        Some(spec) => parse_rev_arg(spec, max_rev)?,
        None => (max_rev - 1, max_rev),
    };

    let old_body = &revisions[from].body;
    let new_body = &revisions[to].body;

    // Write bodies to temp files for git diff
    let mut old_file = NamedTempFile::new()?;
    old_file.write_all(old_body.as_bytes())?;
    if !old_body.ends_with('\n') {
        old_file.write_all(b"\n")?;
    }
    old_file.flush()?;

    let mut new_file = NamedTempFile::new()?;
    new_file.write_all(new_body.as_bytes())?;
    if !new_body.ends_with('\n') {
        new_file.write_all(b"\n")?;
    }
    new_file.flush()?;

    let old_path_str = old_file.path().to_str().unwrap().to_string();
    let new_path_str = new_file.path().to_str().unwrap().to_string();

    let diff_output = git.diff_no_index(&old_path_str, &new_path_str, &[])?;

    if diff_output.is_empty() {
        return Ok(format!(
            "{}: revisions {from} and {to} are identical",
            state.id
        ));
    }

    // Replace temp file paths with user-facing revision labels.
    // Git may normalize absolute paths (stripping leading /), so replace
    // both the full path and the path without leading /.
    let old_label = format!("a/rev{from}/body");
    let new_label = format!("b/rev{to}/body");
    let old_no_slash = old_path_str.strip_prefix('/').unwrap_or(&old_path_str);
    let new_no_slash = new_path_str.strip_prefix('/').unwrap_or(&new_path_str);
    let cleaned = diff_output
        .replace(&old_path_str, &old_label)
        .replace(old_no_slash, &old_label)
        .replace(&new_path_str, &new_label)
        .replace(new_no_slash, &new_label);

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rev_single() {
        assert_eq!(parse_rev_arg("1", 3).unwrap(), (0, 1));
        assert_eq!(parse_rev_arg("3", 3).unwrap(), (2, 3));
    }

    #[test]
    fn parse_rev_range() {
        assert_eq!(parse_rev_arg("0..2", 3).unwrap(), (0, 2));
        assert_eq!(parse_rev_arg("1..3", 3).unwrap(), (1, 3));
    }

    #[test]
    fn parse_rev_zero_alone_fails() {
        assert!(parse_rev_arg("0", 3).is_err());
    }

    #[test]
    fn parse_rev_same_fails() {
        assert!(parse_rev_arg("2..2", 3).is_err());
    }

    #[test]
    fn parse_rev_out_of_range() {
        assert!(parse_rev_arg("5", 3).is_err());
        assert!(parse_rev_arg("0..5", 3).is_err());
    }
}
