pub mod fuse_fd;
pub mod launch;
pub mod list;
pub mod migrate;
pub mod mount;
pub mod unmount;
pub mod version;
pub mod workspace;

use crate::error::FsError;
use clap::{Parser, Subcommand};

pub use fuse_fd::is_fuse_fd_invocation;

#[derive(Parser)]
#[command(
    name = "openeral",
    about = "Mount PostgreSQL databases as virtual filesystems"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Mount a PostgreSQL database
    Mount(mount::MountArgs),
    /// Run pending database migrations
    Migrate(migrate::MigrateArgs),
    /// Unmount a previously mounted database
    Unmount(unmount::UnmountArgs),
    /// List active mounts
    List,
    /// Show version information
    Version,
    /// Manage workspaces (create, mount, seed, list, delete)
    Workspace(workspace::WorkspaceArgs),
    /// Launch OpenEral: gateway + providers + sandbox in one command
    Launch(launch::LaunchArgs),
}

pub async fn run() -> Result<(), FsError> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Mount(args) => mount::execute(args).await,
        Commands::Migrate(args) => migrate::execute(args).await,
        Commands::Unmount(args) => unmount::execute(args).await,
        Commands::List => list::execute().await,
        Commands::Version => {
            version::execute();
            Ok(())
        }
        Commands::Workspace(args) => workspace::execute(args).await,
        Commands::Launch(args) => launch::execute(args).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_parse_migrate_subcommand() {
        let cli = Cli::try_parse_from(["openeral", "migrate"]).unwrap();
        assert!(matches!(cli.command, Commands::Migrate(_)));
    }
}
