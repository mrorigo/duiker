use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crossbeam::channel::{unbounded, Receiver};
use ignore::{WalkBuilder, WalkState};

use crate::tree::{FileEntry, FileTree, TreeNode};

type ProgressCallback = Arc<dyn Fn(u64, u64, &Path) -> bool + Send + Sync>;

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub follow_links: bool,
    pub max_depth: Option<usize>,
    pub ignore_hidden: bool,
    pub respect_ignore_files: bool,
    pub ignore_patterns: Vec<String>,
    pub num_threads: Option<usize>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            follow_links: false,
            max_depth: None,
            ignore_hidden: true,
            respect_ignore_files: false,
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
            let progress_tx_for_callback = progress_tx.clone();
            let progress_state = Mutex::new((Instant::now(), 0u64, 0u64));
            let entries = Self::collect_entries(
                &thread_path,
                &config,
                Arc::new(move |files_scanned, total_size, path: &Path| {
                    let mut state = progress_state.lock().unwrap();
                    let now = Instant::now();
                    let significant_change =
                        files_scanned - state.1 > 1_000 || total_size - state.2 > 100 * 1024 * 1024;

                    if now.duration_since(state.0) >= std::time::Duration::from_millis(500)
                        || significant_change
                    {
                        let update = ScanUpdate::Progress(ScanProgress {
                            files_scanned,
                            total_size,
                            current_path: Some(path.to_path_buf()),
                            elapsed: start_time.elapsed(),
                        });
                        if progress_tx_for_callback.send(update).is_err() {
                            return false;
                        }
                        state.0 = now;
                        state.1 = files_scanned;
                        state.2 = total_size;
                    }
                    true
                }),
            );

            let tree = Self::build_tree(&thread_path, entries);
            let _ = progress_tx.send(ScanUpdate::Complete(tree));
        });

        Ok(progress_rx)
    }

    pub fn scan(&self, path: &Path) -> Result<FileTree, ScanError> {
        if !path.exists() {
            return Err(ScanError::PathNotFound(path.to_path_buf()));
        }

        let entries = Self::collect_entries(path, &self.config, Arc::new(|_, _, _: &Path| true));
        Ok(Self::build_tree(path, entries))
    }

    fn collect_entries(
        path: &Path,
        config: &ScanConfig,
        report_progress: ProgressCallback,
    ) -> Vec<FileEntry> {
        let mut walker_builder = WalkBuilder::new(path);
        let respect_ignore_files = config.respect_ignore_files;
        walker_builder
            .hidden(config.ignore_hidden)
            .ignore(respect_ignore_files)
            .git_ignore(respect_ignore_files)
            .git_global(respect_ignore_files)
            .git_exclude(respect_ignore_files)
            .parents(respect_ignore_files)
            .follow_links(config.follow_links)
            .max_depth(config.max_depth)
            .threads(config.num_threads.unwrap_or_else(num_cpus::get));

        for pattern in &config.ignore_patterns {
            walker_builder.add_custom_ignore_filename(pattern);
        }

        let entries = Arc::new(Mutex::new(Vec::new()));
        let inode_cache = Arc::new(Mutex::new(HashSet::new()));
        let scanned = Arc::new(AtomicU64::new(0));
        let total_size = Arc::new(AtomicU64::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        walker_builder.build_parallel().run(|| {
            let entries = Arc::clone(&entries);
            let inode_cache = Arc::clone(&inode_cache);
            let scanned = Arc::clone(&scanned);
            let total_size = Arc::clone(&total_size);
            let cancelled = Arc::clone(&cancelled);
            let report_progress = Arc::clone(&report_progress);
            Box::new(move |result| {
                if cancelled.load(Ordering::Relaxed) {
                    return WalkState::Quit;
                }
                let Ok(entry) = result else {
                    return WalkState::Continue;
                };
                let Ok(metadata) = entry.metadata() else {
                    return WalkState::Continue;
                };
                if !inode_cache
                    .lock()
                    .unwrap()
                    .insert((metadata.dev(), metadata.ino()))
                {
                    return WalkState::Continue;
                }
                let is_directory = metadata.is_dir();
                let size = metadata.blocks().saturating_mul(512);
                entries.lock().unwrap().push(FileEntry {
                    path: entry.path().to_path_buf(),
                    size,
                    is_directory,
                    inode: metadata.ino(),
                    device: metadata.dev(),
                    file_count: u64::from(!is_directory),
                    dir_count: u64::from(is_directory),
                });
                let count = scanned.fetch_add(1, Ordering::Relaxed) + 1;
                let bytes = total_size
                    .fetch_add(size, Ordering::Relaxed)
                    .saturating_add(size);
                if report_progress(count, bytes, entry.path()) {
                    WalkState::Continue
                } else {
                    cancelled.store(true, Ordering::Relaxed);
                    WalkState::Quit
                }
            })
        });
        Arc::try_unwrap(entries).unwrap().into_inner().unwrap()
    }

    fn build_tree(path: &Path, entries: Vec<FileEntry>) -> FileTree {
        let mut tree = FileTree::new(path.to_path_buf());
        let mut path_to_node: HashMap<PathBuf, usize> = HashMap::new();
        let mut nodes: Vec<(FileEntry, Vec<usize>)> = Vec::with_capacity(entries.len() + 1);

        for entry in &entries {
            let index = nodes.len();
            nodes.push((entry.clone(), Vec::new()));
            path_to_node.insert(entry.path.clone(), index);
        }

        if !path_to_node.contains_key(path) {
            let root_entry = FileEntry::new(path.to_path_buf(), 0, true);
            let index = nodes.len();
            nodes.push((root_entry, Vec::new()));
            path_to_node.insert(path.to_path_buf(), index);
        }

        for entry in &entries {
            if let Some(&node) = path_to_node.get(&entry.path) {
                if let Some(parent_path) = entry.path.parent() {
                    if let Some(&parent_node) = path_to_node.get(parent_path) {
                        nodes[parent_node].1.push(node);
                    }
                }
            }
        }

        if let Some(&root_index) = path_to_node.get(path) {
            tree.root = Self::into_shared(root_index, &nodes);
            Self::calculate_cumulative_sizes(&tree.root);
            Self::calculate_totals(&mut tree);
        }

        tree
    }

    fn into_shared(index: usize, nodes: &[(FileEntry, Vec<usize>)]) -> Arc<RwLock<TreeNode>> {
        let (entry, children) = &nodes[index];
        let mut node = TreeNode::new(entry.clone());
        node.children = children
            .iter()
            .map(|&child| Self::into_shared(child, nodes))
            .collect();
        Arc::new(RwLock::new(node))
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
    fn does_not_honor_gitignore_by_default() {
        let directory = tempdir().unwrap();
        std::fs::create_dir(directory.path().join(".git")).unwrap();
        std::fs::write(directory.path().join(".gitignore"), "/target\n").unwrap();
        std::fs::create_dir(directory.path().join("target")).unwrap();
        std::fs::write(directory.path().join("target/big.bin"), vec![0u8; 4096]).unwrap();

        let default_tree = Scanner::new(ScanConfig::default())
            .scan(directory.path())
            .unwrap();
        assert_eq!(default_tree.total_files, 1);

        let respecting_tree = Scanner::new(ScanConfig {
            respect_ignore_files: true,
            ..Default::default()
        })
        .scan(directory.path())
        .unwrap();
        assert_eq!(respecting_tree.total_files, 0);
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
