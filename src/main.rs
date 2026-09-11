use clap::{Parser, Subcommand, ValueEnum};
use rdirstat::output::formatters::{JsonFormatter, OutputFormatter, TreeFormatter};
use rdirstat::scanner::{ScanConfig, Scanner};
use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rdirstat")]
#[command(about = "A modern, high-performance disk usage analyzer")]
#[command(version = "0.1.0")]
struct Cli {
    /// One or more files or directories to scan.
    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,

    #[arg(short, long)]
    json: bool,

    #[arg(short = 'L', long)]
    follow_links: bool,

    /// Include dotfiles and dot-directories.
    #[arg(short = 'H', long = "hidden")]
    include_hidden: bool,

    /// Limit filesystem traversal; truncated scans produce incomplete totals.
    #[arg(long = "scan-depth", alias = "max-depth")]
    scan_depth: Option<usize>,

    /// Limit rendered tree depth without affecting directory summaries.
    #[arg(short = 'd', long, default_value_t = 3)]
    max_reporting_depth: usize,

    #[arg(short, long)]
    threads: Option<usize>,

    /// Colourize human-readable output.
    #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl ColorMode {
    fn enabled(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive TUI for an optional path.
    Interactive {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Export results to file
    Export {
        #[arg(short, long)]
        format: String,

        #[arg(short, long)]
        output: PathBuf,

        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let config = ScanConfig {
        follow_links: cli.follow_links,
        ignore_hidden: !cli.include_hidden,
        max_depth: cli.scan_depth,
        num_threads: cli.threads,
        ..Default::default()
    };

    match cli.command {
        Some(Commands::Interactive { path }) => {
            rdirstat::ui::tui::run_app(path, config)?;
        }
        Some(Commands::Export {
            format,
            output,
            path,
        }) => {
            let scanner = Scanner::new(config);
            let tree = scanner.scan(&path)?;
            tree.sort_by_size();

            let formatter: Box<dyn OutputFormatter> = match format.as_str() {
                "json" => Box::new(JsonFormatter),
                "tree" => Box::new(TreeFormatter::new(false, Some(cli.max_reporting_depth))),
                _ => {
                    eprintln!("Unknown format: {}", format);
                    std::process::exit(1);
                }
            };

            let result = formatter.format(&tree);
            std::fs::write(output, result)?;
        }
        None => {
            let scanner = Scanner::new(config);
            let paths = if cli.paths.is_empty() {
                vec![PathBuf::from(".")]
            } else {
                cli.paths
            };
            let trees = paths
                .iter()
                .map(|path| scanner.scan(path))
                .collect::<Result<Vec<_>, _>>()?;

            if cli.json {
                let formatter = JsonFormatter;
                let output = if trees.len() == 1 {
                    formatter.format(&trees[0])
                } else {
                    JsonFormatter::format_many(&trees)
                };
                println!("{output}");
            } else {
                for tree in &trees {
                    tree.sort_by_size();
                }
                let formatter = TreeFormatter::new(cli.color.enabled(), Some(cli.max_reporting_depth));
                let output = trees
                    .iter()
                    .map(|tree| formatter.format(tree))
                    .collect::<Vec<_>>()
                    .join("\n");
                println!("{output}");
            }
        }
    }

    Ok(())
}
