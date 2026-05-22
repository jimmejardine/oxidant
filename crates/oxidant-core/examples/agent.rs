// End-to-end agent loop demo against a local OpenAI-compatible server.
//
// Defaults to textgen-webui on http://localhost:5000/v1 with whatever model
// is loaded. Without --no-tools, registers the standard oxidant-tools set
// (fs_read, fs_write, glob, grep, edit_string, apply_edits) PLUS an inline
// demo `current_time` tool. ToolContext.workspace_root defaults to the
// current working directory.
//
//   cargo run -p oxidant-core --example agent -- "summarise spec/overview.md"
//   cargo run -p oxidant-core --example agent -- "what time is it?"
//   cargo run -p oxidant-core --example agent -- --no-tools "haiku about ownership"
//   cargo run -p oxidant-core --example agent -- --preset lmstudio "..."

use std::io::Write;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use clap::Parser;
use serde_json::json;

use oxidant_core::{
    AgentLoopConfig, Conversation, Tool, ToolCategory, ToolContext, ToolRegistry, ToolResult, run,
};
use oxidant_providers::{ChatEvent, OllamaConfig, OllamaProvider};
use tokio_util::sync::CancellationToken;

#[derive(Parser, Debug)]
#[command(about = "Run the oxidant agent loop against a local OpenAI-compatible server")]
struct Args {
    /// Server preset: textgen | lmstudio | ollama | llamacpp
    #[arg(long, default_value = "textgen")]
    preset: String,

    /// Override the base URL (e.g. http://localhost:1234/v1)
    #[arg(long)]
    base_url: Option<String>,

    /// Model id to use. If omitted, queries /v1/models and picks the first.
    #[arg(long)]
    model: Option<String>,

    /// Optional system prompt
    #[arg(long)]
    system: Option<String>,

    /// Max tokens per assistant turn
    #[arg(long, default_value_t = 1024)]
    max_tokens: u32,

    /// Max iterations before bailing
    #[arg(long, default_value_t = 8)]
    max_iterations: usize,

    /// Skip registering the demo `current_time` tool
    #[arg(long)]
    no_tools: bool,

    /// The user prompt (positional)
    prompt: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let args = Args::parse();
    let prompt = if args.prompt.is_empty() {
        "What is the current UTC time? Reply in one sentence.".to_string()
    } else {
        args.prompt.join(" ")
    };

    let mut config = match args.preset.as_str() {
        "ollama" => OllamaConfig::ollama(),
        "llamacpp" => OllamaConfig::llamacpp(),
        "lmstudio" => OllamaConfig::lmstudio(),
        "textgen" => OllamaConfig::textgen(),
        other => OllamaConfig::custom(format!("http://localhost:{}/v1", other)),
    };
    if let Some(url) = args.base_url {
        config.base_url = url;
    }

    let model = match args.model {
        Some(m) => m,
        None => first_loaded_model(&config).await?,
    };

    println!(
        "[provider: {} | endpoint: {} | model: {}]",
        config.name, config.base_url, model
    );

    let provider = OllamaProvider::new(config);

    let mut registry = ToolRegistry::new();
    if !args.no_tools {
        oxidant_tools::register_standard_tools(&mut registry);
        oxidant_rust_tools::register_standard_tools(&mut registry);
        oxidant_spec_tools::register_standard_tools(&mut registry);
        registry.register(Arc::new(CurrentTime));
        let mut names: Vec<_> = registry.iter().map(|t| t.name().to_string()).collect();
        names.sort();
        println!("[tools available: {}]", names.join(", "));
    }

    let cwd = std::env::current_dir()?;
    let workspace_root = camino::Utf8PathBuf::from_path_buf(dunce::canonicalize(&cwd)?)
        .map_err(|p| anyhow::anyhow!("non-UTF-8 path: {}", p.display()))?;
    let ctx = ToolContext {
        workspace_root,
        exploration_id: "demo".to_string(),
        cancellation: CancellationToken::new(),
    };

    let mut conv = Conversation::new();
    conv.push_user_text(prompt);

    let mut loop_config = AgentLoopConfig::new(model);
    loop_config.system_prompt = args.system;
    loop_config.max_tokens = args.max_tokens;
    loop_config.max_iterations = args.max_iterations;
    if !args.no_tools {
        loop_config.post_edit_check_tool = Some("spec_diff".into());
    }

    let mut stdout = std::io::stdout().lock();
    let outcome = run(
        &provider,
        &registry,
        &ctx,
        &mut conv,
        &loop_config,
        |event| match event {
            ChatEvent::TextDelta(s) => {
                let _ = write!(stdout, "{s}");
                let _ = stdout.flush();
            }
            ChatEvent::ThinkingDelta(s) => {
                let _ = write!(stdout, "[thinking] {s}");
                let _ = stdout.flush();
            }
            ChatEvent::ToolUseStart { id, name } => {
                let _ = writeln!(stdout, "\n[→ tool {name} id={id}]");
            }
            ChatEvent::ToolUseInputDelta { id, json_delta } => {
                let _ = write!(stdout, "[input id={id}] {json_delta}");
            }
            ChatEvent::ToolUseEnd { id } => {
                let _ = writeln!(stdout, "\n[← tool end id={id}]");
            }
            ChatEvent::Finish { stop_reason, .. } => {
                let _ = writeln!(stdout, "\n[turn finish: {stop_reason:?}]");
            }
            ChatEvent::Error(e) => {
                let _ = writeln!(stdout, "\n[error] {e}");
            }
        },
    )
    .await?;

    writeln!(
        stdout,
        "\n--- agent done: {} iterations | {} tool calls | {} input tokens | {} output tokens ---",
        outcome.iterations,
        outcome.tool_calls_dispatched,
        outcome.total_usage.input_tokens,
        outcome.total_usage.output_tokens
    )?;

    Ok(())
}

/// Demo-only tool. Lives in the example, not in oxidant-tools — real tools
/// land via spec/tools/* entries (see [[flows/add-tool]] once that flow exists).
struct CurrentTime;

#[async_trait]
impl Tool for CurrentTime {
    fn name(&self) -> &str {
        "current_time"
    }
    fn description(&self) -> &str {
        "Return the current UTC time as an ISO-8601 string. Takes no arguments."
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let now = Utc::now().to_rfc3339();
        ToolResult::Ok(json!({ "now_utc": now }))
    }
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
