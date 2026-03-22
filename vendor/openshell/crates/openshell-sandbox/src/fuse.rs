// SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! FUSE filesystem support via /etc/fstab inspection.
//!
//! The supervisor reads /etc/fstab from the container image at startup.
//! Any `fuse.*` entries with the `noauto` flag are set up as
//! supervisor-managed FUSE mounts before the child process is spawned.
//!
//! This enables container images to declare FUSE mounts using standard
//! Linux conventions, without requiring elevated privileges in user code.

use miette::{IntoDiagnostic, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::policy::SandboxPolicy;

/// A FUSE mount discovered from /etc/fstab.
#[derive(Debug)]
pub struct FuseMount {
    /// Source argument passed to the FUSE binary (e.g., connection string).
    pub source: String,
    /// Mount point path (e.g., /db).
    pub mount_point: PathBuf,
    /// Full filesystem type (e.g., "fuse.openeral").
    pub fs_type: String,
    /// Binary name extracted from fs_type (e.g., "openeral").
    pub binary: String,
    /// Comma-separated mount options.
    pub options: String,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

/// Parse /etc/fstab for FUSE mount entries.
///
/// Returns entries where:
/// - Type starts with `fuse.`
/// - Options contain `noauto` (explicit opt-in, not auto-mounted at boot)
/// - The FUSE binary exists in PATH
pub fn discover_fuse_mounts() -> Vec<FuseMount> {
    let fstab_path = Path::new("/etc/fstab");
    let content = match std::fs::read_to_string(fstab_path) {
        Ok(c) => c,
        Err(e) => {
            debug!(error = %e, "No /etc/fstab or cannot read it, skipping FUSE discovery");
            return Vec::new();
        }
    };

    let mut mounts = Vec::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 {
            continue;
        }

        let source = fields[0];
        let mount_point = fields[1];
        let fs_type = fields[2];
        let options = fields[3];

        // Only process fuse.* types
        let binary = match fs_type.strip_prefix("fuse.") {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };

        // Only process noauto entries (explicit opt-in)
        if !options.split(',').any(|o| o == "noauto") {
            debug!(
                mount_point = mount_point,
                "Skipping fuse entry without noauto flag"
            );
            continue;
        }

        // Verify the binary exists
        if which_binary(binary).is_none() {
            warn!(
                binary = binary,
                mount_point = mount_point,
                "FUSE binary not found in PATH, skipping"
            );
            continue;
        }

        // Strip quotes from source (fstab allows quoted source fields)
        let source = source.trim_matches('"').trim_matches('\'');

        let read_only = options.split(',').any(|o| o == "ro");

        info!(
            fs_type = fs_type,
            source = source,
            mount_point = mount_point,
            read_only = read_only,
            "Discovered FUSE mount in /etc/fstab"
        );

        mounts.push(FuseMount {
            source: source.to_string(),
            mount_point: PathBuf::from(mount_point),
            fs_type: fs_type.to_string(),
            binary: binary.to_string(),
            options: options.to_string(),
            read_only,
        });
    }

    mounts
}

/// Create /dev/fuse character device if it doesn't exist.
///
/// Requires CAP_SYS_ADMIN (the supervisor has it).
/// Major 10, minor 229 is the standard Linux FUSE device.
#[cfg(target_os = "linux")]
pub fn ensure_fuse_device() -> Result<()> {
    use nix::sys::stat::{self, SFlag};

    let path = Path::new("/dev/fuse");
    if path.exists() {
        debug!("/dev/fuse already exists");
        return Ok(());
    }

    info!("Creating /dev/fuse character device (10, 229)");
    stat::mknod(
        path,
        SFlag::S_IFCHR,
        stat::Mode::from_bits_truncate(0o666),
        stat::makedev(10, 229),
    )
    .into_diagnostic()?;

    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_fuse_device() -> Result<()> {
    Ok(())
}

/// Spawn a FUSE daemon via mount.fuse3 for a single fstab entry.
///
/// mount.fuse3 handles the privileged setup (opening /dev/fuse, calling
/// mount(2)), then execs the FUSE binary. The binary runs as a long-lived
/// daemon process managed by the supervisor.
pub fn spawn_fuse_mount(mount: &FuseMount, env: &HashMap<String, String>) -> Result<Child> {
    let mount_fuse3 = which_binary("mount.fuse3")
        .ok_or_else(|| miette::miette!("mount.fuse3 not found in PATH"))?;

    info!(
        binary = %mount.binary,
        mount_point = %mount.mount_point.display(),
        "Spawning FUSE daemon via mount.fuse3"
    );

    // mount.fuse3 expects: mount.fuse3 type#source mountpoint -o options
    let type_source = format!("{}#{}", mount.binary, mount.source);

    let mut cmd = Command::new(mount_fuse3);
    cmd.arg(&type_source);
    cmd.arg(&mount.mount_point);
    cmd.arg("-o");
    cmd.arg(&mount.options);

    // Pass provider environment (database URLs, API keys, etc.)
    for (k, v) in env {
        cmd.env(k, v);
    }

    cmd.spawn().into_diagnostic()
}

/// Wait for a mountpoint to become ready.
pub fn wait_for_mount(path: &Path, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(500);

    while start.elapsed() < timeout {
        if is_mountpoint(path) {
            info!(path = %path.display(), "FUSE mount ready");
            return Ok(());
        }
        std::thread::sleep(poll_interval);
    }

    Err(miette::miette!(
        "FUSE mount at {} did not become ready within {:?}",
        path.display(),
        timeout
    ))
}

/// Check if a path is a mountpoint using /proc/mounts.
fn is_mountpoint(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    std::fs::read_to_string("/proc/mounts")
        .ok()
        .map(|content| {
            content
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(path_str.as_ref()))
        })
        .unwrap_or(false)
}

/// Find a binary in PATH, returning its full path.
fn which_binary(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(name);
            if full.is_file() {
                Some(full)
            } else {
                None
            }
        })
    })
}

