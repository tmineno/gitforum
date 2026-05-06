//! `git forum diff <THREAD_ID> [--rev N|N..M]` orchestration.
//!
//! task `913c4s9v`: revisions are
//! derived from the snapshot ref's git history per SPEC-3.0 §5.4 —
//! a "body revision" is a snapshot commit whose tree changed
//! `body.md`. Earlier revisions of this file walked
//! `state.events.filter(EventType::ReviseBody)`; that path is gone.

use std::io::Write;

use tempfile::NamedTempFile;

use super::super::error::{ForumError, ForumResult};
use super::super::git_ops::GitOps;
use super::super::refs::thread_ref;
use super::super::snapshot::history;
use super::super::thread::{self, ThreadState};
use super::context::Context;
use super::shared::resolve_tid;

/// Args for [`run`] — `git forum diff`.
pub struct DiffArgs {
    pub thread_id: String,
    pub rev: Option<String>,
}

/// Uniform entry point for the `diff` subcommand.
pub fn run(args: DiffArgs, ctx: &Context) -> Result<(), ForumError> {
    let thread_id = resolve_tid(&ctx.git, &args.thread_id)?;
    let state = thread::replay_thread(&ctx.git, &thread_id)?;
    let output = diff_body(&ctx.git, &state, args.rev.as_deref())?;
    println!("{output}");
    Ok(())
}

/// A body revision extracted from the snapshot history.
struct BodyRevision {
    body: String,
}

/// Collect body revisions by walking the snapshot ref's git history
/// per SPEC-3.0 §5.4.
///
/// Walks every commit oldest-first and reads `body.md` from each
/// commit's tree, defaulting to empty when the tree omits it
/// (SPEC-3.0 §4.2: optional files MAY be absent). Pushes a new
/// `BodyRevision` only when the body differs from the previous one,
/// so `node-add` / `link-add` snapshot commits that don't change the
/// body don't inflate the revision count. Revision 0 is the body at
/// the first commit (empty when the initial snapshot omits body.md);
/// revision N is the latest distinct body.
///
/// Returns an empty vec on legacy event-chain refs — those don't
/// store the thread body as a tree path, so there is nothing to diff.
fn collect_revisions(git: &GitOps, state: &ThreadState) -> ForumResult<Vec<BodyRevision>> {
    let entries = history::read_log(git, &thread_ref(&state.id))?;
    let mut revisions: Vec<BodyRevision> = Vec::new();
    // history::read_log returns newest-first; revisions go oldest-first
    // so revision 0 is the body at the earliest commit.
    for entry in entries.iter().rev() {
        let body = git.show_file(&entry.sha, "body.md").unwrap_or_default();
        match revisions.last() {
            Some(prev) if prev.body == body => continue,
            _ => revisions.push(BodyRevision { body }),
        }
    }
    Ok(revisions)
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
    let revisions = collect_revisions(git, state)?;
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
