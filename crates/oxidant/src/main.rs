// Application entry point. Launches the GUI by default on the current
// working directory; `oxidant spec` subcommands are CLI affordances that
// don't open a window — see spec_cli.rs.

mod spec_cli;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use oxidant_gui::launch_gui;
use oxidant_providers::{OllamaConfig, OllamaProvider, Provider};

use crate::spec_cli::SpecCommand;

#[derive(Parser)]
#[command(
    name = "oxidant",
    version,
    about = "Rust-native desktop code agent for Rust projects"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Override the workspace root (defaults to the current working directory).
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    /// Server preset for the local LLM: textgen | lmstudio | ollama | llamacpp.
    #[arg(long, default_value = "textgen", global = true)]
    preset: String,
    /// Override the base URL (e.g. http://localhost:1234/v1).
    #[arg(long, global = true)]
    base_url: Option<String>,
    /// Model id to use. If omitted, queried from /v1/models on startup.
    #[arg(long, global = true)]
    model: Option<String>,
    /// Optional system prompt.
    #[arg(long, global = true)]
    system: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch the desktop GUI (default behaviour when no subcommand is given).
    Gui,
    /// Spec graph operations — wraps the same tools the agent uses.
    #[command(subcommand)]
    Spec(SpecCommand),
}

fn main() -> anyhow::Result<ExitCode> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let workspace = cli
        .workspace
        .clone()
        .unwrap_or_else(|| std::env::current_dir().expect("no current_dir"));

    match cli.command {
        Some(Command::Spec(cmd)) => spec_cli::run(workspace, cmd),
        None | Some(Command::Gui) => run_gui(&cli, workspace).map(|_| ExitCode::SUCCESS),
    }
}

fn run_gui(cli: &Cli, workspace: PathBuf) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();

    let mut config = match cli.preset.as_str() {
        "ollama" => OllamaConfig::ollama(),
        "llamacpp" => OllamaConfig::llamacpp(),
        "lmstudio" => OllamaConfig::lmstudio(),
        "textgen" => OllamaConfig::textgen(),
        other => anyhow::bail!(
            "unknown preset {other:?}; expected one of textgen | lmstudio | ollama | llamacpp"
        ),
    };
    if let Some(url) = &cli.base_url {
        config.base_url = url.clone();
    }
    let endpoint = config.base_url.clone();
    let provider_name = config.name.clone();

    let model = match cli.model.clone() {
        Some(m) => m,
        None => handle
            .block_on(first_loaded_model(&config))
            .unwrap_or_else(|e| {
                tracing::warn!(
                    "could not auto-detect model from {endpoint}/models ({e}); falling back to placeholder"
                );
                "auto".to_string()
            }),
    };
    tracing::info!(provider = %provider_name, endpoint = %endpoint, model = %model, "launching gui");

    let provider: Arc<dyn Provider> = Arc::new(OllamaProvider::new(config));
    let result = launch_gui(&workspace, provider, model, cli.system.clone(), handle);
    runtime.shutdown_background();
    result.map_err(|e| anyhow::anyhow!("eframe: {e}"))
}

async fn first_loaded_model(config: &OllamaConfig) -> anyhow::Result<String> {
    let url = format!("{}/models", config.base_url.trim_end_matches('/'));
    let mut req = reqwest::Client::new().get(&url);
    if let Some(key) = &config.api_key {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("GET {url} returned {status}: {body}");
    }
    let parsed: serde_json::Value = serde_json::from_str(&body)?;
    parsed
        .get("data")
        .and_then(|d| d.as_array())
        .and_then(|arr| arr.first())
        .and_then(|m| m.get("id"))
        .and_then(|id| id.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("/v1/models returned no usable model"))
}
