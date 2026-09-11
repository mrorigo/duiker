use std::sync::Arc;

use crate::tree::{FileTree, TreeNode};

pub trait OutputFormatter {
    fn format(&self, tree: &FileTree) -> String;
}

pub struct JsonFormatter;
pub struct TreeFormatter {
    color: bool,
    max_depth: Option<usize>,
}

impl TreeFormatter {
    pub fn new(color: bool, max_depth: Option<usize>) -> Self {
        Self { color, max_depth }
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

impl JsonFormatter {
    pub fn format_many(trees: &[FileTree]) -> String {
        let outputs: Vec<_> = trees
            .iter()
            .map(|tree| {
                serde_json::json!({
                    "path": tree.root.read().unwrap().entry.path,
                    "total_size": tree.total_size,
                    "total_files": tree.total_files,
                    "total_dirs": tree.total_dirs,
                })
            })
            .collect();
        serde_json::to_string_pretty(&outputs).unwrap_or_else(|_| "[]".to_string())
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
        let root = tree.root.read().unwrap();
        let children = root.children.clone();
        let root_size = root.entry.size;
        drop(root);
        if self.max_depth == Some(0) {
            return output;
        }
        for (index, child) in children.iter().enumerate() {
            self.format_node(
                child,
                1,
                root_size,
                &[],
                index + 1 == children.len(),
                &mut output,
            );
        }
        output
    }
}

impl TreeFormatter {
    fn format_node(
        &self,
        node: &Arc<std::sync::RwLock<TreeNode>>,
        depth: usize,
        parent_size: u64,
        ancestors_last: &[bool],
        is_last: bool,
        output: &mut String,
    ) {
        let node_guard = node.read().unwrap();
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
            let tree_prefix = if depth == 1 {
                String::new()
            } else {
                let mut prefix = String::new();
                for ancestor_is_last in ancestors_last
                    .iter()
                    .take(ancestors_last.len().saturating_sub(1))
                {
                    prefix.push_str(if *ancestor_is_last { "    " } else { "|   " });
                }
                prefix.push_str(if is_last { "\\-- " } else { "|-- " });
                prefix
            };
            output.push_str(&format!(
                "{size_str:>10} {percentage:>3}%  {tree_prefix}{styled_name}\n"
            ));
        }

        let children = node_guard.children.clone();
        let node_size = node_guard.entry.size;
        let has_children = !children.is_empty();
        drop(node_guard);
        if self.max_depth.is_some_and(|max_depth| depth >= max_depth) {
            if has_children {
                let indent = "    ".repeat(depth);
                output.push_str(&format!("{indent}… descendants omitted\n"));
            }
            return;
        }
        let mut child_ancestors = ancestors_last.to_vec();
        if depth > 0 {
            child_ancestors.push(is_last);
        }
        for (index, child) in children.iter().enumerate() {
            self.format_node(
                child,
                depth + 1,
                node_size,
                &child_ancestors,
                index + 1 == children.len(),
                output,
            );
        }
    }
}
