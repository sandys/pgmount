use crate::error::FsError;

pub async fn execute() -> Result<(), FsError> {
    // Read /proc/mounts or /etc/mtab to find pgmount entries
    let mounts = std::fs::read_to_string("/proc/mounts")
        .map_err(FsError::IoError)?;

    let mut found = false;
    for line in mounts.lines() {
        if line.contains("pgmount") || line.contains("fuse.pgmount") {
            println!("{}", line);
            found = true;
        }
    }

    if !found {
        println!("No active pgmount mounts found.");
    }

    Ok(())
}
