mod output;
mod scanner;
mod tree;
mod ui;
mod utils;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

use crate::output::formatters::{JsonFormatter, OutputFormatter, TreeFormatter};
use crate::scanner::{ScanConfig, Scanner};

#[derive(Parser)]
#[command(name = "rdirstat")]
#[command(about = "A modern, high-performance disk usage analyzer")]
#[command(version = "0.1.0")]
struct Cli {
    #[arg(default_value = ".")]
    path: PathBuf,

    #[arg(short, long)]
    json: bool,

    #[arg(short = 'L', long)]
    follow_links: bool,

    #[arg(short = 'H', long)]
    no_hidden: bool,

    #[arg(short, long)]
    depth: Option<usize>,

    #[arg(short, long)]
    threads: Option<usize>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive TUI
    Interactive,
    /// Export results to file
    Export {
        #[arg(short, long)]
        format: String,

        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let config = ScanConfig {
        follow_links: cli.follow_links,
        ignore_hidden: cli.no_hidden, // --no-hidden means ignore_hidden = true
        max_depth: cli.depth,
        num_threads: cli.threads,
        ..Default::default()
    };

    match cli.command {
        Some(Commands::Interactive) => {
            // Pass the config to the interactive mode
            ui::tui::run_app(cli.path, config)?;
        }
        Some(Commands::Export { format, output }) => {
            let scanner = Scanner::new(config);
            let tree = scanner.scan(&cli.path)?;

            let formatter: Box<dyn OutputFormatter> = match format.as_str() {
                "json" => Box::new(JsonFormatter),
                "tree" => Box::new(TreeFormatter),
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
            let tree = scanner.scan(&cli.path)?;

            if cli.json {
                let formatter = JsonFormatter;
                println!("{}", formatter.format(&tree));
            } else {
                let formatter = TreeFormatter;
                println!("{}", formatter.format(&tree));
            }
        }
    }

    Ok(())
}
