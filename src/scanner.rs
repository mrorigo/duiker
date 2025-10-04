use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use crossbeam::channel::{bounded, Receiver};
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
    pub is_complete: bool,
}

impl Scanner {
    pub fn new(config: ScanConfig) -> Self {
        Self { config }
    }

    pub fn scan_with_progress(&self, path: &Path) -> Result<Receiver<ScanProgress>, ScanError> {
        let start_time = Instant::now();

        if !path.exists() {
            return Err(ScanError::PathNotFound(path.to_path_buf()));
        }

        let (progress_tx, progress_rx) = bounded::<ScanProgress>(10); // Small buffer

        // Clone the path and config for the thread
        let thread_path = path.to_path_buf();
        let config = self.config.clone();

        // Start the scan in a separate thread
        std::thread::spawn(move || {
            let mut entries = Vec::new();
            let mut inode_cache = HashMap::new();
            let mut files_scanned = 0;
            let mut total_size = 0;
            let mut last_update_time = Instant::now();
            let mut last_files_scanned = 0;
            let mut last_total_size = 0;
            let update_interval = std::time::Duration::from_millis(500); // Update every 500ms

            let mut walker_builder = WalkBuilder::new(&thread_path);
            walker_builder
                .hidden(config.ignore_hidden)
                .follow_links(config.follow_links)
                .max_depth(config.max_depth)
                .threads(config.num_threads.unwrap_or_else(num_cpus::get));

            for pattern in &config.ignore_patterns {
                walker_builder.add_custom_ignore_filename(pattern);
            }

            let walker = walker_builder.build();

            for result in walker {
                match result {
                    Ok(entry) => {
                        if let Ok(metadata) = entry.metadata() {
                            let inode_key = (metadata.dev(), metadata.ino());

                            if inode_cache.contains_key(&inode_key) {
                                continue;
                            }
                            inode_cache.insert(inode_key, metadata.len());

                            let file_entry = FileEntry {
                                path: entry.path().to_path_buf(),
                                size: metadata.len(),
                                is_directory: metadata.is_dir(),
                                inode: metadata.ino(),
                                device: metadata.dev(),
                                file_count: if metadata.is_dir() { 0 } else { 1 },
                                dir_count: if metadata.is_dir() { 1 } else { 0 },
                            };

                            files_scanned += 1;
                            total_size += metadata.len();
                            entries.push(file_entry);

                            // Only send updates if significant changes occurred or time elapsed
                            let now = Instant::now();
                            let significant_change = files_scanned - last_files_scanned > 1000
                                || total_size - last_total_size > 100 * 1024 * 1024; // 100MB

                            if now.duration_since(last_update_time) > update_interval
                                || significant_change
                            {
                                if progress_tx
                                    .send(ScanProgress {
                                        files_scanned,
                                        total_size,
                                        current_path: Some(entry.path().to_path_buf()),
                                        elapsed: start_time.elapsed(),
                                        is_complete: false,
                                    })
                                    .is_err()
                                {
                                    // Receiver dropped, stop scanning
                                    break;
                                }
                                last_update_time = now;
                                last_files_scanned = files_scanned;
                                last_total_size = total_size;
                            }
                        }
                    }
                    Err(_) => {
                        continue;
                    }
                }
            }

            // Send final update with completion flag
            let _ = progress_tx.send(ScanProgress {
                files_scanned,
                total_size,
                current_path: None,
                elapsed: start_time.elapsed(),
                is_complete: true,
            });
        });

        Ok(progress_rx)
    }

    pub fn scan(&self, path: &Path) -> Result<FileTree, ScanError> {
        let start_time = Instant::now();

        if !path.exists() {
            return Err(ScanError::PathNotFound(path.to_path_buf()));
        }

        let mut entries = Vec::new();
        let mut inode_cache = HashMap::new();
        let mut files_scanned = 0;
        let mut total_size = 0;

        let mut walker_builder = WalkBuilder::new(path);
        walker_builder
            .hidden(self.config.ignore_hidden)
            .follow_links(self.config.follow_links)
            .max_depth(self.config.max_depth)
            .threads(self.config.num_threads.unwrap_or_else(num_cpus::get));

        for pattern in &self.config.ignore_patterns {
            walker_builder.add_custom_ignore_filename(pattern);
        }

        let walker = walker_builder.build();

        for result in walker {
            match result {
                Ok(entry) => {
                    if let Ok(metadata) = entry.metadata() {
                        let inode_key = (metadata.dev(), metadata.ino());

                        if inode_cache.contains_key(&inode_key) {
                            continue;
                        }
                        inode_cache.insert(inode_key, metadata.len());

                        let file_entry = FileEntry {
                            path: entry.path().to_path_buf(),
                            size: metadata.len(),
                            is_directory: metadata.is_dir(),
                            inode: metadata.ino(),
                            device: metadata.dev(),
                            file_count: if metadata.is_dir() { 0 } else { 1 },
                            dir_count: if metadata.is_dir() { 1 } else { 0 },
                        };

                        files_scanned += 1;
                        total_size += metadata.len();
                        entries.push(file_entry);
                    }
                }
                Err(_) => {
                    continue;
                }
            }
        }

        println!(
            "Scan completed: {} files, {} in {:.2}s",
            files_scanned,
            humansize::format_size(total_size, humansize::BINARY),
            start_time.elapsed().as_secs_f32()
        );

        Self::build_tree(path, entries)
    }

    fn build_tree(path: &Path, entries: Vec<FileEntry>) -> Result<FileTree, ScanError> {
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

        Ok(tree)
    }

    fn calculate_cumulative_sizes(node: &Arc<RwLock<TreeNode>>) -> u64 {
        let mut total_size = node.read().unwrap().entry.size;

        for child in &node.read().unwrap().children {
            total_size += Self::calculate_cumulative_sizes(child);
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
        let mut files = if node_guard.entry.is_directory { 0 } else { 1 };
        let mut dirs = if node_guard.entry.is_directory { 1 } else { 0 };

        for child in &node_guard.children {
            let (child_files, child_dirs) = Self::count_files_dirs(child);
            files += child_files;
            dirs += child_dirs;
        }

        (files, dirs)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("Path not found: {0}")]
    PathNotFound(PathBuf),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
