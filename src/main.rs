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
    path: Option<PathBuf>,

    #[arg(short, long)]
    json: bool,

    #[arg(short = 'L', long)]
    follow_links: bool,

    /// Include dotfiles and dot-directories.
    #[arg(short = 'H', long = "hidden")]
    include_hidden: bool,

    #[arg(short, long)]
    depth: Option<usize>,

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
        max_depth: cli.depth,
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
                "tree" => Box::new(TreeFormatter::new(false)),
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
            let tree = scanner.scan(
                cli.path
                    .as_deref()
                    .unwrap_or_else(|| std::path::Path::new(".")),
            )?;

            if cli.json {
                let formatter = JsonFormatter;
                println!("{}", formatter.format(&tree));
            } else {
                tree.sort_by_size();
                let formatter = TreeFormatter::new(cli.color.enabled());
                println!("{}", formatter.format(&tree));
            }
        }
    }

    Ok(())
}
