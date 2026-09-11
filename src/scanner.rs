use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crossbeam::channel::{unbounded, Receiver};
use ignore::WalkBuilder;

use crate::tree::{FileEntry, FileTree, TreeNode};

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub follow_links: bool,
    pub max_depth: Option<usize>,
    pub ignore_hidden: bool,
    pub ignore_patterns: Vec<String>,
    pub num_threads: Option<usize>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            follow_links: false,
            max_depth: None,
            ignore_hidden: true,
            ignore_patterns: Vec::new(),
            num_threads: None,
        }
    }
}

pub struct Scanner {
    config: ScanConfig,
}

#[derive(Debug, Clone)]
pub struct ScanProgress {
    pub files_scanned: u64,
    pub total_size: u64,
    pub current_path: Option<PathBuf>,
    pub elapsed: std::time::Duration,
}

/// A scan notification. The completed notification carries the tree produced by
/// the same worker that reported progress, so interactive mode never needs to
/// rescan on the UI thread.
#[derive(Debug)]
pub enum ScanUpdate {
    Progress(ScanProgress),
    Complete(FileTree),
}

impl Scanner {
    pub fn new(config: ScanConfig) -> Self {
        Self { config }
    }

    pub fn scan_with_progress(&self, path: &Path) -> Result<Receiver<ScanUpdate>, ScanError> {
        let start_time = Instant::now();

        if !path.exists() {
            return Err(ScanError::PathNotFound(path.to_path_buf()));
        }

        let (progress_tx, progress_rx) = unbounded::<ScanUpdate>();

        // Clone the path and config for the thread
        let thread_path = path.to_path_buf();
        let config = self.config.clone();

        // Start the scan in a separate thread
        std::thread::spawn(move || {
            let mut last_update_time = Instant::now();
            let mut last_files_scanned = 0;
            let mut last_total_size = 0;
            let entries =
                Self::collect_entries(&thread_path, &config, |files_scanned, total_size, path| {
                    let now = Instant::now();
                    let significant_change = files_scanned - last_files_scanned > 1_000
                        || total_size - last_total_size > 100 * 1024 * 1024;

                    if now.duration_since(last_update_time) >= std::time::Duration::from_millis(500)
                        || significant_change
                    {
                        let update = ScanUpdate::Progress(ScanProgress {
                            files_scanned,
                            total_size,
                            current_path: Some(path.to_path_buf()),
                            elapsed: start_time.elapsed(),
                        });
                        if progress_tx.send(update).is_err() {
                            return false;
                        }
                        last_update_time = now;
                        last_files_scanned = files_scanned;
                        last_total_size = total_size;
                    }
                    true
                });

            let tree = Self::build_tree(&thread_path, entries);
            let _ = progress_tx.send(ScanUpdate::Complete(tree));
        });

        Ok(progress_rx)
    }

    pub fn scan(&self, path: &Path) -> Result<FileTree, ScanError> {
        if !path.exists() {
            return Err(ScanError::PathNotFound(path.to_path_buf()));
        }

        let entries = Self::collect_entries(path, &self.config, |_, _, _| true);
        Ok(Self::build_tree(path, entries))
    }

    fn collect_entries<F>(
        path: &Path,
        config: &ScanConfig,
        mut report_progress: F,
    ) -> Vec<FileEntry>
    where
        F: FnMut(u64, u64, &Path) -> bool,
    {
        let mut walker_builder = WalkBuilder::new(path);
        walker_builder
            .hidden(config.ignore_hidden)
            .follow_links(config.follow_links)
            .max_depth(config.max_depth)
            .threads(config.num_threads.unwrap_or_else(num_cpus::get));

        for pattern in &config.ignore_patterns {
            walker_builder.add_custom_ignore_filename(pattern);
        }

        let mut entries = Vec::new();
        let mut inode_cache = HashMap::new();
        let mut files_scanned = 0;
        let mut total_size: u64 = 0;

        for entry in walker_builder.build().flatten() {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let inode_key = (metadata.dev(), metadata.ino());
            if inode_cache.contains_key(&inode_key) {
                continue;
            }
            inode_cache.insert(inode_key, ());

            // `len` is the logical length. For disk usage, use allocated blocks;
            // this avoids reporting sparse files as consuming their full virtual size.
            let size = metadata.blocks().saturating_mul(512);
            entries.push(FileEntry {
                path: entry.path().to_path_buf(),
                size,
                is_directory: metadata.is_dir(),
                inode: metadata.ino(),
                device: metadata.dev(),
                file_count: u64::from(!metadata.is_dir()),
                dir_count: u64::from(metadata.is_dir()),
            });
            files_scanned += 1;
            total_size = total_size.saturating_add(size);

            if !report_progress(files_scanned, total_size, entry.path()) {
                break;
            }
        }

        entries
    }

