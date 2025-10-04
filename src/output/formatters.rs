use std::sync::Arc;

use crate::tree::{FileTree, TreeNode};

pub trait OutputFormatter {
    fn format(&self, tree: &FileTree) -> String;
}

pub struct JsonFormatter;
pub struct TreeFormatter;

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
        output.push_str(&format!(
            "{}\n",
            tree.root.read().unwrap().entry.path.display()
        ));
        self.format_node(&tree.root, 0, &mut output);
        output
    }
}

impl TreeFormatter {
    fn format_node(
        &self,
        node: &Arc<std::sync::RwLock<TreeNode>>,
        depth: usize,
        output: &mut String,
    ) {
        let node_guard = node.read().unwrap();
        let indent = "  ".repeat(depth);
        let size_str = humansize::format_size(node_guard.entry.size, humansize::BINARY);
        let name = node_guard
            .entry
            .path
            .file_name()
            .unwrap_or(node_guard.entry.path.as_os_str())
            .to_string_lossy();

        output.push_str(&format!("{}{} {}\n", indent, name, size_str));

        for child in &node_guard.children {
            self.format_node(child, depth + 1, output);
        }
    }
}
