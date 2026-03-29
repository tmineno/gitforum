use std::fs;
use std::io::Read;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use git_forum::internal::actor;
use git_forum::internal::branch_ops;
use git_forum::internal::clock::SystemClock;
use git_forum::internal::config;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::diff;
use git_forum::internal::doctor;
use git_forum::internal::editor;
use git_forum::internal::error::ForumError;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::github;
use git_forum::internal::github_export;
use git_forum::internal::github_import;
use git_forum::internal::hook;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::operation_check;
use git_forum::internal::policy::Policy;
use git_forum::internal::purge;
use git_forum::internal::reindex;
use git_forum::internal::show;
use git_forum::internal::state_change;
use git_forum::internal::thread;
use git_forum::internal::tui as forum_tui;
use git_forum::internal::verify;
use git_forum::internal::write_ops;

const GROUPED_HELP: &str = "\
These are common git-forum commands:

setup and repo health
   init        Initialize a git-forum repository
   doctor      Diagnose repo health (config, index, refs)
   reindex     Rebuild local index from Git refs

create and browse threads
   new         Create a new thread (kinds: ask, rfc, dec, job)
   ask         Ask (issue) sub-commands
   job         Job (task) sub-commands
   ls          List threads (filter by kind, status, or branch)
   show        Show thread details (use --what-next for diagnostics)
   diff        Show diff between body revisions
   search      Search threads and nodes
   status      Show unresolved items for a thread

structured discussion (see also: git forum node add --help)
   node show   Show full body of a node by ID
   node add    Add a typed discussion node
   revise      Revise thread body or node body
   retract     Retract a node (soft-delete)
   resolve     Resolve a node (mark as addressed)
   reopen      Reopen a resolved/retracted node

state transitions (see also: git forum state --help)
   state       Transition a thread to a new state

evidence and links
   evidence    Add evidence to a thread
   link        Link two threads
   branch      Bind or clear a thread's branch scope

policy and preflight
   verify      Preflight: is this thread ready to advance?
   policy      Policy sub-commands (show, lint, check)

hooks and maintenance
   hook        Manage the commit-msg hook

interactive
   tui         Open the interactive TUI

import / export
   import      Import from external sources
   export      Export to external platforms

state shorthands (convenience aliases for 'state <ID> <target>')
   close       state <ID> closed
   pend        state <ID> pending
   accept      state <ID> accepted
   propose     state <ID> proposed
   reject      state <ID> rejected
   deprecate   state <ID> deprecated

node shorthands (convenience aliases for 'node add <ID> --type <type>')
   claim       node add --type claim
   question    node add --type question
   objection   node add --type objection
   summary     node add --type summary
   action      node add --type action
   risk        node add --type risk
   review      node add --type review

'git forum <command> --help' for more on a specific command.
'git forum --help-llm' for the full manual.";

#[derive(Parser)]
#[command(
    name = "git-forum",
    about = "Structured discussion in Git",
    help_template = "\
{about-with-newline}
Usage: {usage}

