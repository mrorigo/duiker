use tui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph, Row, Table, Tabs, Wrap},
    Frame,
};

use crate::scanner::ScanProgress;
use crate::tree::TreeNode;
use crate::ui::tui::{App, ViewMode};
use std::sync::{Arc, RwLock};

pub fn render_header<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let current_path = app.current_path.to_string_lossy();
    let title = if app.is_scanning {
        format!("dirstat - Scanning [{}]", current_path)
    } else {
        format!("dirstat - Modern Disk Usage Analyzer [{}]", current_path)
    };

    let header = Paragraph::new(title)
        .style(
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

pub fn render_tabs<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let titles = vec![
        Spans::from("Tree View (1)"),
        Spans::from("Chart View (2)"),
        Spans::from("Details (3)"),
        Spans::from("Treemap (4)"),
    ];
    let tabs = Tabs::new(titles)
        .select(app.view_mode as usize)
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(tabs, area);
}

pub fn render_status_bar<B: Backend>(f: &mut Frame<B>, app: &App, area: Rect) {
    let selected_info = if let Some(selected_node) = app.get_selected_node() {
        let node_guard = selected_node.read().unwrap();
        format!(
            " | Selected: {} ({})",
            node_guard
                .entry
                .path
                .file_name()
                .unwrap_or(std::ffi::OsStr::new(""))
                .to_string_lossy(),
            humansize::format_size(node_guard.entry.size, humansize::BINARY)
        )
    } else {
        String::new()
    };

    let status = if app.is_scanning {
        "Scanning... (Press 'q' to quit)".to_string()
    } else if let Some(tree) = &app.tree {
        format!(
            "Total: {} in {} files, {} dirs{} | ↑↓:Navigate Enter:Open Backspace:Up q:Quit",
            humansize::format_size(tree.total_size, humansize::BINARY),
            tree.total_files,
            tree.total_dirs,
            selected_info
        )
    } else {
        "No data | ↑↓:Navigate Enter:Open Backspace:Up q:Quit".to_string()
    };

    let status_bar = Paragraph::new(status)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(status_bar, area);
}

pub fn render_main_content<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    if app.tree.is_none() && !app.is_scanning {
        render_no_data(f, area);
        return;
    }

    match app.view_mode {
        ViewMode::Tree => render_tree_view(f, app, area),
        ViewMode::Chart => render_chart_view(f, app, area),
        ViewMode::Details => render_details_view(f, app, area),
        ViewMode::Treemap => render_treemap_view(f, app, area),
    }
}

fn render_no_data<B: Backend>(f: &mut Frame<B>, area: Rect) {
    let message = "No scan data available. Run a scan first.";

    let paragraph = Paragraph::new(message)
        .block(Block::default().title("No Data").borders(Borders::ALL))
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::Yellow));

    f.render_widget(paragraph, area);
}

fn render_tree_view<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)].as_ref())
        .split(area);

    render_file_tree(f, app, chunks[0]);
    render_size_distribution(f, app, chunks[1]);
}

fn render_file_tree<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let block = Block::default().title("File Tree").borders(Borders::ALL);

    let children = app.get_current_children();
    let (header, rows) = build_tree_table(&children, app.selected_index);

    let table = Table::new(rows)
        .header(
            header.style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(block)
        .widths(&[
            Constraint::Length(3),  // Selection indicator
            Constraint::Length(2),  // Icon
            Constraint::Min(30),    // Name (flexible)
            Constraint::Length(12), // Size (fixed)
        ]);

    f.render_widget(table, area);
}

fn build_tree_table(children: &[Arc<RwLock<TreeNode>>], selected_index: usize) -> (Row, Vec<Row>) {
    let header = Row::new(vec![
        "".to_string(), // Selection indicator
        "".to_string(), // Icon
        "Name".to_string(),
        "Size".to_string(),
    ]);

    let rows: Vec<Row> = children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let child_guard = child.read().unwrap();
            let name = child_guard
                .entry
                .path
                .file_name()
                .unwrap_or(child_guard.entry.path.as_os_str())
                .to_string_lossy()
                .to_string();

            let size_str = humansize::format_size(child_guard.entry.size, humansize::BINARY);

            // Add slash for directories
            let display_name = if child_guard.entry.is_directory {
                format!("{}/", name)
            } else {
                name
            };

            let icon = if child_guard.entry.is_directory {
                "📁"
            } else {
                "📄"
            };
            let selector = if index == selected_index { "▶" } else { "" };

            Row::new(vec![
                selector.to_string(),
                icon.to_string(),
                display_name,
                size_str,
            ])
        })
        .collect();

    (header, rows)
}

fn render_size_distribution<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let block = Block::default()
        .title("Largest Items in Current Directory")
        .borders(Borders::ALL);

    let children = app.get_current_children();
    let content = build_top_items_list(&children);

    let list = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(list, area);
}

