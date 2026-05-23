// Quick streaming chat against any local OpenAI-compatible server.
//
// textgen-webui default: cargo run -p oxidant-providers --example local_chat -- "hello"
// LM Studio:             cargo run -p oxidant-providers --example local_chat -- --preset lmstudio "hello"
// Ollama:                cargo run -p oxidant-providers --example local_chat -- --preset ollama "hello"
// llama.cpp server:      cargo run -p oxidant-providers --example local_chat -- --preset llamacpp "hello"
// Pinned model:          cargo run -p oxidant-providers --example local_chat -- --model qwen2.5-coder-32b-instruct "hello"
// Custom endpoint:       cargo run -p oxidant-providers --example local_chat -- --base-url http://localhost:9999/v1 "hello"
//
// If --model is omitted, the example queries /v1/models and uses the first one
// loaded — so it "just works" against whatever LM Studio currently has open.

use std::io::Write;

use clap::Parser;
use futures::StreamExt;
use oxidant_providers::{
    ChatEvent, ChatRequest, ContentPart, OllamaConfig, OllamaProvider, Provider, RequestMessage,
    Role,
};

#[derive(Parser, Debug)]
#[command(about = "Stream a chat completion from a local OpenAI-compatible server")]
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

    /// Max tokens to generate
    #[arg(long, default_value_t = 1024)]
    max_tokens: u32,

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
        "Say hello in exactly one sentence.".to_string()
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
    let req = ChatRequest {
        model,
        system: args.system,
        messages: vec![RequestMessage {
            role: Role::User,
            content: vec![ContentPart::Text(prompt)],
        }],
        tools: vec![],
        max_tokens: args.max_tokens,
        temperature: None,
        thinking: None,
    };

    let mut stream = provider.chat(req).await?;
    let mut stdout = std::io::stdout().lock();
    while let Some(event) = stream.next().await {
        match event {
            ChatEvent::TextDelta(s) => {
                write!(stdout, "{s}")?;
                stdout.flush()?;
            }
            ChatEvent::ThinkingDelta(s) => {
                write!(stdout, "[thinking] {s}")?;
                stdout.flush()?;
            }
            ChatEvent::ToolUseStart { id, name } => {
                writeln!(stdout, "\n[tool_use_start id={id} name={name}]")?;
            }
            ChatEvent::ToolUseInputDelta { id, json_delta } => {
                write!(stdout, "[tool_input id={id}] {json_delta}")?;
                stdout.flush()?;
            }
            ChatEvent::ToolUseEnd { id } => {
                writeln!(stdout, "\n[tool_use_end id={id}]")?;
            }
            ChatEvent::Finish { stop_reason, usage } => {
                writeln!(
                    stdout,
                    "\n\n--- finish: {stop_reason:?} | in={} out={} ---",
                    usage.input_tokens, usage.output_tokens
                )?;
            }
            ChatEvent::Error(e) => {
                writeln!(stdout, "\n[error] {e}")?;
            }
        }
    }
    Ok(())
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
    let models = parsed
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow::anyhow!("/v1/models response missing `data` array: {body}"))?;
    let first = models
        .first()
        .and_then(|m| m.get("id"))
        .and_then(|id| id.as_str())
        .ok_or_else(|| anyhow::anyhow!("/v1/models returned an empty model list"))?;
    Ok(first.to_string())
}
