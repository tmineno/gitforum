use std::fs;
use std::io::Read;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, Subcommand};
use git_forum::internal::actor;
use git_forum::internal::branch_ops;
use git_forum::internal::clock::SystemClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::doctor;
use git_forum::internal::error::ForumError;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::evidence::EvidenceKind;
use git_forum::internal::evidence_ops;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::UlidGenerator;
use git_forum::internal::index;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::reindex;
use git_forum::internal::say;
use git_forum::internal::show;
use git_forum::internal::thread;
use git_forum::internal::tui as forum_tui;
use git_forum::internal::verify;

#[derive(Parser)]
#[command(name = "git-forum", about = "Structured discussion in Git")]
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
    /// Check repository health
    Doctor,
    /// Rebuild local index from Git refs
    Reindex,
    /// Issue sub-commands
    Issue {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// RFC sub-commands
    Rfc {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// List all threads (optionally filter by kind)
    Ls {
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
    },
    /// Show thread details
    Show { thread_id: String },
    /// Show unresolved items for a thread or all threads
    Status {
        /// Thread ID (omit for --all)
        thread_id: Option<String>,
        /// Show status across all open threads
        #[arg(long)]
        all: bool,
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
    /// Revise the body of a thread
    ReviseBody {
        thread_id: String,
        /// New thread body text (use "-" to read from stdin)
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read new thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Node IDs to mark as incorporated into this body revision
        #[arg(long = "incorporates", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add a typed discussion node to a thread
    Say {
        thread_id: String,
        #[arg(long = "type", value_name = "NODE_TYPE")]
        node_type: NodeType,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read node body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        /// Reply to a specific node
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add a claim node to a thread
    Claim {
        thread_id: String,
        /// Node body (positional or use --body/--body-file)
        body: Option<String>,
        /// Read node body from a file
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add a question node to a thread
    Question {
        thread_id: String,
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add an objection node to a thread
    Objection {
        thread_id: String,
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add a summary node to a thread
    Summary {
        thread_id: String,
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add an action node to a thread
    Action {
        thread_id: String,
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add a risk node to a thread
    Risk {
        thread_id: String,
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Add a review node to a thread
    Review {
        thread_id: String,
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        #[arg(long = "reply-to", value_name = "NODE_ID")]
        reply_to: Option<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Revise the body of an existing node
    Revise {
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
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Retract a node (soft-delete)
    Retract {
        thread_id: String,
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Resolve a node (mark as addressed)
    Resolve {
        thread_id: String,
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Reopen a resolved or retracted node
    Reopen {
        thread_id: String,
        #[arg(
            value_name = "NODE_ID",
            help = "Full node ID or unique prefix within the thread (8+ chars unless exact match)"
        )]
        node_id: String,
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
        #[arg(long = "sign", value_name = "ACTOR")]
        sign: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
    },
    #[command(
        about = "Verify whether the thread currently satisfies guard conditions for its next forward transition",
        long_about = "Verify whether the thread currently satisfies policy guard conditions for its next forward transition.\n\nCurrent behavior:\n- Issue in `open` is checked as if it were moving to `closed`\n- RFC in `under-review` is checked as if it were moving to `accepted`\n- Other thread kinds or states currently return `ok` because no forward verify target is defined\n\nThis command is read-only. It does not change thread state or attach approvals."
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
    /// Open the interactive TUI
    Tui {
        /// Open a specific thread in detail view directly
        thread_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum PolicyCmd {
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
    /// Add an evidence item to a thread
    Add {
        thread_id: String,
        #[arg(long, value_name = "KIND")]
        kind: EvidenceKind,
        #[arg(long = "ref", value_name = "REF")]
        ref_target: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
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
        #[arg(long = "sign", value_name = "ACTOR")]
        sign: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
        #[arg(long)]
        resolve_open_actions: bool,
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum ThreadCmd {
    /// Create a new thread
    New {
        title: String,
        /// Initial thread body
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        /// Read initial thread body from a file
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
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
    },
    /// List threads of this kind
    Ls {
        #[arg(long, value_name = "BRANCH")]
        branch: Option<String>,
    },
    /// Revise the body of a thread
    ReviseBody {
        thread_id: String,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long = "body-file", value_name = "PATH", conflicts_with = "body")]
        body_file: Option<PathBuf>,
        #[arg(long = "incorporates", value_name = "NODE_ID")]
        incorporates: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
}

fn main() -> Result<(), ForumError> {
    let cli = Cli::parse();
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
    let ids = UlidGenerator;

    match command {
        Commands::Init => {
            let git = GitOps::discover()?;
            let paths = RepoPaths::from_repo_root(git.root());
            init::init_forum(&paths)?;
            println!("Initialized git-forum in {}", git.root().display());
        }

        Commands::Doctor => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let report = doctor::run_doctor(&git, &paths)?;
            for check in &report.checks {
                let marker = if check.passed { " ok " } else { "FAIL" };
                print!("[{marker}] {}", check.name);
                if let Some(detail) = &check.detail {
                    print!(" -- {detail}");
                }
                println!();
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

        Commands::Search { query } => {
            let (_git, paths) = discover_repo_with_init_warning()?;
            let db_path = paths.git_forum.join("index.db");
            let conn = index::open_db(&db_path)?;
            let results = index::search_threads(&conn, &query)?;
            print!("{}", show::render_search_results(&results));
        }

        Commands::Tui { thread_id } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let db_path = paths.git_forum.join("index.db");
            forum_tui::run(&git, &db_path, thread_id.as_deref())?;
        }

        Commands::Issue { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Issue, &clock, &ids)?;
        }
        Commands::Rfc { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Rfc, &clock, &ids)?;
        }

        Commands::Ls { branch } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let states = list_thread_states(&git, None, branch.as_deref())?;
            let refs: Vec<&thread::ThreadState> = states.iter().collect();
            print!("{}", show::render_ls(&refs));
        }

        Commands::Show { thread_id } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let state = thread::replay_thread(&git, &thread_id)?;
            print!("{}", show::render_show(&state));
        }

        Commands::Status { thread_id, all } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            if all {
                let states = list_thread_states(&git, None, None)?;
                let refs: Vec<&thread::ThreadState> = states.iter().collect();
                print!("{}", show::render_status_all(&refs));
            } else if let Some(thread_id) = thread_id {
                let state = thread::replay_thread(&git, &thread_id)?;
                print!("{}", show::render_status(&state));
            } else {
                return Err(ForumError::Config(
                    "usage: git forum status <THREAD_ID> or git forum status --all".into(),
                ));
            }
        }

        Commands::Node { cmd } => match cmd {
            NodeCmd::Show { node_id } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let lookup = thread::find_node(&git, &node_id)?;
                print!("{}", show::render_node_show(&lookup));
            }
        },

        Commands::Branch { cmd } => match cmd {
            BranchCmd::Bind {
                thread_id,
                branch,
                as_actor,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
                branch_ops::set_branch(&git, &thread_id, Some(&branch), &actor, &clock)?;
                println!("{thread_id} -> branch {branch}");
            }
            BranchCmd::Clear {
                thread_id,
                as_actor,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
                branch_ops::set_branch(&git, &thread_id, None, &actor, &clock)?;
                println!("{thread_id} -> branch <cleared>");
            }
        },

        Commands::ReviseBody {
            thread_id,
            body,
            body_file,
            incorporates,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let body_text = resolve_body_required(body, body_file)?;
            say::revise_body(
                &git,
                &thread_id,
                &body_text,
                &incorporates,
                &actor,
                &clock,
                &ids,
            )?;
            println!("Body revised for {thread_id}");
        }

        Commands::Say {
            thread_id,
            node_type,
            body,
            body_file,
            reply_to,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let body_text = resolve_body_required(body, body_file)?;
            let resolved_reply = resolve_reply_to(&git, &thread_id, reply_to.as_deref())?;
            let node_id = say::say_node(
                &git,
                &thread_id,
                node_type,
                &body_text,
                &actor,
                &clock,
                &ids,
                resolved_reply.as_deref(),
            )?;
            println!("Added {node_type} {node_id}");
        }
        Commands::Claim {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Claim,
            &clock,
            &ids,
        )?,
        Commands::Question {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Question,
            &clock,
            &ids,
        )?,
        Commands::Objection {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Objection,
            &clock,
            &ids,
        )?,
        Commands::Summary {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Summary,
            &clock,
            &ids,
        )?,
        Commands::Action {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Action,
            &clock,
            &ids,
        )?,
        Commands::Risk {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Risk,
            &clock,
            &ids,
        )?,
        Commands::Review {
            thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
        } => run_shorthand_say(
            &thread_id,
            body,
            body_file,
            reply_to,
            as_actor,
            NodeType::Review,
            &clock,
            &ids,
        )?,

        Commands::Revise {
            thread_id,
            node_id,
            body,
            body_file,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let body_text = resolve_body_required(body, body_file)?;
            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            say::revise_node(
                &git, &thread_id, &resolved, &body_text, &actor, &clock, &ids,
            )?;
            println!("Revised {resolved}");
        }

        Commands::Retract {
            thread_id,
            node_id,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            say::retract_node(&git, &thread_id, &resolved, &actor, &clock, &ids)?;
            println!("Retracted {resolved}");
        }

        Commands::Resolve {
            thread_id,
            node_id,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            say::resolve_node(&git, &thread_id, &resolved, &actor, &clock, &ids)?;
            println!("Resolved {resolved}");
        }

        Commands::Reopen {
            thread_id,
            node_id,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            say::reopen_node(&git, &thread_id, &resolved, &actor, &clock, &ids)?;
            println!("Reopened {resolved}");
        }

        Commands::State {
            cmd,
            thread_id,
            new_state,
            sign,
            as_actor,
            resolve_open_actions,
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
                    sign,
                    as_actor,
                    resolve_open_actions,
                    dry_run,
                }) => {
                    let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
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
                        &sign,
                        &actor,
                        &clock,
                        &ids,
                        say::StateChangeOptions {
                            resolve_open_actions,
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
                            "usage: git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]... [--resolve-open-actions]"
                                .into(),
                        )
                    })?;
                    let new_state = new_state.ok_or_else(|| {
                        ForumError::Config(
                            "usage: git forum state <THREAD_ID> <NEW_STATE> [--sign <ACTOR_ID>]... [--resolve-open-actions]"
                                .into(),
                        )
                    })?;
                    let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
                    say::change_state(
                        &git,
                        &thread_id,
                        &new_state,
                        &sign,
                        &actor,
                        &clock,
                        &ids,
                        &policy,
                        say::StateChangeOptions {
                            resolve_open_actions,
                        },
                    )?;
                    println!("{thread_id} -> {new_state}");
                }
            }
        }

        Commands::Verify { thread_id } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            let report = verify::verify_thread(&git, &thread_id, &policy)?;
            if report.passed() {
                println!("{thread_id}: ok");
            } else {
                for v in &report.violations {
                    println!("FAIL [{}] {}", v.rule, v.reason);
                }
                std::process::exit(1);
            }
        }

        Commands::Evidence { cmd } => match cmd {
            EvidenceCmd::Add {
                thread_id,
                kind,
                ref_target,
                as_actor,
            } => {
                let (git, _paths) = discover_repo_with_init_warning()?;
                let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
                let commit_sha = evidence_ops::add_evidence(
                    &git,
                    &thread_id,
                    kind,
                    &ref_target,
                    None,
                    &actor,
                    &clock,
                )?;
                println!(
                    "Evidence added ({})",
                    &commit_sha[..commit_sha.len().min(8)]
                );
            }
        },

        Commands::Link {
            thread_id,
            target_thread_id,
            rel,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
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

        Commands::Policy { cmd } => {
            let (git, paths) = discover_repo_with_init_warning()?;
            match cmd {
                PolicyCmd::Lint => {
                    let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
                    let diags = git_forum::internal::policy::lint_policy(&policy);
                    if diags.is_empty() {
                        println!("policy ok");
                    } else {
                        for d in &diags {
                            println!("WARN {d}");
                        }
                    }
                }
                PolicyCmd::Check {
                    thread_id,
                    transition,
                } => {
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

#[allow(clippy::too_many_arguments)]
fn run_shorthand_say(
    thread_id: &str,
    body: Option<String>,
    body_file: Option<PathBuf>,
    reply_to: Option<String>,
    as_actor: Option<String>,
    node_type: NodeType,
    clock: &dyn git_forum::internal::clock::Clock,
    ids: &dyn git_forum::internal::id::IdGenerator,
) -> Result<(), ForumError> {
    let (git, _paths) = discover_repo_with_init_warning()?;
    let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
    let body_text = resolve_body_required(body, body_file)?;
    let resolved_reply = resolve_reply_to(&git, thread_id, reply_to.as_deref())?;
    let node_id = say::say_node(
        &git,
        thread_id,
        node_type,
        &body_text,
        &actor,
        clock,
        ids,
        resolved_reply.as_deref(),
    )?;
    println!("Added {node_type} {node_id}");
    Ok(())
}

fn resolve_body_required(
    body: Option<String>,
    body_file: Option<PathBuf>,
) -> Result<String, ForumError> {
    resolve_thread_body(body, body_file)?
        .ok_or_else(|| ForumError::Config("--body or --body-file is required".into()))
}

fn run_thread_cmd(
    cmd: ThreadCmd,
    kind: ThreadKind,
    clock: &dyn git_forum::internal::clock::Clock,
    ids: &dyn git_forum::internal::id::IdGenerator,
) -> Result<(), ForumError> {
    match cmd {
        ThreadCmd::New {
            title,
            body,
            body_file,
            branch,
            link_to,
            rel,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let body = resolve_thread_body(body, body_file)?;
            let thread_id = create::create_thread_with_branch(
                &git,
                kind,
                &title,
                body.as_deref(),
                branch.as_deref(),
                &actor,
                clock,
                ids,
            )?;
            if !link_to.is_empty() {
                let rel = rel.as_deref().ok_or_else(|| {
                    ForumError::Config("--rel is required when --link-to is used".into())
                })?;
                for target in &link_to {
                    evidence_ops::add_thread_link(&git, &thread_id, target, rel, &actor, clock)?;
                }
            }
            println!("Created {thread_id}");
        }
        ThreadCmd::Ls { branch } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let states = list_thread_states(&git, Some(kind), branch.as_deref())?;
            let refs: Vec<&thread::ThreadState> = states.iter().collect();
            print!("{}", show::render_ls(&refs));
        }
        ThreadCmd::ReviseBody {
            thread_id,
            body,
            body_file,
            incorporates,
            as_actor,
        } => {
            let (git, _paths) = discover_repo_with_init_warning()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let body_text = resolve_body_required(body, body_file)?;
            say::revise_body(
                &git,
                &thread_id,
                &body_text,
                &incorporates,
                &actor,
                clock,
                ids,
            )?;
            println!("Body revised for {thread_id}");
        }
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

fn parse_thread_kind_filter(kind: Option<&str>) -> Result<Option<ThreadKind>, ForumError> {
    match kind {
        None => Ok(None),
        Some("issue") => Ok(Some(ThreadKind::Issue)),
        Some("rfc") => Ok(Some(ThreadKind::Rfc)),
        Some(other) => Err(ForumError::Config(format!(
            "unknown kind '{other}'; valid: issue, rfc"
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
    sign: &[String],
    actor: &str,
    clock: &dyn git_forum::internal::clock::Clock,
    ids: &dyn git_forum::internal::id::IdGenerator,
    options: say::StateChangeOptions,
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

        match say::prepare_state_change(git, &thread_id, new_state, sign, clock, policy, options) {
            Ok(plan) => {
                if !dry_run {
                    if let Err(err) = say::change_state(
                        git, &thread_id, new_state, sign, actor, clock, ids, policy, options,
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

fn discover_repo_with_init_warning() -> Result<(GitOps, RepoPaths), ForumError> {
    let git = GitOps::discover()?;
    let paths = RepoPaths::from_repo_root(git.root());
    if !is_forum_initialized(&paths) {
        eprintln!(
            "warning: git-forum is not initialized in this repository; run `git forum init` first"
        );
    }
    Ok((git, paths))
}

fn is_forum_initialized(paths: &RepoPaths) -> bool {
    paths.dot_forum.join("policy.toml").is_file() && paths.git_forum.join("logs").is_dir()
}

fn resolve_thread_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
) -> Result<Option<String>, ForumError> {
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
