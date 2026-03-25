use crate::error::FsError;

pub async fn execute() -> Result<(), FsError> {
    // Read /proc/mounts or /etc/mtab to find openeral entries
    let mounts = std::fs::read_to_string("/proc/mounts").map_err(FsError::IoError)?;

    let mut found = false;
    for line in mounts.lines() {
        if line.contains("openeral") || line.contains("fuse.openeral") {
            println!("{}", line);
            found = true;
        }
    }

    if !found {
        println!("No active openeral mounts found.");
    }

    Ok(())
}
