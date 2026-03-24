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
    pub source: String,
    pub mount_point: PathBuf,
    pub binary: String,
    pub options: String,
    pub read_only: bool,
}

/// Parse /etc/fstab for FUSE mount entries.
pub fn discover_fuse_mounts() -> Vec<FuseMount> {
    let content = match std::fs::read_to_string("/etc/fstab") {
        Ok(c) => c,
        Err(e) => {
            debug!(error = %e, "No /etc/fstab, skipping FUSE discovery");
            return Vec::new();
        }
    };

    let mut mounts = Vec::new();

    for line in content.lines() {
        let line = line.trim();
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

        let binary = match fs_type.strip_prefix("fuse.") {
            Some(b) if !b.is_empty() => b,
            _ => continue,
        };

        if !options.split(',').any(|o| o == "noauto") {
            continue;
        }

        if which_binary(binary).is_none() && find_binary_in_common_paths(binary).is_none() {
            warn!(binary = binary, mount_point = mount_point, "FUSE binary not found, skipping");
            continue;
        }

        let source = source.trim_matches('"').trim_matches('\'');
        let read_only = options.split(',').any(|o| o == "ro");

        info!(fs_type = fs_type, source = source, mount_point = mount_point, "Discovered FUSE mount");

        mounts.push(FuseMount {
            source: source.to_string(),
            mount_point: PathBuf::from(mount_point),
            binary: binary.to_string(),
            options: options.to_string(),
            read_only,
        });
    }

    mounts
}

/// Verify /dev/fuse is available.
///
/// The fuse-device-plugin DaemonSet provides /dev/fuse to containers that
/// request the `github.com/fuse` resource. The kubelet handles the device
/// cgroup allowlist and bind-mount automatically.
pub fn ensure_fuse_device() -> Result<()> {
    let path = Path::new("/dev/fuse");
    if path.exists() {
        info!("/dev/fuse available (provided by fuse-device-plugin)");
        Ok(())
    } else {
        Err(miette::miette!(
            "/dev/fuse not found. The fuse-device-plugin DaemonSet may not be deployed, \
             or the pod did not request the github.com/fuse resource."
        ))
    }
}

/// Find mount.fuse3 in standard locations.
fn find_mount_fuse3() -> Option<PathBuf> {
    for path in &["/sbin/mount.fuse3", "/usr/sbin/mount.fuse3", "/bin/mount.fuse3", "/usr/bin/mount.fuse3"] {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }
    which_binary("mount.fuse3")
}

/// Spawn a FUSE daemon via mount.fuse3.
pub fn spawn_fuse_mount(mount: &FuseMount, env: &HashMap<String, String>) -> Result<Child> {
    let mount_fuse3 = find_mount_fuse3()
        .ok_or_else(|| miette::miette!("mount.fuse3 not found"))?;

    info!(binary = %mount.binary, mount_point = %mount.mount_point.display(), "Spawning FUSE daemon");

    let type_source = format!("{}#{}", mount.binary, mount.source);

    let mut cmd = Command::new(mount_fuse3);
    cmd.arg(&type_source);
    cmd.arg(&mount.mount_point);
    cmd.arg("-o");
    cmd.arg(&mount.options);

    for (k, v) in env {
        cmd.env(k, v);
    }

    cmd.spawn().into_diagnostic()
}

/// Wait for a mountpoint to become ready.
pub fn wait_for_mount(path: &Path, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if is_mountpoint(path) {
            info!(path = %path.display(), "FUSE mount ready");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(miette::miette!("FUSE mount at {} not ready within {:?}", path.display(), timeout))
}

fn is_mountpoint(path: &Path) -> bool {
    let path_str = path.to_string_lossy();
    std::fs::read_to_string("/proc/mounts")
        .ok()
        .map(|c| c.lines().any(|l| l.split_whitespace().nth(1) == Some(path_str.as_ref())))
        .unwrap_or(false)
}

fn which_binary(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full = dir.join(name);
            if full.is_file() { Some(full) } else { None }
        })
    })
}

/// Search common binary locations that may not be in PATH.
fn find_binary_in_common_paths(name: &str) -> Option<PathBuf> {
    for dir in &["/usr/local/bin", "/usr/bin", "/bin", "/usr/local/sbin", "/usr/sbin", "/sbin"] {
        let full = PathBuf::from(dir).join(name);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

/// Top-level: discover, create device, spawn daemons, wait, update policy.
pub fn setup_fuse_mounts(
    policy: &mut SandboxPolicy,
    env: &HashMap<String, String>,
) -> Result<Vec<Child>> {
    let mounts = discover_fuse_mounts();
    if mounts.is_empty() {
        return Ok(Vec::new());
    }

    info!(count = mounts.len(), "Setting up FUSE mounts from /etc/fstab");
    ensure_fuse_device()?;

    // Build FUSE daemon environment from:
    // 1. Provider env (injected by OpenShell as pod env vars)
    // 2. Process env (inherits pod env vars set by providers)
    let mut fuse_env = env.clone();

    // Inherit relevant env vars from the process environment.
    // Provider credentials are injected as pod env vars and available here.
    for key in &["DATABASE_URL", "OPENERAL_DATABASE_URL", "ANTHROPIC_API_KEY"] {
        if !fuse_env.contains_key(*key) {
            if let Ok(val) = std::env::var(key) {
                fuse_env.insert(key.to_string(), val);
            }
        }
    }

    // Map DATABASE_URL → OPENERAL_DATABASE_URL if the latter isn't set.
    if !fuse_env.contains_key("OPENERAL_DATABASE_URL") {
        if let Some(db_url) = fuse_env.get("DATABASE_URL") {
            info!("Mapping DATABASE_URL → OPENERAL_DATABASE_URL");
            fuse_env.insert("OPENERAL_DATABASE_URL".to_string(), db_url.clone());
        }
    }

    if let Some(db_url) = fuse_env.get("OPENERAL_DATABASE_URL") {
        info!(db_url_len = db_url.len(), db_url_prefix = %&db_url[..db_url.len().min(30)], "OPENERAL_DATABASE_URL resolved");
    }

    if !fuse_env.contains_key("OPENERAL_DATABASE_URL") {
        return Err(miette::miette!(
            "FUSE mounts declared in /etc/fstab but no DATABASE_URL or OPENERAL_DATABASE_URL available. \
             Create an OpenShell provider: openshell provider create --name db --type generic \
             --credential DATABASE_URL=\"host=... user=... dbname=...\""
        ));
    }

    let timeout = Duration::from_secs(30);
    let mut daemons = Vec::new();

    for mount in &mounts {
        if !mount.mount_point.exists() {
            std::fs::create_dir_all(&mount.mount_point).into_diagnostic()?;
        }

        let child = spawn_fuse_mount(mount, &fuse_env)?;
        daemons.push(child);
        wait_for_mount(&mount.mount_point, timeout)?;

        if mount.read_only {
            policy.filesystem.read_only.push(mount.mount_point.clone());
        } else {
            policy.filesystem.read_write.push(mount.mount_point.clone());
        }
    }

    policy.filesystem.read_write.push(PathBuf::from("/dev/fuse"));
    info!(count = daemons.len(), "All FUSE mounts ready");
    Ok(daemons)
}
