use dashmap::DashMap;
use std::path::PathBuf;

/// Tracks active mount sessions
pub struct MountRegistry {
    mounts: DashMap<PathBuf, MountInfo>,
}

#[derive(Debug, Clone)]
pub struct MountInfo {
    pub connection_string: String,
    pub mount_point: PathBuf,
    pub pid: u32,
}

impl Default for MountRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MountRegistry {
    pub fn new() -> Self {
        Self {
            mounts: DashMap::new(),
        }
    }

    pub fn register(&self, info: MountInfo) {
        self.mounts.insert(info.mount_point.clone(), info);
    }

    pub fn unregister(&self, mount_point: &PathBuf) {
        self.mounts.remove(mount_point);
    }

    pub fn list(&self) -> Vec<MountInfo> {
        self.mounts.iter().map(|r| r.value().clone()).collect()
    }
}