fn build_top_items_list(children: &[Arc<RwLock<TreeNode>>]) -> Vec<Spans> {
    let mut entries: Vec<(String, u64, bool)> = children
        .iter()
        .map(|child| {
            let child_guard = child.read().unwrap();
            let name = child_guard
                .entry
                .path
                .file_name()
                .unwrap_or(child_guard.entry.path.as_os_str())
                .to_string_lossy()
                .to_string();
            (name, child_guard.entry.size, child_guard.entry.is_directory)
        })
        .collect();

    // Sort by size and take top 10
    entries.sort_by(|a, b| b.1.cmp(&a.1));
    entries.truncate(10);

    let mut items = Vec::new();
    for (name, size, is_dir) in entries {
        let size_str = humansize::format_size(size, humansize::BINARY);
        let icon = if is_dir { "📁 " } else { "📄 " };

        // Create aligned display with fixed width for size
        let display_text = format!("{} {:40} {:>12}", icon, name, size_str);
        items.push(Spans::from(display_text));
    }

    if items.is_empty() {
        items.push(Spans::from("No items found"));
    }

    items
}

fn render_details_view<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let block = Block::default().title("File Details").borders(Borders::ALL);

    let children = app.get_current_children();
    let (header, rows) = build_file_table(&children, app.selected_index);

    let table = Table::new(rows)
        .header(
            header.style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(block)
        .widths(&[
            Constraint::Length(3),  // Selection indicator
            Constraint::Length(2),  // Icon
            Constraint::Min(30),    // Name
            Constraint::Length(12), // Size
            Constraint::Length(8),  // Type
        ]);

    f.render_widget(table, area);
}

fn build_file_table(children: &[Arc<RwLock<TreeNode>>], selected_index: usize) -> (Row, Vec<Row>) {
    let header = Row::new(vec![
        "".to_string(), // Selection indicator
        "".to_string(), // Icon
        "Name".to_string(),
        "Size".to_string(),
        "Type".to_string(),
    ]);

    let rows: Vec<Row> = children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let child_guard = child.read().unwrap();
            let name = child_guard
                .entry
                .path
                .file_name()
                .unwrap_or(child_guard.entry.path.as_os_str())
                .to_string_lossy()
                .to_string();
            let size_str = humansize::format_size(child_guard.entry.size, humansize::BINARY);
            let type_str = if child_guard.entry.is_directory {
                "DIR"
            } else {
                "FILE"
            };

            let icon = if child_guard.entry.is_directory {
                "📁"
            } else {
                "📄"
            };
            let selector = if index == selected_index { "▶" } else { "" };

            Row::new(vec![
                selector.to_string(),
                icon.to_string(),
                name,
                size_str,
                type_str.to_string(),
            ])
        })
        .collect();

    (header, rows)
}

fn render_chart_view<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)].as_ref())
        .split(area);

    render_size_breakdown(f, app, chunks[0]);
    render_usage_summary(f, app, chunks[1]);
}

fn render_size_breakdown<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let block = Block::default()
        .title("Size Breakdown")
        .borders(Borders::ALL);

    let content = if let Some(tree) = &app.tree {
        build_size_breakdown_text(&tree.root)
    } else {
        "No data available".to_string()
    };

    let breakdown = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(breakdown, area);
}

fn build_size_breakdown_text(node: &Arc<RwLock<TreeNode>>) -> String {
    let entries = collect_all_entries(node);
    if entries.is_empty() {
        return "No files found".to_string();
    }

    let total_size: u64 = entries.iter().map(|(_, size, _)| size).sum();
    if total_size == 0 {
        return "All files are empty".to_string();
    }

    let mut breakdown = String::from("Size distribution:\n\n");

    // Group by file type and calculate percentages
    let mut file_types = std::collections::HashMap::new();
    for (name, size, is_dir) in entries {
        let category = if is_dir {
            "Directories".to_string()
        } else {
            // Simple file type detection
            if let Some(ext) = std::path::Path::new(&name).extension() {
                format!("*.{}", ext.to_string_lossy())
            } else {
                "No extension".to_string()
            }
        };
        *file_types.entry(category).or_insert(0) += size;
    }

    let mut sorted_types: Vec<_> = file_types.into_iter().collect();
    sorted_types.sort_by(|a, b| b.1.cmp(&a.1));

    for (category, size) in sorted_types.into_iter().take(10) {
        let percentage = (size * 100) / total_size;
        breakdown.push_str(&format!(
            "{:12} {:>6}% {}\n",
            humansize::format_size(size, humansize::BINARY),
            percentage,
            category
        ));
    }

    breakdown
}

