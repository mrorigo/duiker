use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tui::{backend::CrosstermBackend, Terminal};

use crate::scanner::{ScanConfig, ScanProgress, ScanUpdate, Scanner};
use crate::tree::{FileTree, TreeNode};

pub use super::components::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ViewMode {
    Tree,
    Chart,
    Details,
    Treemap,
}

#[derive(Debug)]
pub struct App {
    pub tree: Option<FileTree>,
    pub current_path: PathBuf,
    pub scan_progress: Option<ScanProgress>,
    pub view_mode: ViewMode,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub is_scanning: bool,
    pub error_message: Option<String>,
    // Navigation state
    pub current_node: Option<Arc<RwLock<TreeNode>>>,
    pub node_stack: Vec<Arc<RwLock<TreeNode>>>,
    // Async scanning
    pub progress_receiver: Option<crossbeam::channel::Receiver<ScanUpdate>>,
    // Store the scan config for consistent behavior
    pub scan_config: ScanConfig,
}

impl App {
    pub fn new(path: PathBuf, config: ScanConfig) -> Self {
        Self {
            tree: None,
            current_path: path,
            scan_progress: None,
            view_mode: ViewMode::Tree,
            selected_index: 0,
            scroll_offset: 0,
            is_scanning: false,
            error_message: None,
            current_node: None,
            node_stack: Vec::new(),
            progress_receiver: None,
            scan_config: config,
        }
    }

    pub fn start_scan(&mut self) {
        self.is_scanning = true;
        self.error_message = None;
        self.scan_progress = Some(ScanProgress {
            files_scanned: 0,
            total_size: 0,
            current_path: Some(self.current_path.clone()),
            elapsed: Duration::from_secs(0),
        });

        let path = self.current_path.clone();
        let config = self.scan_config.clone();
        let scanner = Scanner::new(config);

        // Start async scan with progress - this is the ONLY scan
        match scanner.scan_with_progress(&path) {
            Ok(progress_rx) => {
                self.progress_receiver = Some(progress_rx);
                self.tree = None; // No tree yet - we'll build it when scan completes
                self.is_scanning = true;
            }
            Err(e) => {
                self.error_message = Some(format!("Scan error: {}", e));
                self.is_scanning = false;
            }
        }
    }

    pub fn update_progress(&mut self) {
        if let Some(ref progress_rx) = self.progress_receiver {
            while let Ok(update) = progress_rx.try_recv() {
                match update {
                    ScanUpdate::Progress(progress) => self.scan_progress = Some(progress),
                    ScanUpdate::Complete(tree) => {
                        self.is_scanning = false;
                        self.progress_receiver = None;
                        self.current_node = Some(tree.root.clone());
                        self.tree = Some(tree);
                        break;
                    }
                }
            }
        }
    }

