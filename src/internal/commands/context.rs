//! Application `Context` — the dependency bundle every CLI command receives.
//!
//! Per RFC `7ymtc4b2`, task `t8o3vnt6`: main.rs builds a `Context`
//! once from the surrounding repo + clock + config, then dispatches to
//! `internal::commands::<cmd>::run(args, &ctx)`. This eliminates the
//! per-arm `discover_repo_with_init_warning` + `resolve_actor` duplication
//! and gives every command the same dependency surface.
//!
//! task `qa8u71j9` snapshot writer extends this struct with a `SnapshotStore`
//! field; the change is additive and does not affect existing callers.
//!
//! ## Init warning
//!
//! Most commands want the "git-forum is not initialized" warning when run
//! against a non-forum repo. `init` itself and the `hook` low-level
//! sub-commands deliberately skip the warning — they construct the
//! `Context` via [`Context::discover_quiet`].

use std::cell::OnceCell;

use crate::internal::clock::Clock;
use crate::internal::config::{self, RepoPaths};
use crate::internal::error::ForumError;
use crate::internal::git_ops::GitOps;
use crate::internal::thread::NodeIdIndex;

/// The dependency bundle every `commands::*::run` receives.
///
/// Built once in `main()`; passed by reference to each command. Owns
/// the discovered git handle, repo paths, and a swappable clock so
/// tests can inject [`FixedClock`](crate::internal::clock::FixedClock).
pub struct Context {
    pub git: GitOps,
    pub paths: RepoPaths,
    pub clock: Box<dyn Clock>,
    /// Bug `bvdk2w48`: lazily-built node-id → thread-id reverse
    /// index, populated on first call to [`Context::node_index`].
    /// Single-process callers (TUI, future bulk commands) that
    /// resolve multiple node ids amortise the ref sweep across
    /// every lookup after the first.
    node_index: OnceCell<NodeIdIndex>,
}

impl Context {
    /// Build a `Context` for normal commands. Emits the
    /// "git-forum is not initialized" stderr warning when the repo has
    /// no `.forum/policy.toml` and no `refs/forum/threads/*` refs.
    pub fn discover(clock: Box<dyn Clock>) -> Result<Self, ForumError> {
        let mut git = GitOps::discover()?;
        let git_dir = git.git_dir()?;
        let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
        if !is_forum_initialized(&paths, &git) {
            eprintln!(
                "warning: git-forum is not initialized in this repository; run `git forum init` first"
            );
        }
        load_local_into_git(&mut git, &paths);
        Ok(Self {
            git,
            paths,
            clock,
            node_index: OnceCell::new(),
        })
    }

    /// Build a `Context` without the init-warning probe. Used by the
    /// `init` command itself (creating `.forum/`) and by `hook`
    /// sub-commands that may run during clone bootstrap when the forum
    /// hasn't been initialised yet.
    pub fn discover_quiet(clock: Box<dyn Clock>) -> Result<Self, ForumError> {
        let mut git = GitOps::discover()?;
        let git_dir = git.git_dir()?;
        let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
        load_local_into_git(&mut git, &paths);
        Ok(Self {
            git,
            paths,
            clock,
            node_index: OnceCell::new(),
        })
    }

    /// Lazy accessor for the per-process node-id reverse index.
    /// First call sweeps `refs/forum/threads/*` with
    /// `ls-tree --name-only nodes/`; subsequent calls hand back the
    /// cached value.
    pub fn node_index(&self) -> Result<&NodeIdIndex, ForumError> {
        if let Some(index) = self.node_index.get() {
            return Ok(index);
        }
        let index = NodeIdIndex::build(&self.git)?;
        Ok(self.node_index.get_or_init(|| index))
    }
}

fn is_forum_initialized(paths: &RepoPaths, git: &GitOps) -> bool {
    if paths.dot_forum.join("policy.toml").is_file() && paths.git_forum.join("logs").is_dir() {
        return true;
    }
    git.list_refs("refs/forum/threads/")
        .map(|refs| !refs.is_empty())
        .unwrap_or(false)
}

fn load_local_into_git(git: &mut GitOps, paths: &RepoPaths) {
    let local_cfg = config::load_local_config(paths).unwrap_or_default();
    if let Some(identity) = local_cfg.commit_identity {
        git.set_commit_identity(identity);
    }
    if let Some(default_actor) = local_cfg.default_actor {
        git.set_default_actor(default_actor);
    }
}
