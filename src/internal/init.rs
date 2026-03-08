use std::fs;

use super::config::RepoPaths;
use super::error::ForumResult;

const DEFAULT_POLICY: &str = r#"# git-forum default policy
# See docs/spec/MVP_SPEC.md for details.

[roles.reviewer]
can_say = ["objection", "evidence", "summary", "risk"]
can_transition = ["under-review->changes-requested"]

[roles.maintainer]
can_say = ["claim", "decision", "summary"]
can_transition = ["draft->proposed", "under-review->accepted"]

[[guards]]
on = "under-review->accepted"
requires = ["one_human_approval", "at_least_one_summary", "no_open_objections"]
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
const TEMPLATE_DECISION: &str = "# {title}\n";

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
    write_if_missing(&templates_dir.join("decision.md"), TEMPLATE_DECISION)?;

    // .git/forum/ local-only data
    fs::create_dir_all(paths.git_forum.join("logs"))?;

    Ok(())
}

fn write_if_missing(path: &std::path::Path, content: &str) -> ForumResult<()> {
    if !path.exists() {
        fs::write(path, content)?;
    }
    Ok(())
}
