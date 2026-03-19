pub mod mount;
pub mod unmount;
pub mod list;
pub mod version;

use clap::{Parser, Subcommand};
use crate::error::FsError;

#[derive(Parser)]
#[command(name = "pgmount", about = "Mount PostgreSQL databases as virtual filesystems")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Mount a PostgreSQL database
    Mount(mount::MountArgs),
    /// Unmount a previously mounted database
    Unmount(unmount::UnmountArgs),
    /// List active mounts
    List,
    /// Show version information
    Version,
}

pub async fn run() -> Result<(), FsError> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Mount(args) => mount::execute(args).await,
        Commands::Unmount(args) => unmount::execute(args).await,
        Commands::List => list::execute().await,
        Commands::Version => {
            version::execute();
            Ok(())
        }
    }
}
