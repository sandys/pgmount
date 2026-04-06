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

    /// Rename while preserving the moved inode identity.
    ///
    /// This is especially important for replace-style renames (`mv tmp file`),
    /// where the kernel can continue to reference the moved source inode
    /// immediately after the rename completes.
    pub fn rename(&self, old_path: &str, new_path: &str) {
        if old_path == new_path {
            return;
        }

        let old_prefix = format!("{old_path}/");
        let new_prefix = format!("{new_path}/");

        let source_entries: Vec<(String, u64)> = self
            .path_to_ino
            .iter()
            .filter_map(|entry| {
                let path = entry.key().clone();
                if path == old_path || path.starts_with(&old_prefix) {
                    Some((path, *entry.value()))
                } else {
                    None
                }
            })
            .collect();

        if source_entries.is_empty() {
            return;
        }

        let target_paths: Vec<String> = self
            .path_to_ino
            .iter()
            .filter_map(|entry| {
                let path = entry.key().clone();
                if path == new_path || path.starts_with(&new_prefix) {
                    Some(path)
                } else {
                    None
                }
            })
            .collect();

        for path in target_paths {
            self.remove(&path);
        }

        for (path, _) in &source_entries {
            self.path_to_ino.remove(path);
        }

        for (path, ino) in source_entries {
            let renamed_path = if path == old_path {
                new_path.to_string()
            } else {
                format!("{new_path}{}", &path[old_path.len()..])
            };

            self.path_to_ino.insert(renamed_path.clone(), ino);
            self.ino_to_path.insert(ino, renamed_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceInodeTable;

    #[test]
    fn rename_preserves_source_inode_when_replacing_target() {
        let table = WorkspaceInodeTable::new();
        let target_ino = table.get_or_insert("/file");
        let source_ino = table.get_or_insert("/file.tmp");

        table.rename("/file.tmp", "/file");

        assert_eq!(table.get_ino("/file"), Some(source_ino));
        assert_eq!(table.get_path(source_ino), Some("/file".to_string()));
        assert_eq!(table.get_ino("/file.tmp"), None);
        assert_eq!(table.get_path(target_ino), None);
    }

    #[test]
    fn rename_updates_cached_descendants_for_directory_moves() {
        let table = WorkspaceInodeTable::new();
        let dir_ino = table.get_or_insert("/dir");
        let child_ino = table.get_or_insert("/dir/file.txt");
        let nested_dir_ino = table.get_or_insert("/dir/nested");
        let nested_child_ino = table.get_or_insert("/dir/nested/file.txt");
        let replaced_dir_ino = table.get_or_insert("/dest");
        let replaced_child_ino = table.get_or_insert("/dest/old.txt");

        table.rename("/dir", "/dest");

        assert_eq!(table.get_ino("/dest"), Some(dir_ino));
        assert_eq!(table.get_path(dir_ino), Some("/dest".to_string()));
        assert_eq!(table.get_ino("/dest/file.txt"), Some(child_ino));
        assert_eq!(
            table.get_path(child_ino),
            Some("/dest/file.txt".to_string())
        );
        assert_eq!(table.get_ino("/dest/nested"), Some(nested_dir_ino));
        assert_eq!(
            table.get_path(nested_dir_ino),
            Some("/dest/nested".to_string())
        );
        assert_eq!(
            table.get_ino("/dest/nested/file.txt"),
            Some(nested_child_ino)
        );
        assert_eq!(
            table.get_path(nested_child_ino),
            Some("/dest/nested/file.txt".to_string())
        );

        assert_eq!(table.get_ino("/dir"), None);
        assert_eq!(table.get_ino("/dir/file.txt"), None);
        assert_eq!(table.get_ino("/dir/nested"), None);
        assert_eq!(table.get_ino("/dir/nested/file.txt"), None);

        assert_eq!(table.get_path(replaced_dir_ino), None);
        assert_eq!(table.get_path(replaced_child_ino), None);
    }
}
