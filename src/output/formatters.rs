use std::sync::Arc;

use crate::tree::{FileTree, TreeNode};

pub trait OutputFormatter {
    fn format(&self, tree: &FileTree) -> String;
}

pub struct JsonFormatter;
pub struct TreeFormatter {
    color: bool,
}

impl TreeFormatter {
    pub fn new(color: bool) -> Self {
        Self { color }
    }
}

impl OutputFormatter for JsonFormatter {
    fn format(&self, tree: &FileTree) -> String {
        #[derive(serde::Serialize)]
        struct JsonOutput {
            total_size: u64,
            total_files: u64,
            total_dirs: u64,
        }

        let output = JsonOutput {
            total_size: tree.total_size,
            total_files: tree.total_files,
            total_dirs: tree.total_dirs,
        };

        serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
    }
}

impl OutputFormatter for TreeFormatter {
    fn format(&self, tree: &FileTree) -> String {
        let mut output = String::new();
        let root = tree.root.read().unwrap();
        output.push_str(&format!("Path: {}\n", root.entry.path.display()));
        output.push_str(&format!(
            "Used: {} · {} files · {} dirs\n\n",
            humansize::format_size(tree.total_size, humansize::BINARY),
            tree.total_files,
            tree.total_dirs
        ));
        output.push_str("SIZE       %       NAME\n");
        drop(root);
        self.format_node(&tree.root, 0, tree.total_size, &mut output);
        output
    }
}

impl TreeFormatter {
    fn format_node(
        &self,
        node: &Arc<std::sync::RwLock<TreeNode>>,
        depth: usize,
        parent_size: u64,
        output: &mut String,
    ) {
        let node_guard = node.read().unwrap();
        let indent = "  ".repeat(depth.saturating_sub(1));
        let size_str = humansize::format_size(node_guard.entry.size, humansize::BINARY);
        let percentage = node_guard
            .entry
            .size
            .saturating_mul(100)
            .checked_div(parent_size)
            .unwrap_or(0);
        let name = node_guard
            .entry
            .path
            .file_name()
            .unwrap_or(node_guard.entry.path.as_os_str())
            .to_string_lossy();

        if depth > 0 {
            let name = if node_guard.entry.is_directory {
                format!("{name}/")
            } else {
                name.to_string()
            };
            let styled_name = if self.color {
                if node_guard.entry.is_directory {
                    format!("\x1b[36m{name}\x1b[0m")
                } else {
                    name
                }
            } else {
                name
            };
            output.push_str(&format!(
                "{indent}{size_str:>10} {percentage:>3}%  {styled_name}\n"
            ));
        }

        for child in &node_guard.children {
            self.format_node(child, depth + 1, node_guard.entry.size, output);
        }
    }
}