/// Top-level entry point: discover FUSE mounts from fstab, set up the device,
/// spawn all daemons, wait for readiness, and update the policy for Landlock.
///
/// Returns the FUSE daemon child processes. These must be kept alive for the
/// duration of the sandbox. On cleanup, they should be killed and their mounts
/// unmounted via fusermount.
pub fn setup_fuse_mounts(
    policy: &mut SandboxPolicy,
    env: &HashMap<String, String>,
) -> Result<Vec<Child>> {
    let mounts = discover_fuse_mounts();

    if mounts.is_empty() {
        debug!("No FUSE mounts declared in /etc/fstab");
        return Ok(Vec::new());
    }

    info!(count = mounts.len(), "Setting up FUSE mounts from /etc/fstab");

    // Create /dev/fuse if missing (requires CAP_SYS_ADMIN)
    ensure_fuse_device()?;

    let timeout = Duration::from_secs(30);
    let mut daemons = Vec::new();

    for mount in &mounts {
        // Ensure mount point directory exists
        if !mount.mount_point.exists() {
            std::fs::create_dir_all(&mount.mount_point).into_diagnostic()?;
        }

        // Spawn the FUSE daemon
        let child = spawn_fuse_mount(mount, env)?;
        daemons.push(child);

        // Wait for mount to become ready
        wait_for_mount(&mount.mount_point, timeout)?;

        // Add mount point to Landlock-allowed paths so the child process can access it
        if mount.read_only {
            policy
                .filesystem
                .read_only
                .push(mount.mount_point.clone());
        } else {
            policy
                .filesystem
                .read_write
                .push(mount.mount_point.clone());
        }
    }

    // /dev/fuse needs to stay accessible for FUSE daemon processes
    policy
        .filesystem
        .read_write
        .push(PathBuf::from("/dev/fuse"));

    info!(
        count = daemons.len(),
        "All FUSE mounts ready"
    );

    Ok(daemons)
}