    pub fn on_key(&mut self, key: KeyCode) {
        if self.is_scanning {
            // Only allow quitting during scan
            if let KeyCode::Char('q') = key {
                self.is_scanning = false;
            }
            return;
        }

        match key {
            KeyCode::Char('1') => {
                self.view_mode = ViewMode::Tree;
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            KeyCode::Char('2') => {
                self.view_mode = ViewMode::Chart;
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            KeyCode::Char('3') => {
                self.view_mode = ViewMode::Details;
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            KeyCode::Char('4') => {
                self.view_mode = ViewMode::Treemap;
                self.selected_index = 0;
                self.scroll_offset = 0;
            }
            KeyCode::Down => self.next(),
            KeyCode::Up => self.previous(),
            KeyCode::PageDown => self.page_down(),
            KeyCode::PageUp => self.page_up(),
            KeyCode::Enter => self.enter_selected(),
            KeyCode::Backspace => self.go_up(),
            _ => {}
        }
    }

    fn next(&mut self) {
        if self.view_mode != ViewMode::Tree && self.view_mode != ViewMode::Details {
            return;
        }

        let item_count = self.get_current_children().len();
        if item_count == 0 {
            return;
        }

        if self.selected_index < item_count - 1 {
            self.selected_index += 1;

            let visible_items = 20;
            if self.selected_index >= self.scroll_offset + visible_items {
                self.scroll_offset += 1;
            }
        }
    }

    fn previous(&mut self) {
        if self.view_mode != ViewMode::Tree && self.view_mode != ViewMode::Details {
            return;
        }

        if self.selected_index > 0 {
            self.selected_index -= 1;

            if self.selected_index < self.scroll_offset {
                self.scroll_offset = self.selected_index;
            }
        }
    }

    fn page_down(&mut self) {
        if self.view_mode != ViewMode::Tree && self.view_mode != ViewMode::Details {
            return;
        }

        let item_count = self.get_current_children().len();
        let visible_items = 20;

        self.selected_index =
            (self.selected_index + visible_items).min(item_count.saturating_sub(1));
        self.scroll_offset =
            (self.scroll_offset + visible_items).min(item_count.saturating_sub(visible_items));
    }

    fn page_up(&mut self) {
        if self.view_mode != ViewMode::Tree && self.view_mode != ViewMode::Details {
            return;
        }

        let visible_items = 20;

        self.selected_index = self.selected_index.saturating_sub(visible_items);
        self.scroll_offset = self.scroll_offset.saturating_sub(visible_items);
    }

    fn enter_selected(&mut self) {
        if self.view_mode != ViewMode::Tree {
            return;
        }

        let children = self.get_current_children();
        if let Some(selected_child) = children.get(self.selected_index) {
            let child_guard = selected_child.read().unwrap();

            if child_guard.entry.is_directory {
                if let Some(current) = self.current_node.take() {
                    self.node_stack.push(current);
                }
                self.current_node = Some(selected_child.clone());
                self.selected_index = 0;
                self.scroll_offset = 0;
                self.current_path = child_guard.entry.path.clone();
            } else {
                println!(
                    "File selected: {} ({})",
                    child_guard.entry.path.display(),
                    humansize::format_size(child_guard.entry.size, humansize::BINARY)
                );
            }
        }
    }

    fn go_up(&mut self) {
        if self.view_mode != ViewMode::Tree {
            return;
        }

        if let Some(parent_node) = self.node_stack.pop() {
            let parent_path = {
                let parent_guard = parent_node.read().unwrap();
                parent_guard.entry.path.clone()
            };
            self.current_path = parent_path;
            self.current_node = Some(parent_node);
            self.selected_index = 0;
            self.scroll_offset = 0;
        }
    }

    pub fn get_current_children(&self) -> Vec<Arc<RwLock<TreeNode>>> {
        if let Some(ref current_node) = self.current_node {
            let node_guard = current_node.read().unwrap();
            node_guard.children.clone()
        } else if let Some(ref tree) = self.tree {
            let root_guard = tree.root.read().unwrap();
            root_guard.children.clone()
        } else {
            Vec::new()
        }
    }

    pub fn get_selected_node(&self) -> Option<Arc<RwLock<TreeNode>>> {
        let children = self.get_current_children();
        children.get(self.selected_index).cloned()
    }
}

pub fn run_app(path: PathBuf, config: ScanConfig) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(path, config);

    // Start initial scan with the provided config
    app.start_scan();

    let res = run_app_loop(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Error: {:?}", err);
    }

    Ok(())
}

fn run_app_loop<B: tui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> Result<(), Box<dyn std::error::Error>> {
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        // Update progress from async scanner
        app.update_progress();

        terminal.draw(|f| {
            if app.is_scanning {
                if let Some(progress) = &app.scan_progress {
                    render_scan_progress(f, progress, f.size());
                } else {
                    // Show initial scanning state
                    let progress = ScanProgress {
                        files_scanned: 0,
                        total_size: 0,
                        current_path: Some(app.current_path.clone()),
                        elapsed: Duration::from_secs(0),
                    };
                    render_scan_progress(f, &progress, f.size());
                }
            } else {
                ui(f, &mut app);
            }
        })?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('?') => {
                        println!("Help:");
                        println!("1-4: Switch views");
                        println!("↑↓: Navigate");
                        println!("Enter: Open directory/select file");
                        println!("Backspace: Go up");
                        println!("q: Quit");
                    }
                    _ => app.on_key(key.code),
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    Ok(())
}

fn ui<B: tui::backend::Backend>(f: &mut tui::Frame<B>, app: &mut App) {
    let chunks = tui::layout::Layout::default()
        .direction(tui::layout::Direction::Vertical)
        .margin(1)
        .constraints(
            [
                tui::layout::Constraint::Length(3), // Header
                tui::layout::Constraint::Length(3), // Tabs
                tui::layout::Constraint::Min(10),   // Main content
                tui::layout::Constraint::Length(3), // Status bar
            ]
            .as_ref(),
        )
        .split(f.size());

    render_header(f, app, chunks[0]);
    render_tabs(f, app, chunks[1]);
    render_main_content(f, app, chunks[2]);
    render_status_bar(f, app, chunks[3]);
}
