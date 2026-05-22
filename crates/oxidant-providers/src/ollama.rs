// Realises spec/components/providers/ollama.md.
//
// Thin wrapper around OpenAIProvider with conservative defaults for local
// OpenAI-compatible servers. Despite the spec name, this also serves
// llama.cpp and LM Studio — same wire format, different defaults.

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::openai::{OpenAIConfig, OpenAIProvider};
use crate::provider::{ChatEvent, ChatRequest, Provider, ProviderCapabilities};

#[derive(Debug, Clone)]
pub struct OllamaConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    /// Reported as `capabilities().name`. Defaults to "ollama".
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

impl OllamaConfig {
    /// Ollama defaults: `http://localhost:11434/v1`, no auth, name "ollama".
    pub fn ollama() -> Self {
        Self {
            base_url: "http://localhost:11434/v1".to_string(),
            api_key: None,
            name: "ollama".to_string(),
            capabilities: local_capabilities(8192),
        }
    }

    /// LM Studio defaults: `http://localhost:1234/v1`, no auth, name "lmstudio".
    pub fn lmstudio() -> Self {
        Self {
            base_url: "http://localhost:1234/v1".to_string(),
            api_key: None,
            name: "lmstudio".to_string(),
            capabilities: local_capabilities(32_768),
        }
    }

    /// llama.cpp `server` defaults: `http://localhost:8080/v1`, no auth.
    pub fn llamacpp() -> Self {
        Self {
            base_url: "http://localhost:8080/v1".to_string(),
            api_key: None,
            name: "llamacpp".to_string(),
            capabilities: local_capabilities(8192),
        }
    }

    /// text-generation-webui (oobabooga) defaults: `http://localhost:5000/v1`,
    /// no auth. The OpenAI-compatible API extension must be enabled in the UI.
    pub fn textgen() -> Self {
        Self {
            base_url: "http://localhost:5000/v1".to_string(),
            api_key: None,
            name: "textgen".to_string(),
            capabilities: local_capabilities(32_768),
        }
    }

    /// Configure the base URL freely; name defaults to "local-openai".
    pub fn custom(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            name: "local-openai".to_string(),
            capabilities: local_capabilities(8192),
        }
    }
}

fn local_capabilities(max_context_tokens: usize) -> ProviderCapabilities {
    ProviderCapabilities {
        // Default true; per the spec this is meant to be model-dependent via
        // /api/show probing. Until that lands, we send tools and let
        // tool-capable models (Qwen2.5-Coder, Llama 3.1, Hermes-3, etc.) work
        // and reasoning models (DeepSeek-R1) silently no-op.
        tool_use: true,
        prompt_cache: false,
        extended_thinking: false,
        vision: false,
        max_context_tokens,
    }
}

pub struct OllamaProvider {
    inner: OpenAIProvider,
    name: String,
    capabilities: ProviderCapabilities,
}

impl OllamaProvider {
    pub fn new(config: OllamaConfig) -> Self {
        let name = config.name.clone();
        let capabilities = config.capabilities;
        let inner = OpenAIProvider::new(OpenAIConfig {
            base_url: config.base_url,
            api_key: config.api_key,
            name: config.name,
            capabilities: config.capabilities,
        });
        Self { inner, name, capabilities }
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn chat(&self, req: ChatRequest) -> anyhow::Result<BoxStream<'static, ChatEvent>> {
        self.inner.chat(req).await
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.capabilities
    }

    fn name(&self) -> &str {
        &self.name
    }
}
