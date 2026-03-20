use std::fs;

use super::config::RepoPaths;
use super::error::ForumResult;

const DEFAULT_POLICY: &str = r#"# git-forum default policy
# See doc/spec/SPEC.md for details.

[roles.reviewer]
can_say = ["question", "objection", "summary", "risk"]
can_transition = []

[roles.maintainer]
can_say = ["claim", "summary", "action"]
can_transition = ["draft->proposed", "proposed->under-review", "under-review->accepted"]

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]

[[guards]]
on = "open->closed"
requires = ["no_open_actions"]

# Uncomment to require commit evidence before closing issues:
# [[guards]]
# on = "open->closed"
# requires = ["has_commit_evidence"]
"#;

const DEFAULT_ACTORS: &str = r#"# git-forum actors
# Register human and AI actors here.
#
# [[actors]]
# id = "human/alice"
# kind = "human"
# display_name = "Alice"
"#;

const TEMPLATE_ISSUE: &str = "# {title}\n";
const TEMPLATE_RFC: &str = "# {title}\n";

/// Initialize `.forum/` and `.git/forum/` structure.
///
/// Idempotent: skips files that already exist.
pub fn init_forum(paths: &RepoPaths) -> ForumResult<()> {
    // .forum/ shared config
    let templates_dir = paths.dot_forum.join("templates");
    fs::create_dir_all(&templates_dir)?;

    write_if_missing(&paths.dot_forum.join("policy.toml"), DEFAULT_POLICY)?;
    write_if_missing(&paths.dot_forum.join("actors.toml"), DEFAULT_ACTORS)?;
    write_if_missing(&templates_dir.join("issue.md"), TEMPLATE_ISSUE)?;
    write_if_missing(&templates_dir.join("rfc.md"), TEMPLATE_RFC)?;

    // .git/forum/ local-only data
    fs::create_dir_all(paths.git_forum.join("logs"))?;

    // Set up a local git alias so `git forum --help` works correctly
    if let Some(repo_root) = paths.dot_forum.parent() {
        let _ = std::process::Command::new("git")
            .args(["config", "--local", "alias.forum", "!git-forum"])
            .current_dir(repo_root)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .status();
    }

    Ok(())
}

fn write_if_missing(path: &std::path::Path, content: &str) -> ForumResult<()> {
    if !path.exists() {
        fs::write(path, content)?;
    }
    Ok(())
}
