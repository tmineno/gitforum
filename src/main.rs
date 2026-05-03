use std::fs;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use git_forum::internal::actor;
use git_forum::internal::clock::SystemClock;
use git_forum::internal::commands;
use git_forum::internal::commands::branch;
use git_forum::internal::commands::brief;
use git_forum::internal::commands::bulk::{
    list_thread_states, print_bulk_report, run_bulk_state_change, BulkSelectors,
};
use git_forum::internal::commands::diff;
use git_forum::internal::commands::doctor;
use git_forum::internal::commands::hook;
use git_forum::internal::commands::ls;
use git_forum::internal::commands::node_bulk::{run_node_lifecycle_bulk, NodeLifecycleOp};
use git_forum::internal::commands::revise::{self as revise_cmd, ReviseCmd};
use git_forum::internal::commands::shared::{
    discover_repo_with_init_warning, parse_since_date, parse_thread_kind, parse_thread_kind_filter,
    parse_unrecognized_subcommand, resolve_actor, resolve_tid, subcommand_hint,
    terminal_state_date,
};
use git_forum::internal::commands::shorthand_say::run_shorthand_say;
use git_forum::internal::commands::show;
use git_forum::internal::commands::state::run_state_shorthand;
use git_forum::internal::commands::state::StateCmd;
use git_forum::internal::commands::thread_new::ThreadCmd;
use git_forum::internal::commands::thread_new::ThreadNewInline;
use git_forum::internal::commands::thread_new::{
    parse_lifecycle, preset_lookup, valid_preset_names,
};
use git_forum::internal::commands::verify;
use git_forum::internal::commands::Context;
use git_forum::internal::config;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::error::ForumError;
use git_forum::internal::event;
use git_forum::internal::event::NodeType;
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::github;
use git_forum::internal::github_export;
use git_forum::internal::github_import;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::lint_emit::{self, LintEmitter};
use git_forum::internal::policy::Policy;
use git_forum::internal::purge;
use git_forum::internal::reindex;
use git_forum::internal::repair;
use git_forum::internal::state_change;
use git_forum::internal::thread;
use git_forum::internal::timeline;
use git_forum::internal::tui as forum_tui;

const GROUPED_HELP: &str = "\
These are common git-forum commands:

setup and repo health
   init               Initialize a git-forum repository
   doctor             Diagnose repo health (config, index, refs)
   repair             Detect and fix thread ID conflicts with a remote
   reindex            Rebuild local index from Git refs
   prune-orphans      Delete thread refs that have no valid create event
   prune-stale-events Drop events whose target_node_id references a vanished node
   migrate            Rewrite a 1.x repo to the 2.0 storage format

create and browse threads
   new         Create a new thread via kind preset (rfc/dec/task/issue/bug)
   thread      Canonical lifecycle/tag form (power-user / scripts)
   ls          List threads (filter by lifecycle, tag, status, or branch)
   show        Show thread details (use --what-next for diagnostics)
   diff        Show diff between body revisions
   search      Search threads and nodes
   shortlog    List threads resolved since a date or tag
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
   hook        Manage git-forum hooks (commit-msg, post-checkout)

interactive
   tui         Open the interactive TUI

import / export
   import      Import from external sources
   export      Export to external platforms

state shorthands (lifecycle-aware: close/accept/propose/pend/reject/withdraw/deprecate)
   close       proposal: rejected (use accept) | execution/record: -> done
   accept      proposal/record: -> done | execution: rejected (use close)
   propose     proposal: draft -> open | other lifecycles: rejected
   pend        execution: -> working | other lifecycles: rejected
   reject      any lifecycle: -> rejected
   withdraw    proposal: -> withdrawn | other lifecycles: rejected
   deprecate   any lifecycle: -> deprecated