fn collect_all_entries(node: &Arc<RwLock<TreeNode>>) -> Vec<(String, u64, bool)> {
    let mut entries = Vec::new();
    collect_entries_recursive(node, &mut entries);
    entries
}

fn collect_entries_recursive(node: &Arc<RwLock<TreeNode>>, entries: &mut Vec<(String, u64, bool)>) {
    let node_guard = node.read().unwrap();

    let name = node_guard
        .entry
        .path
        .file_name()
        .unwrap_or(node_guard.entry.path.as_os_str())
        .to_string_lossy()
        .to_string();

    entries.push((name, node_guard.entry.size, node_guard.entry.is_directory));

    for child in &node_guard.children {
        collect_entries_recursive(child, entries);
    }
}

fn render_usage_summary<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let block = Block::default()
        .title("Usage Summary")
        .borders(Borders::ALL);

    let summary = if let Some(tree) = &app.tree {
        let largest = find_largest_item(&tree.root);
        let avg_size = if tree.total_files > 0 {
            tree.total_size / tree.total_files
        } else {
            0
        };

        format!(
            "Total Size: {}\nFiles: {}\nDirectories: {}\nLargest Item: {}\nAverage File Size: {}",
            humansize::format_size(tree.total_size, humansize::BINARY),
            tree.total_files,
            tree.total_dirs,
            largest,
            humansize::format_size(avg_size, humansize::BINARY)
        )
    } else {
        "No data available".to_string()
    };

    let summary_widget = Paragraph::new(summary)
        .block(block)
        .wrap(Wrap { trim: true });

    f.render_widget(summary_widget, area);
}

fn find_largest_item(node: &Arc<RwLock<TreeNode>>) -> String {
    let entries = collect_all_entries(node);
    if let Some((name, size, _)) = entries.into_iter().max_by_key(|(_, size, _)| *size) {
        format!(
            "{} ({})",
            name,
            humansize::format_size(size, humansize::BINARY)
        )
    } else {
        "None".to_string()
    }
}

fn render_treemap_view<B: Backend>(f: &mut Frame<B>, app: &mut App, area: Rect) {
    let block = Block::default()
        .title("Size Visualization")
        .borders(Borders::ALL);

    let content = if let Some(tree) = &app.tree {
        build_treemap_visualization(&tree.root)
    } else {
        "No data available for visualization".to_string()
    };

    let treemap = Paragraph::new(content)
        .block(block)
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::LightBlue));

    f.render_widget(treemap, area);
}

fn build_treemap_visualization(node: &Arc<RwLock<TreeNode>>) -> String {
    let entries = collect_all_entries(node);
    if entries.is_empty() {
        return "No files to display".to_string();
    }

    let total_size: u64 = entries.iter().map(|(_, size, _)| size).sum();
    if total_size == 0 {
        return "All files are empty".to_string();
    }

    let mut visualization = String::from("Size-based visualization:\n\n");

    // Show top items with relative size indicators
    let mut top_entries: Vec<_> = entries
        .into_iter()
        .filter(|(_, size, _)| *size > 0)
        .collect();
    top_entries.sort_by(|a, b| b.1.cmp(&a.1));
    top_entries.truncate(15);

    for (name, size, is_dir) in top_entries {
        let percentage = (size * 100) / total_size;
        let bars = "█".repeat((percentage / 2).max(1) as usize); // 2% per bar
        let icon = if is_dir { "📁" } else { "📄" };
        visualization.push_str(&format!("{} {} {:3}% {}\n", icon, bars, percentage, name));
    }

    visualization
}

pub fn render_scan_progress<B: Backend>(f: &mut Frame<B>, progress: &ScanProgress, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ]
            .as_ref(),
        )
        .split(area);

    let progress_text = Paragraph::new(format!(
        "Scanning... {} files, {}",
        progress.files_scanned,
        humansize::format_size(progress.total_size, humansize::BINARY)
    ))
    .block(Block::default().title("Progress").borders(Borders::ALL))
    .wrap(Wrap { trim: true });

    let current_file = progress
        .current_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "Initializing...".to_string());

    let file_display = Paragraph::new(current_file)
        .block(Block::default().title("Current File").borders(Borders::ALL))
        .wrap(Wrap { trim: true });

    let stats = format!(
        "Total Size: {}\nElapsed Time: {:.2}s\nFiles Scanned: {}",
        humansize::format_size(progress.total_size, humansize::BINARY),
        progress.elapsed.as_secs_f32(),
        progress.files_scanned
    );

    let stats_display = Paragraph::new(stats)
        .block(Block::default().title("Statistics").borders(Borders::ALL))
        .wrap(Wrap { trim: true });

    f.render_widget(progress_text, chunks[0]);
    f.render_widget(file_display, chunks[1]);
    f.render_widget(stats_display, chunks[2]);
}