    fn build_tree(path: &Path, entries: Vec<FileEntry>) -> FileTree {
        let mut tree = FileTree::new(path.to_path_buf());
        let mut path_to_node: HashMap<PathBuf, Arc<RwLock<TreeNode>>> = HashMap::new();

        for entry in &entries {
            let node = Arc::new(RwLock::new(TreeNode::new(entry.clone())));
            path_to_node.insert(entry.path.clone(), node);
        }

        if !path_to_node.contains_key(path) {
            let root_entry = FileEntry::new(path.to_path_buf(), 0, true);
            let root_node = Arc::new(RwLock::new(TreeNode::new(root_entry)));
            path_to_node.insert(path.to_path_buf(), root_node);
        }

        for entry in &entries {
            if let Some(node) = path_to_node.get(&entry.path) {
                if let Some(parent_path) = entry.path.parent() {
                    if let Some(parent_node) = path_to_node.get(parent_path) {
                        parent_node.write().unwrap().add_child(node.clone());
                    }
                }
            }
        }

        if let Some(root_node) = path_to_node.get(path) {
            tree.root = root_node.clone();
            Self::calculate_cumulative_sizes(&tree.root);
            Self::calculate_totals(&mut tree);
        }

        tree
    }

    fn calculate_cumulative_sizes(node: &Arc<RwLock<TreeNode>>) -> u64 {
        let mut total_size = node.read().unwrap().entry.size;

        for child in &node.read().unwrap().children {
            total_size = total_size.saturating_add(Self::calculate_cumulative_sizes(child));
        }

        node.write().unwrap().entry.size = total_size;
        total_size
    }

    fn calculate_totals(tree: &mut FileTree) {
        let root = tree.root.read().unwrap();
        tree.total_size = root.entry.size;

        let (files, dirs) = Self::count_files_dirs(&tree.root);
        tree.total_files = files;
        tree.total_dirs = dirs;
    }

    fn count_files_dirs(node: &Arc<RwLock<TreeNode>>) -> (u64, u64) {
        let node_guard = node.read().unwrap();
        let mut files: u64 = if node_guard.entry.is_directory { 0 } else { 1 };
        let mut dirs: u64 = if node_guard.entry.is_directory { 1 } else { 0 };

        for child in &node_guard.children {
            let (child_files, child_dirs) = Self::count_files_dirs(child);
            files = files.saturating_add(child_files);
            dirs = dirs.saturating_add(child_dirs);
        }

        (files, dirs)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::os::unix::fs::MetadataExt;
    use std::time::Duration;

    use tempfile::tempdir;

    use super::{ScanConfig, ScanUpdate, Scanner};

    #[test]
    fn reports_allocated_size_for_sparse_files() {
        let directory = tempdir().unwrap();
        let sparse_file = directory.path().join("sparse.img");
        File::create(&sparse_file)
            .unwrap()
            .set_len(1024 * 1024 * 1024)
            .unwrap();

        let tree = Scanner::new(ScanConfig::default())
            .scan(directory.path())
            .unwrap();
        let root = tree.root.read().unwrap();
        let sparse_size = root
            .children
            .iter()
            .find(|node| node.read().unwrap().entry.path == sparse_file)
            .unwrap()
            .read()
            .unwrap()
            .entry
            .size;
        let expected_size = std::fs::metadata(&sparse_file)
            .unwrap()
            .blocks()
            .saturating_mul(512);

        assert_eq!(sparse_size, expected_size);
        assert!(sparse_size < 1024 * 1024 * 1024);
    }

    #[test]
    fn ignores_hidden_entries_by_default() {
        let directory = tempdir().unwrap();
        std::fs::write(directory.path().join("visible"), "visible").unwrap();
        std::fs::write(directory.path().join(".hidden"), "hidden").unwrap();

        let tree = Scanner::new(ScanConfig::default())
            .scan(directory.path())
            .unwrap();

        assert_eq!(tree.total_files, 1);
    }

    #[test]
    fn progress_scan_returns_the_completed_tree() {
        let directory = tempdir().unwrap();
        std::fs::write(directory.path().join("file"), "contents").unwrap();

        let updates = Scanner::new(ScanConfig::default())
            .scan_with_progress(directory.path())
            .unwrap();
        let completion = loop {
            match updates.recv_timeout(Duration::from_secs(5)).unwrap() {
                ScanUpdate::Progress(_) => continue,
                ScanUpdate::Complete(tree) => break tree,
            }
        };

        assert_eq!(completion.total_files, 1);
    }
}
