use crate::error::FsError;
use clap::Args;
use std::path::PathBuf;
use std::process::Command;

#[derive(Args)]
pub struct UnmountArgs {
    /// Mount point to unmount
    pub mount_point: PathBuf,
}

pub async fn execute(args: UnmountArgs) -> Result<(), FsError> {
    let output = Command::new("fusermount")
        .arg("-u")
        .arg(&args.mount_point)
        .output()
        .map_err(FsError::IoError)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FsError::InternalError(format!(
            "Unmount failed: {}",
            stderr
        )));
    }

    tracing::info!(mount_point = %args.mount_point.display(), "Unmounted successfully");
    Ok(())
}
