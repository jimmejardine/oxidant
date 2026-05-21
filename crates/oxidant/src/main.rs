// Application entry point. Subcommand surface to be defined as components land.
// See spec/overview.md for the project's intent and spec/components/* for the
// pieces this binary will wire together.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oxidant", version, about = "Rust-native desktop code agent for Rust projects")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Spec graph operations (rebuild-index, validate, diff). See spec/components/spec-tools/.
    Spec,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    match cli.command {
        None => {
            println!("oxidant {} — pre-MVP scaffold. See spec/overview.md.", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Command::Spec) => {
            anyhow::bail!("`oxidant spec` subcommands not yet implemented")
        }
    }
}