node shorthands (convenience aliases for 'node add <ID> --type <type>')
   comment     node add --type comment (canonical 2.0 form)
   objection   node add --type objection
   action      node add --type action
   claim, question, summary, risk, review — deprecated aliases for `comment`

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
        /// Surface every silent replay no-op (unknown target node, etc.) as FAIL.
        /// Intended for migration verification and CI gates; default doctor stays
        /// lenient so historical write-side mistakes don't fail every run.
        #[arg(long)]
        strict: bool,
    },
    /// Rebuild local index from Git refs
    Reindex,
    /// Delete thread refs whose oldest commit is not a valid event.json
    PruneOrphans {
        /// Actually delete the orphan refs (default is dry-run preview)
        #[arg(long)]
        apply: bool,
    },
    /// Drop events whose target_node_id references a vanished node.
    /// Surfaces of `git forum doctor --strict` that aren't ref-level damage.
    PruneStaleEvents {
        /// Actually rewrite affected thread chains (default is dry-run preview)
        #[arg(long)]
        apply: bool,
    },
    /// Migrate a 1.x repo to the 2.0 storage format (ADR-004 / SPEC-2.0 §10)
    Migrate {
        /// Report planned changes without writing anything
        #[arg(long)]
        dry_run: bool,
        /// Override the actor recorded on synthetic facet_set events
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Detect and fix thread ID conflicts with a remote, or repair
    /// historical workflow violations (#uu9wxn1d) via append-only
    /// corrective `state` events.
    Repair {
        /// Remote to compare against (default mode: ID conflict repair)
        #[arg(long, default_value = "origin")]
        remote: String,
        /// Show what would be repaired without modifying
        #[arg(long)]
        dry_run: bool,
        /// Switch to workflow-violation repair mode (#uu9wxn1d): scan
        /// every thread for `InvalidTransition` strict-replay issues
        /// and append corrective `state` events to bring each chain
        /// onto a legal terminal path. Always defaults to a dry-run
        /// preview; pair with `--apply` to write events.
        #[arg(long = "workflow-violations")]
        workflow_violations: bool,
        /// Write the corrective events (with `--workflow-violations`).
        /// Without `--apply`, the command prints the per-thread plan
        /// and exits without modifying any refs.
        #[arg(long, requires = "workflow_violations")]
        apply: bool,
        /// Override the actor recorded on the corrective events. By
        /// default the local config's actor is used.
        #[arg(long = "as", value_name = "ACTOR", requires = "workflow_violations")]
        as_actor: Option<String>,
    },
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
    /// Canonical thread sub-commands (lifecycle/tag — power-user form)
    Thread {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// Create a new thread via kind preset (rfc, dec, task, issue, bug)
    New {
        /// Kind preset: rfc, dec, task, issue, bug
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
    /// List threads that reached terminal state since a date or tag
    Shortlog {
        /// Show threads resolved after this date (ISO) or git revision (tag/SHA)
        #[arg(long, value_name = "DATE_OR_REV")]
        since: String,
        /// Filter by thread kind (ask, rfc, dec, job)
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
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
    /// Withdraw a thread (shorthand for state <ID> withdrawn)
    Withdraw {
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
        /// Advisory: list direct incoming `implements` children (one hop, no recursion).
        /// Cross-thread display only — never gates an operation.
        #[arg(long)]
        tree: bool,
    },
    /// Show event timeline for a thread
    Log {
        thread_id: String,
        /// Show newest events first
        #[arg(long)]
        reverse: bool,
        /// Limit to the last N events
        #[arg(short = 'n', long)]
        last: Option<usize>,
        /// Show only events after this date (ISO date, RFC 3339, or git revision)
        #[arg(long)]
        since: Option<String>,
        /// Filter events by displayed type (e.g. review, state, claim, revise-body)
        #[arg(long = "type", value_name = "TYPE")]
        event_type: Option<String>,
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
    /// Read-only single-thread digest (RFC-5wf2v8hv).
    ///
    /// Reads only the named thread's events. Outgoing-link summary is grouped
    /// by relation; incoming counts come from the SQLite reverse-link index.
    /// Never reads linked threads' bodies, titles, or states.
    Brief {
        thread_id: String,
        /// Emit a stable v1 JSON object instead of plaintext.
        #[arg(long)]
        json: bool,
    },
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
        #[arg(long = "incorporates", alias = "incorporate", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
        #[command(subcommand)]
        cmd: Option<ReviseCmd>,
    },
    /// Add a comment node to a thread (2.0 canonical)
    Comment {
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
    /// Reopen resolved/retracted node(s), or reopen a closed thread (when no NODE_ID given)
    Reopen {
        thread_id: String,
        #[arg(
            num_args = 1..,
            value_name = "NODE_ID",
            help = "Full node ID(s) or unique prefix within the thread; omit to reopen the thread itself"
        )]
        node_ids: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Change the type of an existing node
    Retype {
        thread_id: String,
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or unique prefix within the thread"
        )]
        node_id: String,
        /// New node type (claim, question, objection, evidence, summary, action, risk, review, alternative, assumption)
        #[arg(long = "type", value_name = "TYPE")]
        new_type: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        /// Bypass warning-level operation checks (does not bypass errors)
        #[arg(long)]
        force: bool,
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
    /// Manage git-forum hooks (commit-msg, post-checkout)
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
    /// Install git-forum hooks (commit-msg + post-checkout)
    Install {
        /// Overwrite existing hooks without backup
        #[arg(long)]
        force: bool,
    },
    /// Remove git-forum hooks
    Uninstall,
    /// Validate thread references in a commit message file (used by the hook)
    CheckCommitMsg {
        /// Path to the commit message file (provided by Git)
        file: PathBuf,
    },
    /// Repair missing blob references in the git index (used by post-checkout hook)
    FixIndex,
    /// Initialize git-forum in a new worktree (used by post-checkout hook)
    WorktreeInit,
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

/// Apply operation check violations: print to stderr, block on errors.
/// Returns Ok(()) if the operation should proceed, Err if blocked.
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
            Some(
                "state" | "close" | "reject" | "accept" | "propose" | "deprecate" | "pend"
                | "withdraw",
            ) => {
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

    // Install the throttled lint emitter as early as possible so the
    // first Policy::load anywhere downstream renders paths repo-relative
    // and honours the on-disk suppression cache. Failure to discover a
    // repo (e.g. running `git forum --help` outside a repo) is fine —
    // we fall back to the in-memory default emitter (#6k7hq482).
    if let Ok(git) = GitOps::discover() {
        if let Ok(git_dir) = git.git_dir() {
            let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
            lint_emit::install(LintEmitter::new_for_paths(&paths));
        }
    }

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
                let thread_ids = git.list_refs("refs/forum/threads/").unwrap_or_default();
                if !thread_ids.is_empty() {
                    let db_path = paths.git_forum.join("index.db");
                    match reindex::run_reindex(&git, &db_path) {
                        Ok(report) => {
                            eprintln!("Reindexed {} threads", report.threads_replayed.len());
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
            hook::install_all_hooks(&git, false)?;
        }

        Commands::Doctor { verbose, strict } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let report = if strict {
                doctor::run_doctor_strict(&git, &paths)?
            } else {
                doctor::run_doctor(&git, &paths)?
            };

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

            // Cross-thread advisories (SPEC-2.0 §B.6) — informational only,
            // do not affect exit status per CORE-VALUE.md "Advisories".
            for advisory in &report.advisories {
                println!("[ADV ] {advisory}");
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

        Commands::PruneOrphans { apply } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let orphans = git_forum::internal::prune::scan(&git)?;
            if orphans.is_empty() {
                println!("No orphan thread refs found.");
                return Ok(());
            }
            println!("Orphan thread refs ({}):", orphans.len());
            for orphan in &orphans {
                println!("  {} ({})", orphan.ref_name, orphan.thread_id);
            }
            if !apply {
                println!("\nDry run — re-run with --apply to delete these refs.");
                return Ok(());
            }
            git_forum::internal::prune::delete(&git, &orphans)?;
            println!("\nDeleted {} orphan ref(s).", orphans.len());
            // Stale rows remain in the index until the operator runs reindex;
            // mention it inline so they don't have to remember.
            let db_path = paths.git_forum.join("index.db");
            if db_path.exists() {
                println!("hint: run `git forum reindex` to drop stale index rows.");
            }
        }

        Commands::PruneStaleEvents { apply } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let plans = git_forum::internal::prune::scan_stale_events(&git)?;
            if plans.is_empty() {
                println!("No stale-target events found.");
                return Ok(());
            }
            let total_threads = plans.len();
            let total_events: usize = plans.iter().map(|p| p.events_to_drop.len()).sum();
            let total_targets: usize = plans.iter().map(|p| p.orphan_target_count).sum();
            println!(
                "Stale-target events: {total_events} event(s) across {total_threads} thread(s) (referencing {total_targets} vanished node(s))"
            );
            for plan in &plans {
                println!(
                    "  {}: drop {} event(s) ({} orphan target(s))",
                    plan.thread_id,
                    plan.events_to_drop.len(),
                    plan.orphan_target_count
                );
                for sha in &plan.events_to_drop {
                    println!("    - {sha}");
                }
            }
            if !apply {
                println!(
                    "\nDry run — re-run with --apply to rewrite these threads. Backup: refs/forum/threads/* are local-only on this clone."
                );
                return Ok(());
            }
            let dropped = git_forum::internal::prune::apply_stale_event_plans(&git, &plans)?;
            println!("\nDropped {dropped} event(s) from {total_threads} thread(s).");
            let db_path = paths.git_forum.join("index.db");
            if db_path.exists() {
                println!(
                    "hint: run `git forum reindex` to refresh index rows for affected threads."
                );
            }
        }

        Commands::Migrate { dry_run, as_actor } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let actor = as_actor
                .or_else(|| git.default_actor().map(str::to_string))
                .unwrap_or_else(|| "system/migrate".to_string());
            let outcome =
                git_forum::internal::commands::migrate::run(&git, &paths, &actor, dry_run)?;
            // After a write run, the local index can drift (refs renamed,
            // events rewritten). Rebuild it so subsequent reads see the
            // migrated state.
            if !dry_run {
                let db_path = paths.git_forum.join("index.db");
                if db_path.exists() {
                    if let Err(e) = reindex::run_reindex(&git, &db_path) {
                        eprintln!("warning: reindex failed after migrate: {e}");
                    }
                }
            }
            // Non-zero exit if any thread plan failed — currently `run`
            // already printed warnings; the outcome counts both successes
            // and skips. Normal exit unless we want hard errors later.
            let _ = outcome;
        }

        Commands::Repair {
            remote,
            dry_run,
            workflow_violations,
            apply,
            as_actor,
        } => {
            if workflow_violations {
                git_forum::internal::commands::repair_workflow::run_workflow_repair(
                    apply, as_actor, &clock,
                )?;
            } else {
                let (git, paths) = discover_repo_with_init_warning()?;
                let report = repair::repair_conflicts(&git, &remote, dry_run)?;
                if report.reallocated.is_empty() {
                    println!("No thread ID conflicts found with remote '{remote}'");
                } else if dry_run {
                    println!("Found {} conflict(s):", report.reallocated.len());
                    for (old_id, _) in &report.reallocated {
                        println!("  {old_id}");
                    }
                    println!("\nRun without --dry-run to re-allocate conflicting thread IDs");
                } else {
                    for (old_id, new_id) in &report.reallocated {
                        println!("Reallocated {old_id} -> {new_id}");
                    }
                    // Reindex if index exists
                    let db_path = paths.git_forum.join("index.db");
                    if db_path.exists() {
                        if let Err(e) = reindex::run_reindex(&git, &db_path) {
                            eprintln!("warning: reindex failed: {e}");
                        }
                    }
                    println!("\nYou can now push with: git push");
                }
                for err in &report.errors {
                    eprintln!("warning: {err}");
                }
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
            // SPEC-2.0 §9.2 / Track D: legacy `kind:<name>` query tokens
            // auto-translate to lifecycle/tag pairs for one minor release,
            // emitting a deprecation warning so scripts have a runway to
            // migrate to the canonical vocabulary.
            let translated = translate_legacy_kind_query(&query);
            let results = index::search_threads(&conn, &translated)?;
            print!("{}", ls::render_search_results(&results));
        }

        Commands::Tui { thread_id } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = thread_id.map(|id| resolve_tid(&git, &id)).transpose()?;
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

        Commands::Thread {
            cmd:
                ThreadCmd::New {
                    title,
                    lifecycle,
                    tag,
                    body,
                    body_file,
                    edit,
                    branch,
                    link_to,
                    rel,
                    as_actor,
                    from_commit,
                    from_thread,
                    force,
                },
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            let lifecycle = parse_lifecycle(&lifecycle)?;
            commands::thread_new::run(
                commands::thread_new::ThreadNewArgs {
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
                    inline: ThreadNewInline::default(),
                    force,
                    lifecycle,
                    tags: tag,
                },
                &ctx,
            )?;
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
            let ctx = Context::discover(Box::new(SystemClock))?;
            let preset = preset_lookup(&kind).ok_or_else(|| {
                ForumError::Config(format!(
                    "unknown kind '{kind}'; valid presets: {}",
                    valid_preset_names(),
                ))
            })?;
            commands::thread_new::run(
                commands::thread_new::ThreadNewArgs {
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
                    inline: ThreadNewInline {
                        claim,
                        question,
                        objection,
                        action,
                        risk,
                        summary,
                    },
                    force,
                    lifecycle: preset.lifecycle,
                    tags: preset.tags.iter().map(|s| s.to_string()).collect(),
                },
                &ctx,
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
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "closed".into(),
                    approve,
                    as_actor,
                    resolve_open_actions,
                    link_to,
                    rel,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
            )?;
        }
        Commands::Pend {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "pending".into(),
                    approve: vec![],
                    as_actor,
                    resolve_open_actions: false,
                    link_to: vec![],
                    rel: None,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
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
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "accepted".into(),
                    approve,
                    as_actor,
                    resolve_open_actions: false,
                    link_to,
                    rel,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
            )?;
        }
        Commands::Propose {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "proposed".into(),
                    approve: vec![],
                    as_actor,
                    resolve_open_actions: false,
                    link_to: vec![],
                    rel: None,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
            )?;
        }
        Commands::Deprecate {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "deprecated".into(),
                    approve: vec![],
                    as_actor,
                    resolve_open_actions: false,
                    link_to: vec![],
                    rel: None,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
            )?;
        }
        Commands::Reject {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "rejected".into(),
                    approve: vec![],
                    as_actor,
                    resolve_open_actions: false,
                    link_to: vec![],
                    rel: None,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
            )?;
        }

        Commands::Withdraw {
            thread_id,
            as_actor,
            comment,
            fast_track,
            force,
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            commands::state::run(
                commands::state::StateShorthandArgs {
                    thread_id,
                    new_state: "withdrawn".into(),
                    approve: vec![],
                    as_actor,
                    resolve_open_actions: false,
                    link_to: vec![],
                    rel: None,
                    comment,
                    fast_track,
                    force,
                },
                &ctx,
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
                .filter(|s| status.as_deref().is_none_or(|st| s.status.as_str() == st))
                .collect();
            print!("{}", ls::render_ls(&filtered));
        }

        Commands::Shortlog { since, kind } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let kind_filter = kind.as_deref().map(parse_thread_kind).transpose()?;
            let since_dt = parse_since_date(&since, &git)?;
            let states = list_thread_states(&git, kind_filter, None)?;
            let mut entries: Vec<(&thread::ThreadState, chrono::DateTime<chrono::Utc>)> =
                Vec::new();
            for state in &states {
                if let Some(term_date) = terminal_state_date(state) {
                    if term_date >= since_dt {
                        entries.push((state, term_date));
                    }
                }
            }
            print!("{}", ls::render_shortlog(&entries));
        }

        Commands::Show {
            thread_id,
            what_next,
            compact,
            no_timeline,
            tree,
        } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            let state = thread::replay_thread(&git, &thread_id)?;
            if tree {
                let children = collect_implements_children(&git, &paths, &thread_id)?;
                print!("{}", show::render_tree(&state, &children));
            } else {
                let mode = if what_next {
                    show::ShowMode::WhatNext
                } else {
                    show::ShowMode::Full
                };
                print!(
                    "{}",
                    show::render_show(
                        &state,
                        &show::ShowOptions {
                            compact,
                            no_timeline,
                            policy: Some(policy),
                            mode,
                        }
                    )
                );
            }
        }

        Commands::Log {
            thread_id,
            reverse,
            last,
            since,
            event_type,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            if let Some(ref type_str) = event_type {
                const VALID_TYPES: &[&str] = &[
                    // EventType display names
                    "create",
                    "edit",
                    "retract",
                    "say",
                    "link",
                    "state",
                    "scope",
                    "resolve",
                    "reopen",
                    "verify",
                    "merge",
                    "revise-body",
                    "retype",
                    // NodeType display names (shown for Say events)
                    "claim",
                    "question",
                    "objection",
                    "evidence",
                    "summary",
                    "action",
                    "risk",
                    "review",
                    "alternative",
                    "assumption",
                ];
                if !VALID_TYPES.contains(&type_str.as_str()) {
                    eprintln!(
                        "error: unknown event type '{type_str}'\naccepted values: {}",
                        VALID_TYPES.join(", ")
                    );
                    std::process::exit(1);
                }
            }
            let state = thread::replay_thread(&git, &thread_id)?;
            let mut events: Vec<&event::Event> = state.events.iter().collect();
            if let Some(ref since_str) = since {
                let since_dt = parse_since_date(since_str, &git)?;
                events.retain(|e| e.created_at >= since_dt);
            }
            if let Some(ref type_str) = event_type {
                events.retain(|e| timeline::event_display_type(e) == *type_str);
            }
            let len = events.len();
            let skip = last.map(|n| len.saturating_sub(n)).unwrap_or(0);
            let selected: Vec<&event::Event> = if reverse {
                events.iter().rev().take(len - skip).copied().collect()
            } else {
                events.iter().skip(skip).copied().collect()
            };
            for line in timeline::render_markdown_refs(&selected) {
                println!("{line}");
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
            print!(
                "{}",
                show::render_show(
                    &state,
                    &show::ShowOptions {
                        mode: show::ShowMode::Status,
                        ..show::ShowOptions::default()
                    }
                )
            );
        }

        Commands::Node { cmd } => match cmd {
            NodeCmd::Show { node_id } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let lookup = thread::find_node(&git, &node_id)?;
                print!(
                    "{}",
                    show::render_node_show(&lookup, &show::ShowOptions::default())
                );
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
                branch::set_branch(&git, &thread_id, Some(&branch), &actor, &clock)?;
                println!("{thread_id} -> branch {branch}");
            }
            BranchCmd::Clear {
                thread_id,
                as_actor,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let thread_id = resolve_tid(&git, &thread_id)?;
                let actor = resolve_actor(as_actor, &git);
                branch::set_branch(&git, &thread_id, None, &actor, &clock)?;
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
        } => {
            let ctx = Context::discover(Box::new(SystemClock))?;
            revise_cmd::run(
                revise_cmd::ReviseArgs {
                    thread_id,
                    body,
                    body_file,
                    edit,
                    incorporates,
                    as_actor,
                    force,
                    cmd,
                },
                &ctx,
            )?;
        }
        Commands::Comment {
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
            NodeType::Comment,
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
        Commands::Retract {
            thread_id,
            node_ids,
            as_actor,
        } => run_node_lifecycle_bulk(
            &thread_id,
            &node_ids,
            as_actor,
            NodeLifecycleOp::Retract,
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
            NodeLifecycleOp::Resolve,
            "Resolved",
            &clock,
        )?,

        Commands::Reopen {
            thread_id,
            node_ids,
            as_actor,
        } => {
            if node_ids.is_empty() {
                run_state_shorthand(
                    &thread_id,
                    "open",
                    &[],
                    as_actor,
                    false,
                    &[],
                    None,
                    None,
                    false,
                    false,
                    &clock,
                )?;
            } else {
                run_node_lifecycle_bulk(
                    &thread_id,
                    &node_ids,
                    as_actor,
                    NodeLifecycleOp::Reopen,
                    "Reopened",
                    &clock,
                )?;
            }
        }

        Commands::Retype {
            thread_id,
            node_id,
            new_type,
            as_actor,
            force,
        } => {
            commands::retype::run_retype(&thread_id, &node_id, &new_type, as_actor, force, &clock)?
        }

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
            force,
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
                    let new_state = new_state.ok_or_else(|| {
                        ForumError::Config(
                            "usage: git forum state <THREAD_ID> <NEW_STATE> [--approve <ACTOR_ID>]... [--resolve-open-actions]"
                                .into(),
                        )
                    })?;
                    let ctx = Context::discover(Box::new(SystemClock))?;
                    commands::state::run(
                        commands::state::StateShorthandArgs {
                            thread_id,
                            new_state,
                            approve,
                            as_actor,
                            resolve_open_actions,
                            link_to,
                            rel,
                            comment,
                            fast_track,
                            force,
                        },
                        &ctx,
                    )?;
                }
            }
        }

        Commands::Brief { thread_id, json } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let thread_id = resolve_tid(&git, &thread_id)?;
            let state = thread::replay_thread(&git, &thread_id)?;
            let incoming = read_incoming_link_counts(&paths, &thread_id);
            if json {
                let payload = brief::build_json(&state, &incoming);
                let s = serde_json::to_string_pretty(&payload)
                    .map_err(|e| ForumError::Repo(e.to_string()))?;
                println!("{s}");
            } else {
                print!("{}", brief::render_plaintext(&state, &incoming));
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
                let state = thread::replay_thread(&git, &thread_id)?;
                println!("{thread_id}: not ready");
                for v in &report.violations {
                    println!("  BLOCKED [{}] {}", v.rule, v.reason);
                    let hint = state_change::remediation_hint(&v.rule, &state, &thread_id);
                    if !hint.is_empty() {
                        println!("    fix: {hint}");
                    }
                }
            }
            for entry in &report.lookahead {
                println!("  lookahead ({}):", entry.path);
                for v in &entry.violations {
                    println!("    [{}] {}", v.rule, v.reason);
                }
            }
            for adv in &report.linked_advisories {
                println!("  advisory: {}", adv.message);
            }
            if !report.passed() {
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
            } => commands::evidence::run_evidence_add(
                &thread_id,
                kind,
                &ref_targets,
                as_actor,
                force,
                &clock,
            )?,
        },

        Commands::Link {
            thread_id,
            target_thread_id,
            rel,
            as_actor,
        } => commands::link::run_link(&thread_id, &target_thread_id, &rel, as_actor, &clock)?,

        Commands::Hook { cmd } => {
            let git = GitOps::discover()?;
            match cmd {
                HookCmd::Install { force } => {
                    hook::install_all_hooks(&git, force)?;
                }
                HookCmd::Uninstall => {
                    hook::uninstall_all_hooks(&git)?;
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
                HookCmd::FixIndex => {
                    let result = hook::fix_index_blobs(&git)?;
                    for (path, sha) in &result.fixed {
                        eprintln!("fix-index: re-hashed {path} (missing blob {sha})");
                    }
                    for (path, sha) in &result.warnings {
                        eprintln!(
                            "fix-index: WARNING — {path} has missing blob {sha} and no working-tree copy"
                        );
                    }
                    if result.fixed.is_empty() && result.warnings.is_empty() {
                        eprintln!("fix-index: all index blobs present");
                    }
                }
                HookCmd::WorktreeInit => {
                    let git_dir = git.git_dir()?;
                    let paths = RepoPaths::from_repo_root_and_git_dir(git.root(), &git_dir);
                    if !paths.git_forum.join("logs").is_dir() {
                        // Per ADR-007: worktree-init writes only .git/forum/
                        // local state. Tracked .forum/ content arrives via
                        // checkout, never via this hook.
                        init::init_forum_local(&paths)?;
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
                            fs::write(&local_toml_path, content)?;
                        }
                        let _ = init::ensure_forum_refspecs(&git);
                        hook::install_all_hooks(&git, false)?;
                        eprintln!(
                            "git-forum: initialized worktree at {}",
                            git.root().display()
                        );
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
                        &policy, &state, parts[0], parts[1],
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

// TODO(phase-4): goes with the `Search` arm at slot 11; see
// `doc/internal/main-rs-audit.md`.
/// SPEC-2.0 §9.2 / Track D: rewrite legacy `kind:<name>` tokens in a search
/// query string to the equivalent `lifecycle:<L> AND tag:<T>` form for one
/// minor release, then prints a deprecation warning to stderr.
///
/// Mapping (drawn from the kind preset table):
/// - `kind:rfc`  → `lifecycle:proposal AND tag:cross-cutting`
/// - `kind:dec`  → `lifecycle:record`
/// - `kind:task` (alias `job`) → `lifecycle:execution AND tag:task`
/// - `kind:issue` (aliases `ask`, `bug`) → `lifecycle:execution AND tag:bug`
///
/// Tokens are split on whitespace; `kind:` matching is case-insensitive on
/// the prefix and value. Unknown values pass through unchanged. The query
/// returned is a plain substring expression (the index search is LIKE-based;
/// SPEC-2.0 §9.2 reserves the right to add a real grammar later).
fn translate_legacy_kind_query(query: &str) -> String {
    let mut out: Vec<String> = Vec::with_capacity(8);
    let mut translated_any = false;
    for token in query.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("kind:") {
            if let Some(preset) = preset_lookup(rest) {
                let mut parts = vec![format!("lifecycle:{}", preset.lifecycle.as_str())];
                for tag in preset.tags {
                    parts.push(format!("tag:{tag}"));
                }
                out.push(parts.join(" AND "));
                translated_any = true;
                continue;
            }
        }
        out.push(token.to_string());
    }
    if translated_any {
        eprintln!(
            "warning: `kind:<name>` is deprecated in search queries (SPEC-2.0 §9.2). \
             Use `lifecycle:<L>` and/or `tag:<T>` instead. \
             Auto-translation will be removed in 3.0."
        );
    }
    out.join(" ")
}

// TODO(phase-4): goes with the `Import` arm at slot 11.
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

/// Collect direct incoming `--rel implements` children of a thread for the
/// `show --tree` advisory.
///
/// Reads the SQLite reverse-link index for child IDs (single indexed lookup),
/// then replays each child's tip ref to get its lifecycle/status/title. One
/// hop only — does not recurse, does not include other relations. Per
/// SPEC-2.0 §2.1 and CORE-VALUE.md "Advisories", the result is informational
/// and never gates an operation.
///
/// If the index is missing or stale (e.g. before the first reindex on an
/// upgraded repo), falls back to scanning thread refs directly so the
/// advisory still works.
fn collect_implements_children(
    git: &GitOps,
    paths: &config::RepoPaths,
    parent_thread_id: &str,
) -> Result<Vec<show::TreeChild>, ForumError> {
    let db_path = paths.git_forum.join("index.db");
    let child_ids: Vec<String> = if db_path.is_file() {
        match index::open_db(&db_path) {
            Ok(conn) => index::find_incoming_links(&conn, parent_thread_id, Some("implements"))?
                .into_iter()
                .map(|l| l.from_thread_id)
                .collect(),
            Err(_) => fallback_scan_implements(git, parent_thread_id)?,
        }
    } else {
        fallback_scan_implements(git, parent_thread_id)?
    };

    let mut out = Vec::with_capacity(child_ids.len());
    for id in child_ids {
        match thread::replay_thread(git, &id) {
            Ok(child_state) => out.push(show::TreeChild {
                id: child_state.id.clone(),
                title: child_state.title.clone(),
                lifecycle_label: child_state.lifecycle.as_str().to_string(),
                status: child_state.status.to_string(),
            }),
            Err(_) => {
                // Skip unreplayable children rather than aborting the
                // advisory — staleness in the reverse-link index is recoverable
                // by `git forum reindex`.
                continue;
            }
        }
    }
    Ok(out)
}

/// Read incoming-link counts grouped by relation for `brief`.
///
/// Returns an empty `IncomingLinkCounts` (not an error) when the SQLite index
/// is missing — `brief` is read-only and per RFC-5wf2v8hv MUST NOT open other
/// threads' refs to compute these counts. A stale or absent index just means
/// the in= line shows `0`; `git forum reindex` rebuilds it.
fn read_incoming_link_counts(
    paths: &config::RepoPaths,
    thread_id: &str,
) -> brief::IncomingLinkCounts {
    let db_path = paths.git_forum.join("index.db");
    if !db_path.is_file() {
        return brief::IncomingLinkCounts::default();
    }
    let Ok(conn) = index::open_db(&db_path) else {
        return brief::IncomingLinkCounts::default();
    };
    let Ok(rows) = index::find_incoming_links(&conn, thread_id, None) else {
        return brief::IncomingLinkCounts::default();
    };
    let mut counts = brief::IncomingLinkCounts::default();
    for row in rows {
        *counts.by_rel.entry(row.rel).or_insert(0) += 1;
    }
    counts
}

/// Fallback for `show --tree` when the SQLite index is missing/unreadable:
/// list all thread refs and replay each to find the ones whose forward links
/// include `(parent_thread_id, implements)`. O(N) on thread count — only used
/// as a safety net so the advisory never silently fails.
fn fallback_scan_implements(
    git: &GitOps,
    parent_thread_id: &str,
) -> Result<Vec<String>, ForumError> {
    let ids = thread::list_thread_ids(git)?;
    let mut out = Vec::new();
    for id in ids {
        if id == parent_thread_id {
            continue;
        }
        let Ok(state) = thread::replay_thread(git, &id) else {
            continue;
        };
        if state
            .links
            .iter()
            .any(|l| l.target_thread_id == parent_thread_id && l.rel == "implements")
        {
            out.push(state.id);
        }
    }
    out.sort();
    Ok(out)
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
