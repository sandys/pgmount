use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Maps workspace file paths to inode numbers and vice versa.
/// Simpler than NodeIdentity-based InodeTable — uses String paths directly.
pub struct WorkspaceInodeTable {
    path_to_ino: DashMap<String, u64>,
    ino_to_path: DashMap<u64, String>,
    next_ino: AtomicU64,
}

impl Default for WorkspaceInodeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceInodeTable {
    pub fn new() -> Self {
        let table = Self {
            path_to_ino: DashMap::new(),
            ino_to_path: DashMap::new(),
            next_ino: AtomicU64::new(2), // 1 is reserved for root
        };
        // Pre-register root
        table.path_to_ino.insert("/".to_string(), 1);
        table.ino_to_path.insert(1, "/".to_string());
        table
    }

    /// Get or allocate an inode for the given path.
    pub fn get_or_insert(&self, path: &str) -> u64 {
        if let Some(ino) = self.path_to_ino.get(path) {
            return *ino;
        }
        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        self.path_to_ino.insert(path.to_string(), ino);
        self.ino_to_path.insert(ino, path.to_string());
        ino
    }

    /// Look up path by inode number.
    pub fn get_path(&self, ino: u64) -> Option<String> {
        self.ino_to_path.get(&ino).map(|r| r.clone())
    }

    /// Look up inode by path.
    pub fn get_ino(&self, path: &str) -> Option<u64> {
        self.path_to_ino.get(path).map(|r| *r)
    }

    /// Remove a path mapping (used on unlink/rmdir).
    pub fn remove(&self, path: &str) {
        if let Some((_, ino)) = self.path_to_ino.remove(path) {
            self.ino_to_path.remove(&ino);
        }
    }

    /// Rename: remove old path, insert new path with a fresh inode.
    pub fn rename(&self, old_path: &str, new_path: &str) {
        self.remove(old_path);
        self.get_or_insert(new_path);
    }
}
