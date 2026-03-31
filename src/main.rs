mod build;
mod init;
mod parse;
mod render;
mod serve;
mod validate;
mod xref;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "leandown", about = "Convert annotated Lean 4 files to a static website")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a frontend bundle in <root>/leandown_site/ (run once per project)
    Init {
        /// Root directory of the Lean project
        #[arg(default_value = ".")]
        root: PathBuf,
    },
    /// Build the site from annotated Lean files
    Build {
        /// Root directory of the Lean project
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Output directory for generated HTML (default: <root>/leandown_site/output)
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Build and serve with live reload on Lean file changes
    Serve {
        /// Root directory of the Lean project
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Output directory for generated HTML (default: <root>/leandown_site/output)
        #[arg(long)]
        output: Option<PathBuf>,
        /// Port to serve on
        #[arg(long, default_value = "8000")]
        port: u16,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { root } => {
            let root = root.canonicalize().unwrap_or(root);
            init::init(&root)?;
        }
        Command::Build { root, output } => {
            let root = root.canonicalize()?;
            let output = resolve_output(&root, output);
            build::build(&root, &output)?;
        }
        Command::Serve { root, output, port } => {
            let root = root.canonicalize()?;
            let output = resolve_output(&root, output);
            serve::serve(&root, &output, port)?;
        }
    }

    Ok(())
}

fn resolve_output(root: &std::path::Path, output: Option<PathBuf>) -> PathBuf {
    output
        .map(|p| p.canonicalize().unwrap_or(p))
        .unwrap_or_else(|| root.join("leandown_site").join("output"))
}
