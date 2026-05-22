// Live smoke test against a local OpenAI-compatible server.
//
// Marked #[ignore] so cargo test doesn't require LM Studio / Ollama running.
// Run manually with:
//   cargo test -p oxidant-providers --test local_smoke -- --ignored --nocapture
//
// Override defaults via env vars:
//   OXIDANT_LOCAL_BASE_URL  default http://localhost:1234/v1 (LM Studio)
//   OXIDANT_LOCAL_MODEL     default: first model returned by GET /v1/models

use futures::StreamExt;
use oxidant_providers::{
    ChatEvent, ChatRequest, ContentPart, OllamaConfig, OllamaProvider, Provider, RequestMessage,
    Role,
};

#[tokio::test]
#[ignore = "requires a running local OpenAI-compatible server"]
async fn streams_a_short_completion() {
    let base_url = std::env::var("OXIDANT_LOCAL_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:1234/v1".to_string());

    let config = OllamaConfig {
        base_url: base_url.clone(),
        ..OllamaConfig::lmstudio()
    };

    let model = std::env::var("OXIDANT_LOCAL_MODEL")
        .ok()
        .unwrap_or_else(|| {
            // Block on a quick /v1/models lookup. tokio::test wraps us in
            // a runtime; reuse it via Handle.
            let url = format!("{}/models", base_url.trim_end_matches('/'));
            let body = reqwest::blocking::get(&url)
                .expect("local server reachable")
                .text()
                .expect("model list response");
            let parsed: serde_json::Value =
                serde_json::from_str(&body).expect("parse /v1/models JSON");
            parsed["data"][0]["id"]
                .as_str()
                .expect("at least one model loaded")
                .to_string()
        });

    eprintln!("smoke: endpoint={base_url} model={model}");

    let provider = OllamaProvider::new(config);
    let req = ChatRequest {
        model,
        system: None,
        messages: vec![RequestMessage {
            role: Role::User,
            content: vec![ContentPart::Text(
                "Reply with exactly the word: pong".to_string(),
            )],
        }],
        tools: vec![],
        max_tokens: 32,
        temperature: Some(0.0),
        thinking: None,
    };

    let mut stream = provider.chat(req).await.expect("chat() initial call");
    let mut text = String::new();
    let mut saw_finish = false;
    while let Some(ev) = stream.next().await {
        match ev {
            ChatEvent::TextDelta(s) => text.push_str(&s),
            ChatEvent::Finish { stop_reason, usage } => {
                eprintln!(
                    "smoke: finish={stop_reason:?} in={} out={}",
                    usage.input_tokens, usage.output_tokens
                );
                saw_finish = true;
            }
            ChatEvent::Error(e) => panic!("provider error: {e}"),
            _ => {}
        }
    }
    eprintln!("smoke: text={text:?}");
    assert!(saw_finish, "stream ended without Finish event");
    assert!(!text.is_empty(), "no TextDelta events received");
}
