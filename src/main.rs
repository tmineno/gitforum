use clap::{Parser, Subcommand};
use git_forum::internal::actor;
use git_forum::internal::clock::SystemClock;
use git_forum::internal::config::RepoPaths;
use git_forum::internal::create;
use git_forum::internal::doctor;
use git_forum::internal::error::ForumError;
use git_forum::internal::event::ThreadKind;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::id::UlidGenerator;
use git_forum::internal::init;
use git_forum::internal::reindex;
use git_forum::internal::show;
use git_forum::internal::thread;

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
