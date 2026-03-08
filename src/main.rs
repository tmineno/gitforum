use clap::{Parser, Subcommand};
use git_forum::internal::actor;
use git_forum::internal::clock::SystemClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::doctor;
use git_forum::internal::error::ForumError;
use git_forum::internal::event::{NodeType, ThreadKind};
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::UlidGenerator;
use git_forum::internal::init;
use git_forum::internal::policy::Policy;
use git_forum::internal::reindex;
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
        node_id: String,
        #[arg(long)]
        body: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Retract a node (soft-delete)
    Retract {
        thread_id: String,
        node_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Resolve a node (mark as addressed)
    Resolve {
        thread_id: String,
        node_id: String,
        #[arg(long = "as", value_name = "ACTOR")]
        as_actor: Option<String>,
    },
    /// Reopen a resolved or retracted node
    Reopen {
        thread_id: String,
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
    /// Verify a thread against policy guards
    Verify { thread_id: String },
    /// Policy sub-commands
    Policy {
        #[command(subcommand)]
        cmd: PolicyCmd,
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
enum ThreadCmd {
    /// Create a new thread
    New {
        title: String,
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
            say::revise_node(&git, &thread_id, &node_id, &body, &actor, &clock, &ids)?;
            println!("Revised {node_id}");
        }

        Commands::Retract {
            thread_id,
            node_id,
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            say::retract_node(&git, &thread_id, &node_id, &actor, &clock, &ids)?;
            println!("Retracted {node_id}");
        }

        Commands::Resolve {
            thread_id,
            node_id,
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            say::resolve_node(&git, &thread_id, &node_id, &actor, &clock, &ids)?;
            println!("Resolved {node_id}");
        }

        Commands::Reopen {
            thread_id,
            node_id,
            as_actor,
        } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            say::reopen_node(&git, &thread_id, &node_id, &actor, &clock, &ids)?;
            println!("Reopened {node_id}");
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
        ThreadCmd::New { title, as_actor } => {
            let git = GitOps::discover()?;
            let actor = as_actor.unwrap_or_else(|| actor::current_actor(&git));
            let thread_id = create::create_thread(&git, kind, &title, &actor, clock, ids)?;
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
