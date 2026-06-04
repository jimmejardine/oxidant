// Realises spec/tools/ask-user.md.
//
// `ask_user` — let the agent defer a branching decision to the user.
// Posts a multiple-choice question through the `UiBridge` on
// `ToolContext` and awaits the answer (a chosen option, or the
// free-form fallback if the user typed one). If the bridge is absent
// (CLI / headless), the tool fails fast with a descriptive error so
// the model doesn't loop pretending it can interact.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};

pub struct AskUser;

#[derive(Deserialize)]
struct Args {
    question: String,
    options: Vec<String>,
    /// Defaults to `true` — most uses want the safety net of "or type
    /// your own answer". Set false when the question genuinely admits
    /// only the listed options (e.g. yes/no).
    #[serde(default = "default_allow_freeform")]
    allow_freeform: bool,
}

fn default_allow_freeform() -> bool {
    true
}

#[async_trait]
impl Tool for AskUser {
    fn name(&self) -> &str {
        "ask_user"
    }
    fn description(&self) -> &str {
        "Ask the user a multiple-choice question with an optional free-form fallback. Call when the considered branches depend on user preference and you cannot reasonably pick on their behalf — e.g. a design choice with no objectively-better option, a name the user must decide, a tradeoff only they can evaluate. Don't use for trivia the user shouldn't have to answer, and don't use to ask permission for a tool call you're already authorised to make. Returns { answer: \"…\" } with the chosen option's text verbatim or the user's typed free-form text. Fails when no interactive UI is hosting the loop."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "minLength": 1,
                    "description": "The question to ask. Be specific; the user sees this in a modal with no other context."
                },
                "options": {
                    "type": "array",
                    "items": { "type": "string", "minLength": 1 },
                    "minItems": 1,
                    "description": "The pre-canned answers. Each renders as a clickable button; clicking returns that string verbatim."
                },
                "allow_freeform": {
                    "type": "boolean",
                    "default": true,
                    "description": "When true (default), a single-line text field is rendered below the buttons; submitting it returns the typed text. Set false to force a choice among the listed options."
                }
            },
            "required": ["question", "options"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let parsed: Args = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        if parsed.question.trim().is_empty() {
            return ToolResult::Err("`question` must not be empty".into());
        }
        if parsed.options.is_empty() {
            return ToolResult::Err("`options` must contain at least one entry".into());
        }
        let Some(ui) = ctx.ui.as_ref() else {
            return ToolResult::Err(
                "ask_user requires an interactive UI host; this context (headless / CLI) cannot answer questions".into(),
            );
        };
        match ui
            .ask_user(parsed.question, parsed.options, parsed.allow_freeform)
            .await
        {
            Ok(answer) => ToolResult::Ok(json!({ "answer": answer })),
            Err(e) => ToolResult::Err(format!("ask_user failed: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxidant_core::UiBridge;
    use tokio_util::sync::CancellationToken;

    use super::*;

    /// Mock bridge that returns a pre-set answer regardless of input.
    struct StaticBridge {
        answer: String,
    }

    #[async_trait]
    impl UiBridge for StaticBridge {
        async fn ask_user(
            &self,
            _question: String,
            _options: Vec<String>,
            _allow_freeform: bool,
        ) -> anyhow::Result<String> {
            Ok(self.answer.clone())
        }
    }

    /// Mock bridge that always errors — simulating cancellation.
    struct ErrorBridge;

    #[async_trait]
    impl UiBridge for ErrorBridge {
        async fn ask_user(
            &self,
            _question: String,
            _options: Vec<String>,
            _allow_freeform: bool,
        ) -> anyhow::Result<String> {
            Err(anyhow::anyhow!("user cancelled"))
        }
    }

    fn ctx_with(bridge: Option<Arc<dyn UiBridge>>) -> ToolContext {
        ToolContext {
            workspace_root: camino::Utf8PathBuf::from("."),
            exploration_id: "ask-user-test".into(),
            cancellation: CancellationToken::new(),
            ui: bridge,
        }
    }

    #[tokio::test]
    async fn returns_answer_from_bridge() {
        let bridge: Arc<dyn UiBridge> = Arc::new(StaticBridge {
            answer: "token bucket".into(),
        });
        let result = AskUser
            .invoke(
                json!({
                    "question": "which rate limiter?",
                    "options": ["token bucket", "leaky bucket"]
                }),
                &ctx_with(Some(bridge)),
            )
            .await;
        match result {
            ToolResult::Ok(v) => assert_eq!(v, json!({ "answer": "token bucket" })),
            ToolResult::Err(e) => panic!("expected Ok, got Err: {e}"),
        }
    }

    #[tokio::test]
    async fn errors_without_ui_host() {
        let result = AskUser
            .invoke(
                json!({
                    "question": "q?",
                    "options": ["a", "b"]
                }),
                &ctx_with(None),
            )
            .await;
        match result {
            ToolResult::Err(msg) => {
                assert!(msg.contains("requires an interactive UI host"), "{msg}")
            }
            ToolResult::Ok(v) => panic!("expected Err, got Ok({v})"),
        }
    }

    #[tokio::test]
    async fn surfaces_bridge_error() {
        let bridge: Arc<dyn UiBridge> = Arc::new(ErrorBridge);
        let result = AskUser
            .invoke(
                json!({
                    "question": "q?",
                    "options": ["a"]
                }),
                &ctx_with(Some(bridge)),
            )
            .await;
        match result {
            ToolResult::Err(msg) => assert!(msg.contains("user cancelled"), "{msg}"),
            ToolResult::Ok(v) => panic!("expected Err, got Ok({v})"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_question() {
        let bridge: Arc<dyn UiBridge> = Arc::new(StaticBridge { answer: "x".into() });
        let result = AskUser
            .invoke(
                json!({ "question": "   ", "options": ["a"] }),
                &ctx_with(Some(bridge)),
            )
            .await;
        match result {
            ToolResult::Err(msg) => assert!(msg.contains("question"), "{msg}"),
            ToolResult::Ok(v) => panic!("expected Err, got Ok({v})"),
        }
    }

    #[tokio::test]
    async fn rejects_empty_options() {
        let bridge: Arc<dyn UiBridge> = Arc::new(StaticBridge { answer: "x".into() });
        let result = AskUser
            .invoke(
                json!({ "question": "q?", "options": [] }),
                &ctx_with(Some(bridge)),
            )
            .await;
        match result {
            ToolResult::Err(msg) => assert!(msg.contains("options"), "{msg}"),
            ToolResult::Ok(v) => panic!("expected Err, got Ok({v})"),
        }
    }

    #[test]
    fn schema_required_fields() {
        let s = AskUser.schema();
        let required = s["required"].as_array().unwrap();
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"question"));
        assert!(names.contains(&"options"));
    }
}
