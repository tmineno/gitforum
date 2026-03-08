use clap::{Parser, Subcommand};
use git_forum::internal::config::RepoPaths;
use git_forum::internal::doctor;
use git_forum::internal::error::ForumError;
use git_forum::internal::git_ops::GitOps;
use git_forum::internal::init;
use git_forum::internal::reindex;

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
}

fn main() -> Result<(), ForumError> {
    let cli = Cli::parse();

    let git = GitOps::discover()?;
    let paths = RepoPaths::from_repo_root(git.root());

    match cli.command {
        Commands::Init => {
            init::init_forum(&paths)?;
            println!("Initialized git-forum in {}", git.root().display());
        }
        Commands::Doctor => {
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
            let report = reindex::run_reindex(&git)?;
            println!(
                "Reindex complete: {} threads found, {} replayed, {} errors",
                report.threads_found,
                report.threads_replayed.len(),
                report.errors.len()
            );
            for (id, err) in &report.errors {
                println!("  error: {id}: {err}");
            }
        }
    }

    Ok(())
}
