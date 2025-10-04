use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub is_directory: bool,
    pub inode: u64,
    pub device: u64,
    pub file_count: u64,
    pub dir_count: u64,
}

impl FileEntry {
    pub fn new(path: PathBuf, size: u64, is_directory: bool) -> Self {
        Self {
            path,
            size,
            is_directory,
            inode: 0,
            device: 0,
            file_count: if is_directory { 0 } else { 1 },
            dir_count: if is_directory { 1 } else { 0 },
        }
    }
}

#[derive(Debug)]
pub struct TreeNode {
    pub entry: FileEntry,
    pub children: Vec<Arc<RwLock<TreeNode>>>,
}

impl TreeNode {
    pub fn new(entry: FileEntry) -> Self {
        Self {
            entry,
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, child: Arc<RwLock<TreeNode>>) {
        self.children.push(child);
    }

    pub fn sort_children_by_size(&mut self) {
        self.children.sort_by(|a, b| {
            let a_size = a.read().unwrap().entry.size;
            let b_size = b.read().unwrap().entry.size;
            b_size.cmp(&a_size)
        });

        for child in &self.children {
            child.write().unwrap().sort_children_by_size();
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FileTree {
    #[serde(skip_serializing)] // Skip the complex RwLock in serialization
    pub root: Arc<RwLock<TreeNode>>,
    pub total_size: u64,
    pub total_files: u64,
    pub total_dirs: u64,
}

impl FileTree {
    pub fn new(root_path: PathBuf) -> Self {
        let root_entry = FileEntry::new(root_path, 0, true);
        Self {
            root: Arc::new(RwLock::new(TreeNode::new(root_entry))),
            total_size: 0,
            total_files: 0,
            total_dirs: 0,
        }
    }

    pub fn sort_by_size(&self) {
        self.root.write().unwrap().sort_children_by_size();
    }
}
