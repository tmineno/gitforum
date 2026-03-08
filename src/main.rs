use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use git_forum::internal::actor;
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
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::reindex;
use git_forum::internal::run_ops;
use git_forum::internal::say;
use git_forum::internal::show;
use git_forum::internal::thread;
use git_forum::internal::verify;

#[derive(Parser)]
#[command(name = "git-forum", about = "Structured discussion in Git")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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
    /// Decision sub-commands
    Decision {
        #[command(subcommand)]
        cmd: ThreadCmd,
    },
    /// List all threads (optionally filter by kind)
    Ls,
    /// Show thread details
    Show { thread_id: String },
    /// Node sub-commands
    Node {
        #[command(subcommand)]
        cmd: NodeCmd,
    },
    /// Add a typed discussion node to a thread
    Say {
        thread_id: String,
        #[arg(long = "type", value_name = "NODE_TYPE")]
        node_type: NodeType,
        #[arg(long)]
        body: String,
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
        #[arg(long)]
        body: String,
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
        thread_id: String,
        new_state: String,
        /// Actor IDs to record as approvals (may be repeated)
        #[arg(long = "sign", value_name = "ACTOR")]
        sign: Vec<String>,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    #[command(
        about = "Verify whether the thread currently satisfies guard conditions for its next forward transition",
        long_about = "Verify whether the thread currently satisfies policy guard conditions for its next forward transition.\n\nCurrent behavior:\n- RFC in `under-review` is checked as if it were moving to `accepted`\n- Decision in `proposed` is checked as if it were moving to `accepted`\n- Other thread kinds or states currently return `ok` because no forward verify target is defined\n\nThis command is read-only. It does not change thread state or attach approvals."
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
    /// AI run sub-commands
    Run {
        #[command(subcommand)]
        cmd: RunCmd,
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
enum RunCmd {
    /// Spawn a new AI run for a thread
    Spawn {
        thread_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// List all runs
    Ls,
    /// Show a single run
    Show { run_label: String },
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
        /// Override actor ID (default: from git config)
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// List threads of this kind
    Ls,
}

fn main() -> Result<(), ForumError> {
    let cli = Cli::parse();
    let clock = SystemClock;
    let ids = UlidGenerator;

    match cli.command {
        Commands::Init => {
            let git = GitOps::discover()?;
            let paths = RepoPaths::from_repo_root(git.root());
            init::init_forum(&paths)?;
            println!("Initialized git-forum in {}", git.root().display());
        }

        Commands::Doctor => {
            let git = GitOps::discover()?;
            let paths = RepoPaths::from_repo_root(git.root());
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
            let git = GitOps::discover()?;
            let report = reindex::run_reindex(&git)?;
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

        Commands::Issue { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Issue, &clock, &ids)?;
        }
        Commands::Rfc { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Rfc, &clock, &ids)?;
        }
        Commands::Decision { cmd } => {
            run_thread_cmd(cmd, ThreadKind::Decision, &clock, &ids)?;
        }

        Commands::Ls => {
            let git = GitOps::discover()?;
            let ids_list = thread::list_thread_ids(&git)?;
            let mut states = Vec::new();
            for id in &ids_list {
                states.push(thread::replay_thread(&git, id)?);
            }
            let refs: Vec<&thread::ThreadState> = states.iter().collect();
            print!("{}", show::render_ls(&refs));
        }

        Commands::Show { thread_id } => {
            let git = GitOps::discover()?;
            let state = thread::replay_thread(&git, &thread_id)?;
            print!("{}", show::render_show(&state));
        }

        Commands::Node { cmd } => match cmd {
            NodeCmd::Show { node_id } => {
                let git = GitOps::discover()?;
                let lookup = thread::find_node(&git, &node_id)?;
                print!("{}", show::render_node_show(&lookup));
            }
        },

        Commands::Say {
            thread_id,
            node_type,
            body,
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let node_id = say::say_node(&git, &thread_id, node_type, &body, &actor, &clock, &ids)?;
            println!("Added {node_type} {node_id}");
        }

        Commands::Revise {
            thread_id,
            node_id,
            body,
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            say::revise_node(&git, &thread_id, &resolved, &body, &actor, &clock, &ids)?;
            println!("Revised {resolved}");
        }

        Commands::Retract {
            thread_id,
            node_id,
            as_actor,
        } => {
            let git = GitOps::discover()?;
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
            let git = GitOps::discover()?;
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
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let resolved = thread::resolve_node_id_in_thread(&git, &thread_id, &node_id)?;
            say::reopen_node(&git, &thread_id, &resolved, &actor, &clock, &ids)?;
            println!("Reopened {resolved}");
        }

        Commands::State {
            thread_id,
            new_state,
            sign,
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let paths = RepoPaths::from_repo_root(git.root());
            let policy = Policy::load(&paths.dot_forum.join("policy.toml"))?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            say::change_state(
                &git, &thread_id, &new_state, &sign, &actor, &clock, &ids, &policy,
            )?;
            println!("{thread_id} -> {new_state}");
        }

        Commands::Verify { thread_id } => {
            let git = GitOps::discover()?;
            let paths = RepoPaths::from_repo_root(git.root());
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
                let git = GitOps::discover()?;
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
            let git = GitOps::discover()?;
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

        Commands::Run { cmd } => match cmd {
            RunCmd::Spawn {
                thread_id,
                as_actor,
            } => {
                let git = GitOps::discover()?;
                let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
                let run_label = run_ops::spawn_run(&git, &thread_id, &actor, &clock)?;
                println!("Spawned {run_label}");
            }
            RunCmd::Ls => {
                let git = GitOps::discover()?;
                let runs = run_ops::list_runs(&git)?;
                print!("{}", show::render_run_ls(&runs));
            }
            RunCmd::Show { run_label } => {
                let git = GitOps::discover()?;
                let run = run_ops::read_run(&git, &run_label)?;
                print!("{}", show::render_run_show(&run));
            }
        },

        Commands::Policy { cmd } => {
            let git = GitOps::discover()?;
            let paths = RepoPaths::from_repo_root(git.root());
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
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let body = resolve_thread_body(body, body_file)?;
            let thread_id =
                create::create_thread(&git, kind, &title, body.as_deref(), &actor, clock, ids)?;
            println!("Created {thread_id}");
        }
        ThreadCmd::Ls => {
            let git = GitOps::discover()?;
            let all_ids = thread::list_thread_ids(&git)?;
            let mut states = Vec::new();
            for id in &all_ids {
                let s = thread::replay_thread(&git, id)?;
                if s.kind == kind {
                    states.push(s);
                }
            }
            let refs: Vec<&thread::ThreadState> = states.iter().collect();
            print!("{}", show::render_ls(&refs));
        }
    }
    Ok(())
}

fn resolve_thread_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
) -> Result<Option<String>, ForumError> {
    match (body, body_file) {
        (Some(body), None) => Ok(Some(body)),
        (None, Some(path)) => Ok(Some(fs::read_to_string(path)?)),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => unreachable!("clap enforces body/body-file conflicts"),
    }
}