{options}
{after-help}",
    after_help = GROUPED_HELP,
)]
struct Cli {
    #[arg(long = "help-llm", help = "Print the full manual for LLMs and exit")]
    help_llm: bool,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a git-forum repository
    Init,
    /// Diagnose repo health (config, index, refs)
    Doctor {
        /// Show all checks including passing ones
        #[arg(long, short)]
        verbose: bool,
    },
    /// Rebuild local index from Git refs
    Reindex,
    /// Purge event content from git history (hard-delete)
    Purge {
        /// Thread ID (required with --event or --node)
        #[arg(long, value_name = "THREAD_ID")]
        thread: Option<String>,
        /// Event SHA to purge (requires --thread)
        #[arg(
            long,
            value_name = "EVENT_SHA",
            requires = "thread",
            conflicts_with = "node"
        )]
        event: Option<String>,
        /// Node ID to purge (requires --thread; resolves to the originating event)
        #[arg(
            long,
            value_name = "NODE_ID",
            requires = "thread",
            conflicts_with = "event"
        )]
        node: Option<String>,
        /// Purge all events by a specific actor across all threads
        #[arg(long, value_name = "ACTOR_ID", conflicts_with_all = ["thread", "event", "node"])]
        actor: Option<String>,
        /// Show what would be purged without modifying
        #[arg(long)]
        dry_run: bool,
    },
    /// Ask (issue) sub-commands
    Ask {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// Issue sub-commands (legacy alias for ask)
    #[command(hide = true)]
    Issue {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// RFC sub-commands
    #[command(hide = true)]
    Rfc {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// DEC sub-commands
    #[command(hide = true)]
    Dec {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// Job (task) sub-commands
    Job {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// Task sub-commands (legacy alias for job)
    #[command(hide = true)]
    Task {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// Create a new thread
    New {
        /// Thread kind: rfc or issue
        kind: String,
        /// Thread title (omit when using --from-commit)
        #[arg(
            allow_hyphen_values = true,
            required_unless_present_any = ["from_commit", "from_thread"]
        )]
        title: Option<String>,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long = "from-commit", value_name = "REV")]
        from_commit: Option<String>,
        #[arg(long = "from-thread", value_name = "THREAD_ID")]
        from_thread: Option<String>,
        #[arg(long)]
        claim: Vec<String>,
        #[arg(long)]
        question: Vec<String>,
        #[arg(long)]
        objection: Vec<String>,
        #[arg(long)]
        action: Vec<String>,
        #[arg(long)]
        risk: Vec<String>,
        #[arg(long)]
        summary: Vec<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// List all threads (optionally filter by kind and/or status)
    #[command(alias = "list")]
    Ls {
        /// Thread kind (rfc or issue) — positional shorthand for --kind
        kind_positional: Option<String>,
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        /// Filter by thread kind (rfc or issue)
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Filter by thread status (open, closed, draft, etc.)
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
    },
    /// Close a thread (issue shorthand)
    Close {
        thread_id: String,
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Mark a thread as pending (issue shorthand)
    Pend {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Accept an RFC (shorthand for state <ID> accepted)
    Accept {
        thread_id: String,
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Propose an RFC for review (shorthand for state <ID> proposed)
    Propose {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Deprecate an RFC (shorthand for state <ID> deprecated)
    Deprecate {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Reject a thread (shorthand for state <ID> rejected)
    Reject {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Show thread details
    Show {
        thread_id: String,
        /// Show valid next actions, transitions, and guard check results
        #[arg(long)]
        what_next: bool,
        /// Truncate node bodies and timeline details to single-line previews
        #[arg(long)]
        compact: bool,
        /// Omit the timeline section
        #[arg(long)]
        no_timeline: bool,
    },
    /// Show unified diff between body revisions
    Diff {
        thread_id: String,
        /// Revision specifier: N (diff rev N-1 vs N) or N..M (diff rev N vs M)
        #[arg(long)]
        rev: Option<String>,
    },
    /// Show unresolved items for a thread
    Status { thread_id: String },
    /// Node sub-commands
    Node {
        #[command(subcommand)]
        cmd: NodeCmd,
    },
    /// Bind or clear a thread's Git branch scope
    Branch {
        #[command(subcommand)]
        cmd: BranchCmd,
    },
    /// Revise thread body (default) or node body
    #[command(args_conflicts_with_subcommands = true)]
    Revise {
        /// Thread ID (for default body revision)
        thread_id: Option<String>,
        /// New thread body text (use "-" to read from stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read new thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        /// Node IDs to mark as incorporated into this body revision
        #[arg(long = "incorporates", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
        #[command(subcommand)]
        cmd: Option<ReviseCmd>,
    },
    /// Add a claim node to a thread
    Claim {
        thread_id: String,
        /// Node body (positional; use --body or --body-file for named alternatives)
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Add a question node to a thread
    Question {
        thread_id: String,
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Add an objection node to a thread
    Objection {
        thread_id: String,
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Add a summary node to a thread
    Summary {
        thread_id: String,
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Add an action node to a thread
    Action {
        thread_id: String,
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Add a risk node to a thread
    Risk {
        thread_id: String,
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Add a review node to a thread
    Review {
        thread_id: String,
        body_positional: Option<String>,
        #[arg(long = "body", value_name = "TEXT")]
        body_flag: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Retract one or more nodes (soft-delete)
    Retract {
        thread_id: String,
        #[arg(
            num_args = 1..,
            required = true,
            value_name = "NODE_ID",
            help = "Full node ID(s) or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_ids: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Resolve one or more nodes (mark as addressed)
    Resolve {
        thread_id: String,
        #[arg(
            num_args = 1..,
            required = true,
            value_name = "NODE_ID",
            help = "Full node ID(s) or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_ids: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Reopen resolved/retracted node(s)
    Reopen {
        thread_id: String,
        #[arg(
            num_args = 1..,
            required = true,
            value_name = "NODE_ID",
            help = "Full node ID(s) or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_ids: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Transition a thread to a new state
    State {
        #[command(subcommand)]
        cmd: Option<StateCmd>,
        thread_id: Option<String>,
        new_state: Option<String>,
        /// Actor IDs to record as approvals (may be repeated)
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
        /// Create thread links atomically with the state transition
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        /// Relation to use with --link-to
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
        /// Walk through intermediate states to reach the target
        #[arg(long)]
        fast_track: bool,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    #[command(
        about = "Preflight check: test whether a thread is ready for its next forward transition",
        long_about = "Preflight check: evaluate policy guard conditions for the thread's next forward transition.\n\nThis is NOT a history audit or integrity check — it only answers: \"if I tried to advance this thread now, which guards would block?\"\n\nForward targets checked:\n- Issue in `open` → checks guards for `open->closed`\n- RFC in `under-review` → checks guards for `under-review->accepted`\n- DEC in `proposed` → checks guards for `proposed->accepted`\n- TASK in `reviewing` → checks guards for `reviewing->closed`\n- Other states → reports ready (no preflight target defined)\n\nThis command is read-only. It does not change thread state or attach approvals."
    )]
    Verify { thread_id: String },
    /// Policy sub-commands
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
    },
    /// Evidence sub-commands
    Evidence {
        #[command(subcommand)]
        cmd: EvidenceCmd,
    },
    /// Add a link between two threads
    Link {
        thread_id: String,
        target_thread_id: String,
        #[arg(long, value_name = "REL")]
        rel: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Search threads by title, kind, or status
    Search {
        /// Search query (matched against title, id, kind, and status)
        query: String,
    },
    /// Hook sub-commands
    Hook {
        #[command(subcommand)]
        cmd: HookCmd,
    },
    /// Open the interactive TUI
    Tui {
        /// Open a specific thread in detail view directly
        thread_id: Option<String>,
    },
    /// Import from external sources
    Import {
        #[command(subcommand)]
        cmd: ImportCmd,
    },
    /// Export to external platforms
    Export {
        #[command(subcommand)]
        cmd: ExportCmd,
    },
}

#[derive(Subcommand)]
enum PolicyCmd {
    /// Display the loaded policy in human-readable format
    Show,
    /// Check policy file for structural problems
    Lint,
    /// Check whether a transition satisfies policy guards
    Check {
        thread_id: String,
        #[arg(long)]
        transition: String,
    },
}

#[derive(Subcommand)]
enum ReviseCmd {
    /// Revise the body of a thread
    Body {
        thread_id: String,
        /// New thread body text (use "-" to read from stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read new thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        /// Node IDs to mark as incorporated into this body revision
        #[arg(long = "incorporates", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// Revise the body of an existing node
    Node {
        thread_id: String,
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_id: String,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read revised body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum EvidenceCmd {
    /// Add evidence items to a thread (accepts multiple --ref values)
    Add {
        thread_id: String,
        #[arg(long, value_name = "KIND")]
        kind: EvidenceKind,
        #[arg(long = "ref", value_name = "REF", num_args = 1..)]
        ref_targets: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum NodeCmd {
    /// Show a single node by ID
    Show {
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or globally unique prefix (8+ chars unless exact match)"
        )]
        node_id: String,
    },
    /// Add a typed node to a thread
    Add {
        thread_id: String,
        /// Node type (claim, question, objection, evidence, summary, action, risk, review, alternative, assumption)
        #[arg(long = "type", value_name = "TYPE")]
        node_type: NodeType,
        /// Node body (positional)
        #[arg(allow_hyphen_values = true)]
        body_positional: Option<String>,
        /// Node body (flag)
        #[arg(long = "body", value_name = "TEXT", conflicts_with = "body_positional")]
        body_flag: Option<String>,
        /// Read body from file
        #[arg(
            long = "body-file",
            value_name = "PATH",
            conflicts_with_all = ["body_positional", "body_flag"]
        )]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body_positional", "body_flag", "body_file"])]
        edit: bool,
        /// Reply to a specific node
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        /// Override actor identity
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum BranchCmd {
    /// Bind a thread to an existing Git branch
    Bind {
        thread_id: String,
        branch: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Clear the bound branch from a thread
    Clear {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
}

#[derive(Subcommand)]
enum HookCmd {
    /// Install git-forum commit-msg hook into the Git hooks directory
    Install {
        /// Overwrite existing hook without backup
        #[arg(long)]
        force: bool,
    },
    /// Remove git-forum commit-msg hook
    Uninstall,
    /// Validate thread references in a commit message file (used by the hook)
    CheckCommitMsg {
        /// Path to the commit message file (provided by Git)
        file: PathBuf,
    },
}

#[derive(Subcommand)]
enum ImportCmd {
    /// Import a GitHub issue into git-forum
    GithubIssue {
        /// GitHub repository (owner/repo)
        #[arg(long)]
        repo: String,
        /// Issue number to import
        #[arg(long, required_unless_present = "all")]
        issue: Option<u64>,
        /// Import all issues from the repository
        #[arg(long, conflicts_with = "issue")]
        all: bool,
        /// Actor identity
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Show what would be imported without creating anything
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ExportCmd {
    /// Export a git-forum thread to a GitHub issue
    GithubIssue {
        /// Thread ID to export
        thread_id: String,
        /// Target GitHub repository (owner/repo)
        #[arg(long)]
        repo: String,
        /// Actor identity
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Show what would be created without actually creating
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum StateCmd {
    /// Apply the same transition to multiple threads
    Bulk {
        #[arg(long = "to", value_name = "STATE")]
        new_state: String,
        thread_ids: Vec<String>,
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        #[arg(long, value_name = "STATUS")]
        status: Option<String>,
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum ThreadCmd {
    /// Create a new thread
    New {
        /// Thread title (omit when using --from-commit)
        #[arg(
            allow_hyphen_values = true,
            required_unless_present_any = ["from_commit", "from_thread"]
        )]
        title: Option<String>,
        /// Initial thread body
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read initial thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        /// Bind the new thread to an existing Git branch
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
        /// Create thread links immediately after creation (may be repeated)
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        /// Relation to use with --link-to
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        /// Override actor ID (default: from git config)
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Populate title/body from a commit and auto-add it as evidence
        #[arg(long = "from-commit", value_name = "REV")]
        from_commit: Option<String>,
        /// Create from an existing thread (supersede pattern: copies title/body, links both, auto-deprecates source RFC)
        #[arg(long = "from-thread", value_name = "THREAD_ID")]
        from_thread: Option<String>,
        /// Add a claim node after creation
        #[arg(long)]
        claim: Vec<String>,
        /// Add a question node after creation
        #[arg(long)]
        question: Vec<String>,
        /// Add an objection node after creation
        #[arg(long)]
        objection: Vec<String>,
        /// Add an action node after creation
        #[arg(long)]
        action: Vec<String>,
        /// Add a risk node after creation
        #[arg(long)]
        risk: Vec<String>,
        /// Add a summary node after creation
        #[arg(long)]
        summary: Vec<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
    },
    /// List threads of this kind
    #[command(alias = "list")]
    Ls {
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
    },
    /// Close a thread (shorthand for state <ID> closed)
    Close {
        thread_id: String,
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Mark a thread as pending (shorthand for state <ID> pending)
    Pend {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Reopen a closed or rejected thread (shorthand for state <ID> open)
    #[command(alias = "open")]
    Reopen {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Reject a thread (shorthand for state <ID> rejected)
    Reject {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Accept an RFC (shorthand for state <ID> accepted)
    Accept {
        thread_id: String,
        #[arg(long = "approve", value_name = "ACTOR")]
        approve: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long = "link-to", value_name = "THREAD_ID")]
        link_to: Vec<String>,
        #[arg(long, requires = "link_to", value_name = "REL")]
        rel: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Propose an RFC for review (shorthand for state <ID> proposed)
    Propose {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Deprecate an RFC (shorthand for state <ID> deprecated)
    Deprecate {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Attach a comment to the state-change event
        #[arg(long)]
        comment: Option<String>,
    },
    /// Revise thread body (default) or node body
    #[command(args_conflicts_with_subcommands = true)]
    Revise {
        /// Thread ID (for default body revision)
        thread_id: Option<String>,
        /// New thread body text (use "-" to read from stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read new thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Open $EDITOR to compose the body
        #[arg(long, conflicts_with_all = ["body", "body_file"])]
        edit: bool,
        /// Node IDs to mark as incorporated into this body revision
        #[arg(long = "incorporates", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
        #[command(subcommand)]
        cmd: Option<ReviseCmd>,
    },
}

/// Apply operation check violations: print to stderr, block on errors.
/// Returns Ok(()) if the operation should proceed, Err if blocked.
fn apply_operation_checks(
    violations: &[operation_check::OperationViolation],
    force: bool,
    strict: bool,
) -> Result<(), ForumError> {
    if violations.is_empty() {
        return Ok(());
    }
    let (has_errors, output) = operation_check::evaluate_violations(violations, force, strict);
    eprint!("{output}");
    if has_errors {
        Err(ForumError::Policy(
            "operation blocked by check violations".into(),
        ))
    } else {
        Ok(())
    }
}

fn main() -> Result<(), ForumError> {
    // Check for --help-llm before clap parsing so it works at any subcommand level
    // (e.g. `git-forum issue --help-llm` where clap would otherwise require a subcommand)
    if std::env::args().any(|a| a == "--help-llm") {
        let args: Vec<String> = std::env::args().collect();
        let context = args
            .iter()
            .position(|a| a == "--help-llm")
            .and_then(|pos| pos.checked_sub(1))
            .and_then(|prev| args.get(prev))
            .map(|s| s.as_str());

        use git_forum::internal::help;
        match context {
            Some(
                "claim" | "question" | "objection" | "summary" | "action" | "risk" | "review"
                | "alternative" | "assumption" | "node",
            ) => {
                print!("{}", help::node_type_taxonomy());
            }
            Some("state" | "close" | "reject" | "accept" | "propose" | "deprecate" | "pend") => {
                print!("{}", help::state_transition_map());
            }
            Some("evidence") => {
                print!("{}", help::evidence_kinds_reference());
            }
            _ => {
                print!("{}", include_str!("../doc/MANUAL.md"));
            }
        }
        return Ok(());
    }

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            let msg = e.to_string();
            if let Some(sub) = parse_unrecognized_subcommand(&msg) {
                if let Some(hint) = subcommand_hint(&sub) {
                    eprintln!("error: unrecognized subcommand '{sub}'\n\n  tip: {hint}\n");
                    std::process::exit(2);
                } else {
                    eprintln!("error: unrecognized subcommand '{sub}'\n\n  tip: run 'git forum --help-llm' for command reference\n");
                    std::process::exit(2);
                }
            }
            e.exit();
        }
    };
    if cli.help_llm {
        print!("{}", include_str!("../doc/MANUAL.md"));
        return Ok(());
    }

    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        std::process::exit(2);
    };

    let clock = SystemClock;

    match command {
        Commands::Init => {
            let git = GitOps::discover()?;
            let git_dir = git.git_dir()?;
            let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
            init::init_forum(&paths)?;
            // Generate local.toml with default_actor if it doesn't exist
            let local_toml_path = paths.git_forum.join("local.toml");
            if !local_toml_path.exists() {
                let default_actor = actor::actor_from_git_config(&git);
                let content = format!(
                    "# git-forum local config (per-clone, not committed)\n\
                     \n\
                     # Default actor ID for this clone.\n\
                     # Override per-command with --as or GIT_FORUM_ACTOR env var.\n\
                     default_actor = \"{default_actor}\"\n\
                     \n\
                     # Override git commit author/committer on forum commits.\n\
                     # Uncomment to use a pseudonym instead of git config user.name/email.\n\
                     # [commit_identity]\n\
                     # name = \"pseudonym\"\n\
                     # email = \"pseudonym@example.com\"\n"
                );
                std::fs::write(&local_toml_path, content)?;
                println!("Default actor: {default_actor}");
                eprintln!(
                    "hint: edit .git/forum/local.toml to change your actor ID or commit identity"
                );
            }
            // Configure fetch refspecs for forum refs on all remotes
            match init::ensure_forum_refspecs(&git) {
                Ok(modified) => {
                    for remote in &modified {
                        eprintln!("Added forum fetch refspec for remote '{remote}'");
                    }
                }
                Err(e) => {
                    eprintln!("warning: could not configure forum fetch refspecs: {e}");
                }
            }

            // Fetch forum refs from all remotes
            let mut fetched_any = false;
            if let Ok(remotes_output) = git.run(&["remote"]) {
                for remote in remotes_output.lines() {
                    let remote = remote.trim();
                    if remote.is_empty() {
                        continue;
                    }
                    match git.run(&["fetch", remote, init::FORUM_REFSPEC]) {
                        Ok(_) => {
                            eprintln!("Fetched forum refs from '{remote}'");
                            fetched_any = true;
                        }
                        Err(e) => {
                            eprintln!("warning: could not fetch forum refs from '{remote}': {e}");
                        }
                    }
                }
            }

            // Reindex if we fetched forum refs
            if fetched_any {
                let thread_ids = git
                    .list_refs("refs/forum/threads/")
                    .unwrap_or_default();
                if !thread_ids.is_empty() {
                    let db_path = paths.git_forum.join("index.db");
                    match reindex::run_reindex(&git, &db_path) {
                        Ok(report) => {
                            eprintln!(
                                "Reindexed {} threads",
                                report.threads_replayed.len()
                            );
                        }
                        Err(e) => {
                            eprintln!("warning: reindex failed: {e}");
                        }
                    }
                }
            }

            let dir_name = git
                .root()
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());
            println!("Initialized git-forum in {dir_name}");
            eprintln!("note: actor IDs (--as) are claimed identities, not authenticated. Approvals are recorded, not cryptographically verified.");
            let hook_path = hook::resolve_hook_path(&git)?;
            hook::install_hook(&hook_path, false)?;
        }

        Commands::Doctor { verbose } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let report = doctor::run_doctor(&git, &paths)?;

            // Separate replay checks from non-replay checks
            let mut replay_ok = 0u32;
            let mut replay_fail: Vec<&doctor::DoctorCheck> = Vec::new();
            let mut ok_count = 0u32;
            let mut warn_count = 0u32;
            let mut fail_count = 0u32;

            for check in &report.checks {
                match check.level {
                    doctor::CheckLevel::Ok => ok_count += 1,
                    doctor::CheckLevel::Warn => warn_count += 1,
                    doctor::CheckLevel::Fail => fail_count += 1,
                }

                let is_replay = check.name.starts_with("replay ");
                if is_replay {
                    if check.level == doctor::CheckLevel::Ok {
                        replay_ok += 1;
                        continue; // suppress passing replays unless verbose
                    } else {
                        replay_fail.push(check);
                        continue; // print replay failures below
                    }
                }

                // Non-replay checks: always show failures/warnings, show ok only if verbose
                if check.level != doctor::CheckLevel::Ok || verbose {
                    let marker = match check.level {
                        doctor::CheckLevel::Ok => " ok ",
                        doctor::CheckLevel::Warn => "WARN",
                        doctor::CheckLevel::Fail => "FAIL",
                    };
                    print!("[{marker}] {}", check.name);
                    if let Some(detail) = &check.detail {
                        print!(" -- {detail}");
                    }
                    println!();
                }
            }

            // Collapsed replay summary
            let total_replay = replay_ok + replay_fail.len() as u32;
            if total_replay > 0 {
                if replay_fail.is_empty() {
                    println!("[ ok ] replay: {replay_ok} threads replayed successfully");
                } else {
                    for check in &replay_fail {
                        let detail = check.detail.as_deref().unwrap_or("unknown error");
                        println!("[FAIL] {} -- {}", check.name, detail);
                    }
                    if replay_ok > 0 {
                        println!("[ ok ] replay: {replay_ok} other threads ok");
                    }
                }
            }

            // Summary line
            println!();
            if fail_count == 0 && warn_count == 0 {
                println!("All {ok_count} checks passed.");
            } else {
                let parts: Vec<String> = [
                    (fail_count, "failed"),
                    (warn_count, "warning"),
                    (ok_count, "passed"),
                ]
                .iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, label)| format!("{n} {label}"))
                .collect();
                println!("{}", parts.join(", "));
            }

            if !report.all_passed() {
                std::process::exit(1);
            }
        }

        Commands::Reindex => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let db_path = paths.git_forum.join("index.db");
            let report = reindex::run_reindex(&git, &db_path)?;
            println!(
                "Reindex complete: {} threads found, {} replayed, {} errors",
                report.threads_found,
                report.threads_replayed.len(),
                report.errors.len()
            );
            for (id, err) in &report.errors {
                eprintln!("  error: {id}: {err}");
            }
        }

        Commands::Purge {
            thread,
            event,
            node,
            actor,
            dry_run,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            match (thread, event, node, actor) {
                (Some(thread_id), None, Some(node_id), None) => {
                    let thread_id = resolve_tid(&git, &thread_id)?;
                    let resolved_node_id =
                        thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
                    let state = thread::replay_thread(&git, &thread_id)?;
                    let event_sha = state
                        .events
                        .iter()
                        .find(|e| {
                            e.event_type == git_forum::internal::event::EventType::Say
                                && e.target_node_id
                                    .as_deref()
                                    .unwrap_or(e.event_id.as_str())
                                    == resolved_node_id
                        })
                        .map(|e| e.event_id.clone())
                        .ok_or_else(|| {
                            ForumError::Repo(format!(
                                "no originating say event found for node '{node_id}' in thread '{thread_id}'"
                            ))
                        })?;
                    if dry_run {
                        let plan = purge::plan_purge_event(&git, &thread_id, &event_sha)?;
                        println!("Would purge {} event(s):", plan.events.len());
                        for e in &plan.events {
                            println!(
                                "  {} {} by {} (has body: {})",
                                e.thread_id, e.event_type, e.actor, e.has_body
                            );
                        }
                    } else {
                        let report = purge::purge_event(&git, &thread_id, &event_sha)?;
                        println!(
                            "Purged {} event(s), rewrote {} commit(s)",
                            report.events_purged, report.commits_rewritten
                        );
                        let db_path = paths.git_forum.join("index.db");
                        if db_path.exists() {
                            reindex::run_reindex(&git, &db_path)?;
                            println!("Index rebuilt");
                        }
                        eprintln!("warning: commit SHAs have changed — all clones must re-fetch affected refs");
                    }
                }
                (Some(thread_id), Some(event_sha), None, None) => {
                    let thread_id = resolve_tid(&git, &thread_id)?;
                    if dry_run {
                        let plan = purge::plan_purge_event(&git, &thread_id, &event_sha)?;
                        println!("Would purge {} event(s):", plan.events.len());
                        for e in &plan.events {
                            println!(
                                "  {} {} by {} (has body: {})",
                                e.thread_id, e.event_type, e.actor, e.has_body
                            );
                        }
                    } else {
                        let report = purge::purge_event(&git, &thread_id, &event_sha)?;
                        println!(
                            "Purged {} event(s), rewrote {} commit(s)",
                            report.events_purged, report.commits_rewritten
                        );
                        // Rebuild index
                        let db_path = paths.git_forum.join("index.db");
                        if db_path.exists() {
                            reindex::run_reindex(&git, &db_path)?;
                            println!("Index rebuilt");
                        }
                        eprintln!("warning: commit SHAs have changed — all clones must re-fetch affected refs");
                    }
                }
                (None, None, None, Some(actor_id)) => {
                    if dry_run {
                        let plan = purge::plan_purge_actor(&git, &actor_id)?;
                        println!("Would purge {} event(s):", plan.events.len());
                        for e in &plan.events {
                            println!(
                                "  {} {} by {} (has body: {})",
                                e.thread_id, e.event_type, e.actor, e.has_body
                            );
                        }
                    } else {
                        let report = purge::purge_actor(&git, &actor_id)?;
                        println!(
                            "Purged {} event(s) across {} thread(s), rewrote {} commit(s)",
                            report.events_purged,
                            report.threads_affected.len(),
                            report.commits_rewritten
                        );
                        for tid in &report.threads_affected {
                            println!("  {tid}");
                        }
                        // Rebuild index
                        let db_path = paths.git_forum.join("index.db");
                        if db_path.exists() {
                            reindex::run_reindex(&git, &db_path)?;
                            println!("Index rebuilt");
                        }
                        eprintln!("warning: commit SHAs have changed — all clones must re-fetch affected refs");
                    }
                }
                _ => {
                    return Err(ForumError::Config(
                        "specify either --thread + --event, --thread + --node, or --actor".into(),
                    ));
                }
            }
        }

        Commands::Search { query } => {
            let (_git, paths) = discover_repo_with_init_warning()?;
            let db_path = paths.git_forum.join("index.db");
            let conn = index::open_db(&db_path)?;
            let results = index::search_threads(&conn, &query)?;
            print!("{}", show::render_search_results(&results));
        }

        Commands::Tui { thread_id } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = thread_id
                .map(|id| resolve_tid(&git, &id))
                .transpose()?;
            let db_path = paths.git_forum.join("index.db");
            forum_tui::run(&git, &db_path, thread_id.as_deref())?;
        }

        Commands::Import { cmd } => match cmd {
            ImportCmd::GithubIssue {
                repo,
                issue,
                all,
                as_actor,
                dry_run,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let actor = resolve_actor(as_actor, &git);
                if dry_run {
                    if all {
                        let issues = github::list_issues(&repo)?;
                        for gh_issue in &issues {
                            let plan = github_import::plan_import(&git, &repo, gh_issue.number)?;
                            print_import_plan(&plan);
                        }
                    } else {
                        let issue_number = issue
                            .ok_or_else(|| ForumError::Config("--issue is required".into()))?;
                        let plan = github_import::plan_import(&git, &repo, issue_number)?;
                        print_import_plan(&plan);
                    }
                } else if all {
                    let results = github_import::import_all(&git, &repo, &actor, &clock)?;
                    for result in &results {
                        match result {
                            Ok(r) => {
                                println!("Imported {} <- {}", r.thread_id, r.github_url);
                                if r.state_changed {
                                    println!("  (closed)");
                                }
                                println!("  {} comment(s)", r.comments_imported);
                            }
                            Err((num, e)) => eprintln!("Failed #{num}: {e}"),
                        }
                    }
                } else {
                    let issue_number =
                        issue.ok_or_else(|| ForumError::Config("--issue is required".into()))?;
                    let result =
                        github_import::import_issue(&git, &repo, issue_number, &actor, &clock)?;
                    println!("Imported {} <- {}", result.thread_id, result.github_url);
                    if result.state_changed {
                        println!("  (closed)");
                    }
                    println!("  {} comment(s)", result.comments_imported);
                }
            }
        },

        Commands::Export { cmd } => match cmd {
            ExportCmd::GithubIssue {
                thread_id,
                repo,
                as_actor,
                dry_run,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let thread_id = resolve_tid(&git, &thread_id)?;
                let actor = resolve_actor(as_actor, &git);
                if dry_run {
                    let plan = github_export::plan_export(&git, &thread_id)?;
                    print_export_plan(&plan);
                } else {
                    let result =
                        github_export::export_issue(&git, &thread_id, &repo, &actor, &clock)?;
                    println!("Exported {} -> {}", thread_id, result.github_url);
                    println!(
                        "  Comments: {} created, {} updated, {} skipped",
                        result.comments_created, result.comments_updated, result.comments_skipped
                    );
                    if result.was_closed {
                        println!("  (GitHub issue closed)");
                    }
                }
            }
        },

        Commands::Ask { cmd } | Commands::Issue { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Issue, &clock)?;
        }
        Commands::Rfc { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Rfc, &clock)?;
        }
        Commands::Dec { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Dec, &clock)?;
        }
        Commands::Job { cmd } | Commands::Task { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Task, &clock)?;
        }

        Commands::New {
            kind,
            title,
            body,
            body_file,
            edit,
            branch,
            link_to,
            rel,
            as_actor,
            from_commit,
            from_thread,
            claim,
            question,
            objection,
            action,
            risk,
            summary,
            force,
        } => {
            let thread_kind = parse_thread_kind(&kind)?;
            run_thread_cmd(
                ThreadCmd::New {
                    title,
                    body,
                    body_file,
                    edit,
                    branch,
                    link_to,
                    rel,
                    as_actor,
                    from_commit,
                    from_thread,
                    claim,
                    question,
                    objection,
                    action,
                    risk,
                    summary,
                    force,
                },
                thread_kind,
                &clock,
            )?;
        }

        Commands::Close {
            thread_id,
            approve,
            as_actor,
            resolve_open_actions,
            link_to,
            rel,
            comment,
            fast_track,
            force,
        } => {
            run_state_shorthand(
                &thread_id,
                "closed",
                &approve,
                as_actor,
                resolve_open_actions,
                &link_to,
                rel.as_deref(),
                comment.as_deref(),
                fast_track,
                force,
                &clock,
            )?;
        }
        Commands::Pend {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            run_state_shorthand(
                &thread_id,
                "pending",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                fast_track,
                force,
                &clock,
            )?;
        }
        Commands::Accept {
            thread_id,
            approve,
            as_actor,
            link_to,
            rel,
            comment,
            fast_track,
            force,
        } => {
            run_state_shorthand(
                &thread_id,
                "accepted",
                &approve,
                as_actor,
                false,
                &link_to,
                rel.as_deref(),
                comment.as_deref(),
                fast_track,
                force,
                &clock,
            )?;
        }
        Commands::Propose {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            run_state_shorthand(
                &thread_id,
                "proposed",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                fast_track,
                force,
                &clock,
            )?;
        }
        Commands::Deprecate {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            run_state_shorthand(
                &thread_id,
                "deprecated",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                fast_track,
                force,
                &clock,
            )?;
        }
        Commands::Reject {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            run_state_shorthand(
                &thread_id,
                "rejected",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                fast_track,
                force,
                &clock,
            )?;
        }

        Commands::Ls {
            kind_positional,
            branch,
            kind,
            status,
        } => {
            let effective_kind = match (kind_positional.as_deref(), kind.as_deref()) {
                (Some(pos), Some(flag)) if pos != flag => {
                    return Err(ForumError::Config(format!(
                        "conflicting kind: positional '{pos}' vs --kind '{flag}'"
                    )));
                }
                (Some(pos), _) => Some(pos),
                (_, Some(flag)) => Some(flag),
                (None, None) => None,
            };
            let kind_filter = effective_kind.map(parse_thread_kind).transpose()?;
            let (git, _paths) = discover_repo_with_init_warning()?;
            let states = list_thread_states(&git, kind_filter, branch.as_deref())?;
            let filtered: Vec<&thread::ThreadState> = states
                .iter()
                .filter(|s| status.as_deref().is_none_or(|st| s.status == st))
                .collect();
            print!("{}", show::render_ls(&filtered));
        }

        Commands::Show {
            thread_id,
            what_next,
            compact,
            no_timeline,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            let state = thread::replay_thread(&git, &thread_id)?;
            if what_next {
                print!("{}", show::render_what_next(&state, &policy));
            } else {
                print!(
                    "{}",
                    show::render_show_with_options(
                        &state,
                        &show::ShowOptions {
                            compact,
                            no_timeline,
                            policy: Some(policy),
                        }
                    )
                );
            }
        }

        Commands::Diff { thread_id, rev } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let state = thread::replay_thread(&git, &thread_id)?;
            let output = diff::diff_body(&git, &state, rev.as_deref())?;
            println!("{output}");
        }

        Commands::Status { thread_id } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let state = thread::replay_thread(&git, &thread_id)?;
            print!("{}", show::render_status(&state));
        }

        Commands::Node { cmd } => match cmd {
            NodeCmd::Show { node_id } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let lookup = thread::find_node(&git, &node_id)?;
                print!("{}", show::render_node_show(&lookup));
            }
            NodeCmd::Add {
                thread_id,
                node_type,
                body_positional,
                body_flag,
                body_file,
                edit,
                reply_to,
                as_actor,
                force,
            } => run_shorthand_say(
                &thread_id,
                body_positional,
                body_flag,
                body_file,
                edit,
                reply_to,
                as_actor,
                node_type,
                force,
                &clock,
            )?,
        },

        Commands::Branch { cmd } => match cmd {
            BranchCmd::Bind {
                thread_id,
                branch,
                as_actor,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let thread_id = resolve_tid(&git, &thread_id)?;
                let actor = resolve_actor(as_actor, &git);
                branch_ops::set_branch(&git, &thread_id, Some(&branch), &actor, &clock)?;
                println!("{thread_id} -> branch {branch}");
            }
            BranchCmd::Clear {
                thread_id,
                as_actor,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let thread_id = resolve_tid(&git, &thread_id)?;
                let actor = resolve_actor(as_actor, &git);
                branch_ops::set_branch(&git, &thread_id, None, &actor, &clock)?;
                println!("{thread_id} -> branch <cleared>");
            }
        },

        Commands::Revise {
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
            cmd,
        } => run_revise_dispatch(
            cmd,
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
            &clock,
        )?,
        Commands::Claim {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Claim,
            force,
            &clock,
        )?,
        Commands::Question {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Question,
            force,
            &clock,
        )?,
        Commands::Objection {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Objection,
            force,
            &clock,
        )?,
        Commands::Summary {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Summary,
            force,
            &clock,
        )?,
        Commands::Action {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Action,
            force,
            &clock,
        )?,
        Commands::Risk {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Risk,
            force,
            &clock,
        )?,
        Commands::Review {
            thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            force,
        } => run_shorthand_say(
            &thread_id,
            body_positional,
            body_flag,
            body_file,
            edit,
            reply_to,
            as_actor,
            NodeType::Review,
            force,
            &clock,
        )?,

        Commands::Retract {
            thread_id,
            node_ids,
            as_actor,
        } => run_node_lifecycle_bulk(
            &thread_id,
            &node_ids,
            as_actor,
            git_forum::internal::event::EventType::Retract,
            "Retracted",
            &clock,
        )?,

        Commands::Resolve {
            thread_id,
            node_ids,
            as_actor,
        } => run_node_lifecycle_bulk(
            &thread_id,
            &node_ids,
            as_actor,
            git_forum::internal::event::EventType::Resolve,
            "Resolved",
            &clock,
        )?,

        Commands::Reopen {
            thread_id,
            node_ids,
            as_actor,
        } => run_node_lifecycle_bulk(
            &thread_id,
            &node_ids,
            as_actor,
            git_forum::internal::event::EventType::Reopen,
            "Reopened",
            &clock,
        )?,

        Commands::State {
            cmd,
            thread_id,
            new_state,
            approve,
            as_actor,
            resolve_open_actions,
            link_to,
            rel,
            comment,
            fast_track,
            force: _force,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            match cmd {
                Some(StateCmd::Bulk {
                    new_state,
                    thread_ids,
                    branch,
                    kind,
                    status,
                    approve,
                    as_actor,
                    resolve_open_actions,
                    dry_run,
                }) => {
                    let actor = resolve_actor(as_actor, &git);
                    let kind = parse_thread_kind_filter(kind.as_deref())?;
                    let report = run_bulk_state_change(
                        &git,
                        &policy,
                        &thread_ids,
                        BulkSelectors {
                            branch: branch.as_deref(),
                            kind,
                            status: status.as_deref(),
                        },
                        &new_state,
                        &approve,
                        &actor,
                        &clock,
                        state_change::StateChangeOptions {
                            resolve_open_actions,
                            ..Default::default()
                        },
                        dry_run,
                    )?;
                    print_bulk_report(&report);
                    if report.failures > 0 {
                        std::process::exit(1);
                    }
                }
                None => {
                    let thread_id = thread_id.ok_or_else(|| {
                        ForumError::Config(
                            "usage: git forum state <THREAD_ID> <NEW_STATE> [--approve <ACTOR_ID>]... [--resolve-open-actions]"
                                .into(),
                        )
                    })?;
                    let thread_id = resolve_tid(&git, &thread_id)?;
                    let new_state = new_state.ok_or_else(|| {
                        ForumError::Config(
                            "usage: git forum state <THREAD_ID> <NEW_STATE> [--approve <ACTOR_ID>]... [--resolve-open-actions]"
                                .into(),
                        )
                    })?;
                    let actor = resolve_actor(as_actor, &git);
                    let options = state_change::StateChangeOptions {
                        resolve_open_actions,
                        comment,
                    };
                    if fast_track {
                        let walked = state_change::fast_track_state(
                            &git, &thread_id, &new_state, &approve, &actor, &clock, &policy,
                            options,
                        )?;
                        for (i, step) in walked.iter().enumerate() {
                            let is_final = i == walked.len() - 1;
                            if is_final {
                                println!("{thread_id} -> {step}");
                            } else {
                                eprintln!("  {thread_id}: -> {step}");
                            }
                        }
                    } else {
                        state_change::change_state(
                            &git, &thread_id, &new_state, &approve, &actor, &clock, &policy,
                            options,
                        )?;
                        println!("{thread_id} -> {new_state}");
                    }
                    // Create links after state transition if requested
                    if !link_to.is_empty() {
                        let rel = rel.as_deref().ok_or_else(|| {
                            ForumError::Config("--rel is required when --link-to is used".into())
                        })?;
                        for target in &link_to {
                            let resolved_target = resolve_tid(&git, target)?;
                            evidence_ops::add_thread_link(
                                &git, &thread_id, &resolved_target, rel, &actor, &clock,
                            )?;
                        }
                    }
                    if let Ok(state) = thread::replay_thread(&git, &thread_id) {
                        eprintln!("{}", show::render_next_actions(&state, &policy));
                    }
                }
            }
        }

        Commands::Verify { thread_id } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            let report = verify::verify_thread(&git, &thread_id, &policy)?;
            if report.passed() {
                println!("{thread_id}: ready");
            } else {
                println!("{thread_id}: not ready");
                for v in &report.violations {
                    println!("  BLOCKED [{}] {}", v.rule, v.reason);
                }
                std::process::exit(1);
            }
        }

        Commands::Evidence { cmd } => match cmd {
            EvidenceCmd::Add {
                thread_id,
                kind,
                ref_targets,
                as_actor,
                force,
            } => {
                if ref_targets.is_empty() {
                    return Err(ForumError::Config("--ref is required".into()));
                }
                let (git, paths) = discover_repo_with_init_warning()?;
                let thread_id = resolve_tid(&git, &thread_id)?;
                let actor = resolve_actor(as_actor, &git);
                let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
                let state = thread::replay_thread(&git, &thread_id)?;
                let violations = operation_check::check_evidence(&policy, &state.status);
                apply_operation_checks(&violations, force, policy.checks.strict)?;
                for ref_target in &ref_targets {
                    let commit_sha = evidence_ops::add_evidence(
                        &git,
                        &thread_id,
                        kind.clone(),
                        ref_target,
                        None,
                        &actor,
                        &clock,
                    )?;
                    println!(
                        "Evidence added ({})",
                        &commit_sha[..commit_sha.len().min(8)]
                    );
                }
            }
        },

        Commands::Link {
            thread_id,
            target_thread_id,
            rel,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let target_thread_id = resolve_tid(&git, &target_thread_id)?;
            let actor = resolve_actor(as_actor, &git);
            evidence_ops::add_thread_link(
                &git,
                &thread_id,
                &target_thread_id,
                &rel,
                &actor,
                &clock,
            )?;
            println!("{thread_id} -> {target_thread_id} ({rel})");
        }

        Commands::Hook { cmd } => {
            let git = GitOps::discover()?;
            match cmd {
                HookCmd::Install { force } => {
                    let hook_path = hook::resolve_hook_path(&git)?;
                    hook::install_hook(&hook_path, force)?;
                }
                HookCmd::Uninstall => {
                    let hook_path = hook::resolve_hook_path(&git)?;
                    hook::uninstall_hook(&hook_path)?;
                }
                HookCmd::CheckCommitMsg { file } => {
                    let raw = fs::read_to_string(&file)?;
                    let comment_char = hook::get_comment_char(&git);
                    let cleaned = hook::strip_comments(&raw, comment_char);
                    let ids = hook::extract_thread_ids(&cleaned);
                    if ids.is_empty() {
                        eprintln!("git-forum: warning: no thread ID referenced in commit message");
                        return Ok(());
                    }
                    let result = hook::check_thread_refs(&git, &ids)?;
                    if result.has_errors() {
                        eprintln!("git-forum: commit message references non-existent thread(s):");
                        for id in &result.missing_ids {
                            eprintln!("  {id} — not found");
                        }
                        eprintln!(
                            "hint: create the thread first, or remove the reference from the commit message."
                        );
                        std::process::exit(1);
                    }
                }
            }
        }

        Commands::Policy { cmd } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            match cmd {
                PolicyCmd::Show => {
                    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
                    print!(
                        "{}",
                        git_forum::internal::policy::render_policy_show(&policy)
                    );
                }
                PolicyCmd::Lint => {
                    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
                    let diags = git_forum::internal::policy::lint_policy(&policy);
                    if diags.is_empty() {
                        println!("policy ok");
                    } else {
                        for d in &diags {
                            println!("{d}");
                        }
                    }
                }
                PolicyCmd::Check {
                    thread_id,
                    transition,
                } => {
                    let thread_id = resolve_tid(&git, &thread_id)?;
                    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
                    let state = thread::replay_thread(&git, &thread_id)?;
                    let parts: Vec<&str> = transition.splitn(2, "->").collect();
                    if parts.len() != 2 {
                        eprintln!("error: --transition must be 'from->to'");
                        std::process::exit(1);
                    }
                    let violations = git_forum::internal::policy::check_guards(
                        &policy,
                        &state,
                        parts[0],
                        parts[1],
                        &[],
                    );
                    if violations.is_empty() {
                        println!("transition {transition}: ok");
                    } else {
                        for v in &violations {
                            println!("FAIL [{}] {}", v.rule, v.reason);
                        }
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    Ok(())
}

fn resolve_reply_to(
    git: &GitOps,
    thread_id: &str,
    reply_to: Option<&str>,
) -> Result<Option<String>, ForumError> {
    match reply_to {
        Some(node_ref) => {
            let resolved = thread::resolve_node_id_in_thread(git, thread_id, node_ref)?;
            Ok(Some(resolved))
        }
        None => Ok(None),
    }
}

fn run_revise_cmd(
    cmd: ReviseCmd,
    clock: &dyn git_forum::internal::clock::Clock,
) -> Result<(), ForumError> {
    match cmd {
        ReviseCmd::Body {
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
            let actor = resolve_actor(as_actor, &git);
            let body_text = resolve_body_required(
                body,
                body_file,
                edit,
                &format!("Revise body for {thread_id}"),
            )?;

            let state = thread::replay_thread(&git, &thread_id)?;
            let violations = operation_check::check_revise(&policy, &state.status, true);
            apply_operation_checks(&violations, force, policy.checks.strict)?;

            write_ops::revise_body(&git, &thread_id, &body_text, &incorporates, &actor, clock)?;
            println!("Body revised for {thread_id}");
        }
        ReviseCmd::Node {
            thread_id,
            node_id,
            body,
            body_file,
            edit,
            as_actor,
            force,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
            let actor = resolve_actor(as_actor, &git);
            let body_text = resolve_body_required(
                body,
                body_file,
                edit,
                &format!("Revise node {node_id} in {thread_id}"),
            )?;

            let state = thread::replay_thread(&git, &thread_id)?;
            let violations = operation_check::check_revise(&policy, &state.status, false);
            apply_operation_checks(&violations, force, policy.checks.strict)?;

            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            write_ops::revise_node(&git, &thread_id, &resolved, &body_text, &actor, clock)?;
            println!("Revised {resolved}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_revise_dispatch(
    cmd: Option<ReviseCmd>,
    thread_id: Option<String>,
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    incorporates: Vec<String>,
    as_actor: Option<String>,
    force: bool,
    clock: &dyn git_forum::internal::clock::Clock,
) -> Result<(), ForumError> {
    match cmd {
        Some(subcmd) => run_revise_cmd(subcmd, clock),
        None => {
            let thread_id = thread_id.ok_or_else(|| {
                ForumError::Config(
                    "usage: git forum revise <THREAD_ID> --body <TEXT> | --body-file <PATH> | --edit".into(),
                )
            })?;
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
            let actor = resolve_actor(as_actor, &git);
            let body_text = resolve_body_required(
                body,
                body_file,
                edit,
                &format!("Revise body for {thread_id}"),
            )?;

            let state = thread::replay_thread(&git, &thread_id)?;
            let violations = operation_check::check_revise(&policy, &state.status, true);
            apply_operation_checks(&violations, force, policy.checks.strict)?;

            write_ops::revise_body(&git, &thread_id, &body_text, &incorporates, &actor, clock)?;
            println!("Body revised for {thread_id}");
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_shorthand_say(
    thread_id: &str,
    body_positional: Option<String>,
    body_flag: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    reply_to: Option<String>,
    as_actor: Option<String>,
    node_type: NodeType,
    force: bool,
    clock: &dyn git_forum::internal::clock::Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
    let actor = resolve_actor(as_actor, &git);
    let body = body_positional.or(body_flag);
    let body_text = resolve_body_required(
        body,
        body_file,
        edit,
        &format!("Compose a {node_type} node"),
    )?;

    // Operation check: is this node type allowed in the current state?
    let state = thread::replay_thread(&git, thread_id)?;
    let violations = operation_check::check_say(&policy, &state.status, node_type);
    apply_operation_checks(&violations, force, policy.checks.strict)?;

    let resolved_reply = resolve_reply_to(&git, thread_id, reply_to.as_deref())?;
    let node_id = write_ops::say_node(
        &git,
        thread_id,
        node_type,
        &body_text,
        &actor,
        clock,
        resolved_reply.as_deref(),
    )?;
    println!("Added {node_type} {node_id}");
    if let Ok(state) = thread::replay_thread(&git, thread_id) {
        eprintln!("{}", show::render_next_actions(&state, &policy));
    }
    Ok(())
}

fn resolve_body_required(
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    edit_hint: &str,
) -> Result<String, ForumError> {
    resolve_thread_body(body, body_file, edit, edit_hint)?
        .ok_or_else(|| ForumError::Config("--body, --body-file, or --edit is required".into()))
}

fn run_thread_cmd(
    cmd: ThreadCmd,
    kind: ThreadKind,
    clock: &dyn git_forum::internal::clock::Clock,
) -> Result<(), ForumError> {
    match cmd {
        ThreadCmd::New {
            title,
            body,
            body_file,
            edit,
            branch,
            link_to,
            rel,
            as_actor,
            from_commit,
            from_thread,
            claim,
            question,
            objection,
            action,
            risk,
            summary,
            force,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml")).unwrap_or_default();
            let actor = resolve_actor(as_actor, &git);

            let edit_hint = format!("Compose body for new {kind} thread");
            // Resolve title and body from --from-thread, --from-commit, or direct args
            let (effective_title, effective_body, commit_ref, source_thread) = if let Some(
                ref source_id,
            ) = from_thread
            {
                let source_id = &resolve_tid(&git, source_id)?;
                let source = thread::replay_thread(&git, source_id)?;
                // Reject RFC -> issue: an issue does not supersede an RFC
                if source.kind == ThreadKind::Rfc && kind == ThreadKind::Issue {
                    return Err(ForumError::Config(
                            "cannot create an issue --from-thread an RFC; an issue does not supersede an RFC. Use `git forum link --rel implements` instead.".into(),
                        ));
                }
                let t = title.unwrap_or_else(|| format!("v2: {}", source.title));
                let b =
                    resolve_thread_body(body, body_file, edit, &edit_hint)?.or(source.body.clone());
                (t, b, None, Some((source_id.clone(), source.kind)))
            } else if let Some(rev) = from_commit {
                let commit_sha = git.resolve_commit(&rev)?;
                let msg = git.run(&["log", "-1", "--format=%B", &commit_sha])?;
                let mut lines = msg.lines();
                let subject = lines.next().unwrap_or("").to_string();
                let body_text: String = lines
                    .skip_while(|l| l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let t = title.unwrap_or(subject);
                let b = resolve_thread_body(body, body_file, edit, &edit_hint)?.or(
                    if body_text.is_empty() {
                        None
                    } else {
                        Some(body_text)
                    },
                );
                (t, b, Some(commit_sha), None)
            } else {
                let t = title.ok_or_else(|| {
                    ForumError::Config(
                        "title is required (or use --from-commit / --from-thread)".into(),
                    )
                })?;
                let b = resolve_thread_body(body, body_file, edit, &edit_hint)?;
                (t, b, None, None)
            };
            // source_thread is now Option<(String, ThreadKind)>

            // Operation check: validate creation rules
            let violations = operation_check::check_create(
                &policy,
                kind,
                &effective_title,
                effective_body.as_deref(),
            );
            apply_operation_checks(&violations, force, policy.checks.strict)?;

            let thread_id = create::create_thread_with_branch(
                &git,
                kind,
                &effective_title,
                effective_body.as_deref(),
                branch.as_deref(),
                &actor,
                clock,
            )?;
            if !link_to.is_empty() {
                let rel = rel.as_deref().ok_or_else(|| {
                    ForumError::Config("--rel is required when --link-to is used".into())
                })?;
                for target in &link_to {
                    let resolved_target = resolve_tid(&git, target)?;
                    evidence_ops::add_thread_link(
                        &git,
                        &thread_id,
                        &resolved_target,
                        rel,
                        &actor,
                        clock,
                    )?;
                }
            }
            if let Some(sha) = commit_ref {
                evidence_ops::add_evidence(
                    &git,
                    &thread_id,
                    EvidenceKind::Commit,
                    &sha,
                    None,
                    &actor,
                    clock,
                )?;
            }
            // --from-thread: link new→old (supersedes), old→new (superseded-by),
            // auto-deprecate only when source is RFC and target is RFC
            if let Some((source_id, source_kind)) = source_thread {
                evidence_ops::add_thread_link(
                    &git,
                    &thread_id,
                    &source_id,
                    "supersedes",
                    &actor,
                    clock,
                )?;
                evidence_ops::add_thread_link(
                    &git,
                    &source_id,
                    &thread_id,
                    "superseded-by",
                    &actor,
                    clock,
                )?;
                if source_kind == ThreadKind::Rfc && kind == ThreadKind::Rfc {
                    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
                    state_change::change_state(
                        &git,
                        &source_id,
                        "deprecated",
                        &[],
                        &actor,
                        clock,
                        &policy,
                        state_change::StateChangeOptions::default(),
                    )?;
                }
                println!("Created {thread_id} (supersedes {source_id})");
            } else {
                println!("Created {thread_id}");
            }
            // Add inline nodes
            let inline_nodes: Vec<(NodeType, &[String])> = vec![
                (NodeType::Claim, &claim),
                (NodeType::Question, &question),
                (NodeType::Objection, &objection),
                (NodeType::Action, &action),
                (NodeType::Risk, &risk),
                (NodeType::Summary, &summary),
            ];
            for (node_type, bodies) in &inline_nodes {
                for body_text in *bodies {
                    let node_id = write_ops::say_node(
                        &git, &thread_id, *node_type, body_text, &actor, clock, None,
                    )?;
                    println!("Added {node_type} {node_id}");
                }
            }
        }
        ThreadCmd::Ls { branch } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let states = list_thread_states(&git, Some(kind), branch.as_deref())?;
            let refs: Vec<&thread::ThreadState> = states.iter().collect();
            print!("{}", show::render_ls(&refs));
        }
        ThreadCmd::Revise {
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
            cmd,
        } => run_revise_dispatch(
            cmd,
            thread_id,
            body,
            body_file,
            edit,
            incorporates,
            as_actor,
            force,
            clock,
        )?,
        ThreadCmd::Close {
            thread_id,
            approve,
            as_actor,
            resolve_open_actions,
            link_to,
            rel,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "closed",
                &approve,
                as_actor,
                resolve_open_actions,
                &link_to,
                rel.as_deref(),
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
        ThreadCmd::Pend {
            thread_id,
            as_actor,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "pending",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
        ThreadCmd::Reopen {
            thread_id,
            as_actor,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "open",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
        ThreadCmd::Reject {
            thread_id,
            as_actor,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "rejected",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
        ThreadCmd::Accept {
            thread_id,
            approve,
            as_actor,
            link_to,
            rel,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "accepted",
                &approve,
                as_actor,
                false,
                &link_to,
                rel.as_deref(),
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
        ThreadCmd::Propose {
            thread_id,
            as_actor,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "proposed",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
        ThreadCmd::Deprecate {
            thread_id,
            as_actor,
            comment,
        } => {
            run_state_shorthand(
                &thread_id,
                "deprecated",
                &[],
                as_actor,
                false,
                &[],
                None,
                comment.as_deref(),
                false,
                false,
                clock,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn run_state_shorthand(
    thread_id: &str,
    new_state: &str,
    approve: &[String],
    as_actor: Option<String>,
    resolve_open_actions: bool,
    link_to: &[String],
    rel: Option<&str>,
    comment: Option<&str>,
    fast_track: bool,
    _force: bool,
    clock: &dyn git_forum::internal::clock::Clock,
) -> Result<(), ForumError> {
    let (git, paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
    let actor = resolve_actor(as_actor, &git);
    let options = state_change::StateChangeOptions {
        resolve_open_actions,
        comment: comment.map(|s| s.to_string()),
    };
    if fast_track {
        let walked = state_change::fast_track_state(
            &git, thread_id, new_state, approve, &actor, clock, &policy, options,
        )?;
        for (i, step) in walked.iter().enumerate() {
            let is_final = i == walked.len() - 1;
            if is_final {
                println!("{thread_id} -> {step}");
            } else {
                eprintln!("  {thread_id}: -> {step}");
            }
        }
    } else {
        state_change::change_state(
            &git, thread_id, new_state, approve, &actor, clock, &policy, options,
        )?;
        println!("{thread_id} -> {new_state}");
    }
    if !link_to.is_empty() {
        let rel = rel
            .ok_or_else(|| ForumError::Config("--rel is required when --link-to is used".into()))?;
        for target in link_to {
            let resolved_target = resolve_tid(&git, target)?;
            evidence_ops::add_thread_link(&git, thread_id, &resolved_target, rel, &actor, clock)?;
        }
    }
    if let Ok(state) = thread::replay_thread(&git, thread_id) {
        eprintln!("{}", show::render_next_actions(&state, &policy));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct BulkSelectors<'a> {
    branch: Option<&'a str>,
    kind: Option<ThreadKind>,
    status: Option<&'a str>,
}

struct BulkStateOutcome {
    thread_id: String,
    from_state: String,
    to_state: String,
    ok: bool,
    dry_run: bool,
    detail: Option<String>,
}

struct BulkStateReport {
    outcomes: Vec<BulkStateOutcome>,
    failures: usize,
}

fn list_thread_states(
    git: &GitOps,
    kind: Option<ThreadKind>,
    branch: Option<&str>,
) -> Result<Vec<thread::ThreadState>, ForumError> {
    let all_ids = thread::list_thread_ids(git)?;
    let mut states = Vec::new();
    for id in &all_ids {
        let state = thread::replay_thread(git, id)?;
        if thread_matches_filters(&state, kind, branch, None) {
            states.push(state);
        }
    }
    states.sort_by_key(|s| s.created_at);
    Ok(states)
}

fn thread_matches_filters(
    state: &thread::ThreadState,
    kind: Option<ThreadKind>,
    branch: Option<&str>,
    status: Option<&str>,
) -> bool {
    kind.is_none_or(|kind| state.kind == kind)
        && branch.is_none_or(|branch| state.branch.as_deref() == Some(branch))
        && status.is_none_or(|status| state.status == status)
}

fn parse_thread_kind(kind: &str) -> Result<ThreadKind, ForumError> {
    match kind {
        "ask" | "issue" => Ok(ThreadKind::Issue),
        "rfc" => Ok(ThreadKind::Rfc),
        "dec" => Ok(ThreadKind::Dec),
        "job" | "task" => Ok(ThreadKind::Task),
        other => Err(ForumError::Config(format!(
            "unknown kind '{other}'; valid: ask, rfc, dec, job (aliases: issue, task)"
        ))),
    }
}

fn parse_thread_kind_filter(kind: Option<&str>) -> Result<Option<ThreadKind>, ForumError> {
    match kind {
        None => Ok(None),
        Some("ask") | Some("issue") => Ok(Some(ThreadKind::Issue)),
        Some("rfc") => Ok(Some(ThreadKind::Rfc)),
        Some("dec") => Ok(Some(ThreadKind::Dec)),
        Some("job") | Some("task") => Ok(Some(ThreadKind::Task)),
        Some(other) => Err(ForumError::Config(format!(
            "unknown kind '{other}'; valid: ask, rfc, dec, job (aliases: issue, task)"
        ))),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_bulk_state_change(
    git: &GitOps,
    policy: &Policy,
    explicit_ids: &[String],
    selectors: BulkSelectors<'_>,
    new_state: &str,
    approve: &[String],
    actor: &str,
    clock: &dyn git_forum::internal::clock::Clock,
    options: state_change::StateChangeOptions,
    dry_run: bool,
) -> Result<BulkStateReport, ForumError> {
    if explicit_ids.is_empty()
        && selectors.branch.is_none()
        && selectors.kind.is_none()
        && selectors.status.is_none()
    {
        return Err(ForumError::Config(
            "state bulk requires at least one THREAD_ID or selector (--branch/--kind/--status)"
                .into(),
        ));
    }

    let candidate_ids = if explicit_ids.is_empty() {
        thread::list_thread_ids(git)?
    } else {
        explicit_ids.to_vec()
    };

    let mut outcomes = Vec::new();
    for thread_id in candidate_ids {
        let state = match thread::replay_thread(git, &thread_id) {
            Ok(state) => state,
            Err(err) => {
                outcomes.push(BulkStateOutcome {
                    thread_id,
                    from_state: "?".into(),
                    to_state: new_state.to_string(),
                    ok: false,
                    dry_run,
                    detail: Some(err.to_string()),
                });
                continue;
            }
        };

        if !thread_matches_filters(&state, selectors.kind, selectors.branch, selectors.status) {
            continue;
        }

        match state_change::prepare_state_change(
            git,
            &thread_id,
            new_state,
            approve,
            clock,
            policy,
            options.clone(),
        ) {
            Ok(plan) => {
                if !dry_run {
                    if let Err(err) = state_change::change_state(
                        git,
                        &thread_id,
                        new_state,
                        approve,
                        actor,
                        clock,
                        policy,
                        options.clone(),
                    ) {
                        outcomes.push(BulkStateOutcome {
                            thread_id,
                            from_state: state.status,
                            to_state: new_state.to_string(),
                            ok: false,
                            dry_run,
                            detail: Some(err.to_string()),
                        });
                        continue;
                    }
                }
                outcomes.push(BulkStateOutcome {
                    thread_id,
                    from_state: plan.from_state,
                    to_state: new_state.to_string(),
                    ok: true,
                    dry_run,
                    detail: None,
                });
            }
            Err(err) => outcomes.push(BulkStateOutcome {
                thread_id,
                from_state: state.status,
                to_state: new_state.to_string(),
                ok: false,
                dry_run,
                detail: Some(err.to_string()),
            }),
        }
    }

    if outcomes.is_empty() {
        return Err(ForumError::Config(
            "state bulk matched no threads for the given selectors".into(),
        ));
    }

    let failures = outcomes.iter().filter(|o| !o.ok).count();
    Ok(BulkStateReport { outcomes, failures })
}

fn print_bulk_report(report: &BulkStateReport) {
    for outcome in &report.outcomes {
        let marker = match (outcome.dry_run, outcome.ok) {
            (false, true) => "OK",
            (false, false) => "FAIL",
            (true, true) => "WOULD-OK",
            (true, false) => "WOULD-FAIL",
        };
        match &outcome.detail {
            Some(detail) => println!(
                "{marker:<10} {:<12} {} -> {}  {}",
                outcome.thread_id, outcome.from_state, outcome.to_state, detail
            ),
            None => println!(
                "{marker:<10} {:<12} {} -> {}",
                outcome.thread_id, outcome.from_state, outcome.to_state
            ),
        }
    }
}

fn resolve_actor(as_actor: Option<String>, git: &GitOps) -> String {
    as_actor.unwrap_or_else(|| actor::current_actor(git, git.default_actor()))
}

fn run_node_lifecycle_bulk(
    thread_id: &str,
    node_ids: &[String],
    as_actor: Option<String>,
    event_type: git_forum::internal::event::EventType,
    label: &str,
    clock: &dyn git_forum::internal::clock::Clock,
) -> Result<(), ForumError> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let thread_id = &resolve_tid(&git, thread_id)?;
    let actor = resolve_actor(as_actor, &git);
    let mut failures = 0usize;
    for node_id in node_ids {
        let resolved = match thread::resolve_node_id_in_thread(&git, thread_id, node_id) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: {node_id}: {e}");
                failures += 1;
                continue;
            }
        };
        match write_ops::node_lifecycle(&git, thread_id, &resolved, &actor, clock, event_type) {
            Ok(()) => println!("{label} {resolved}"),
            Err(e) => {
                eprintln!("error: {resolved}: {e}");
                failures += 1;
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
    if event_type == git_forum::internal::event::EventType::Retract {
        eprintln!("note: retract is a soft-delete — the original content remains in git history");
    }
    Ok(())
}

fn discover_repo_with_init_warning() -> Result<(GitOps, RepoPaths), ForumError> {
    let mut git = GitOps::discover()?;
    let git_dir = git.git_dir()?;
    let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
    if !is_forum_initialized(&paths, &git) {
        eprintln!(
            "warning: git-forum is not initialized in this repository; run `git forum init` first"
        );
    }
    // Load local config and apply settings.
    let local_cfg = config::load_local_config(&paths).unwrap_or_default();
    if let Some(identity) = local_cfg.commit_identity {
        git.set_commit_identity(identity);
    }
    if let Some(default_actor) = local_cfg.default_actor {
        git.set_default_actor(default_actor);
    }
    Ok((git, paths))
}

fn is_forum_initialized(paths: &RepoPaths, git: &GitOps) -> bool {
    // Primary check: config files created by `git forum init`.
    if paths.dot_forum.join("policy.toml").is_file() && paths.git_forum.join("logs").is_dir() {
        return true;
    }
    // Fallback: forum refs already exist (repo is functional even without explicit init).
    git.list_refs("refs/forum/threads/")
        .map(|refs| !refs.is_empty())
        .unwrap_or(false)
}

/// Extract the subcommand name from a clap "unrecognized subcommand" error message.
fn parse_unrecognized_subcommand(msg: &str) -> Option<String> {
    // clap format: "error: unrecognized subcommand 'foo'"
    let marker = "unrecognized subcommand '";
    let start = msg.find(marker)? + marker.len();
    let end = msg[start..].find('\'')?;
    Some(msg[start..start + end].to_string())
}

/// Return a custom hint for known unrecognized subcommands.
fn subcommand_hint(sub: &str) -> Option<&'static str> {
    match sub {
        "say" => Some(
            "\"say\" is an internal module, not a CLI command. \
             Use node shorthands instead:\n  \
             git forum claim, question, objection, summary, action, risk, review\n  \
             or: git forum node add <THREAD> --type <TYPE> \"body\"",
        ),
        "revise-body" => Some(
            "use `git forum revise <THREAD_ID>` to revise a thread body, \
             or `git forum revise node <NODE_ID> <THREAD_ID>` to revise a node",
        ),
        "create" => Some("use `git forum new <kind> \"title\"` to create a thread"),
        "add" => Some("use `git forum node add <THREAD> --type <TYPE> \"body\"` to add a node"),
        _ => None,
    }
}

fn resolve_thread_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
    edit: bool,
    edit_hint: &str,
) -> Result<Option<String>, ForumError> {
    if edit {
        return Ok(Some(editor::edit_body(edit_hint)?));
    }
    match (body, body_file) {
        (Some(body), None) if body == "-" => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            Ok(Some(buf))
        }
        (Some(body), None) => Ok(Some(body)),
        (None, Some(path)) => Ok(Some(fs::read_to_string(path)?)),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => unreachable!("clap enforces body/body-file conflicts"),
    }
}

fn print_import_plan(plan: &github_import::ImportPlan) {
    if let Some(ref existing) = plan.already_imported {
        println!(
            "[SKIP] {} — already imported as {existing}",
            plan.github_url
        );
        return;
    }
    println!("[DRY-RUN] Would import: {}", plan.github_url);
    println!("  Title: {}", plan.title);
    println!("  Comments: {}", plan.comment_count);
    if plan.would_close {
        println!("  State: would be closed after import");
    }
}

/// Resolve a user-supplied thread reference to its canonical full ID.
///
/// Wraps `thread::resolve_thread_id` for use from CLI command handlers.
fn resolve_tid(git: &GitOps, user_input: &str) -> Result<String, ForumError> {
    thread::resolve_thread_id(git, user_input)
}

fn print_export_plan(plan: &github_export::ExportPlan) {
    if plan.already_exported {
        println!(
            "[RE-EXPORT] {} -> {} (will update existing)",
            plan.thread_id,
            plan.existing_github_url.as_deref().unwrap_or("?")
        );
    } else {
        println!("[DRY-RUN] Would export: {}", plan.thread_id);
    }
    println!("  Title: {}", plan.title);
    println!("  Nodes: {}", plan.node_count);
    if plan.would_close {
        println!("  State: GitHub issue would be closed");
    }
}
