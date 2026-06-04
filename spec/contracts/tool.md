```yaml
---
id: tool
kind: contract
parent: overview
order: 1
status: active
responsibility: |
  The trait every agent-callable capability implements: a stable name, a JSON schema, a permission category, and an async invoke method.
depends_on: []
code:
  - crates/oxidant-core/src/registry.rs
---
```

`Tool` is the uniform interface every model-facing capability presents to the agent loop. The agent loop never knows which concrete tool it's dispatching to; it has a `dyn Tool` from the registry and calls `invoke`. The trait deliberately stays minimal — the schema captures everything the model needs to know, the category captures everything the permission layer needs to know, and `invoke` does the work.

## Trait

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str { "" }
    fn schema(&self) -> serde_json::Value;
    fn category(&self) -> ToolCategory;
    async fn invoke(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

pub enum ToolCategory {
    ReadOnly,    // auto-approved
    Mutating,    // prompts unless allowlisted
    Network,     // prompts unless allowlisted
}

pub enum ToolResult {
    Ok(serde_json::Value),
    Err(String),
}
```

`description()` is the short text sent to the model alongside the schema in every chat request — what the tool is for, when to call it. Defaults to empty so trivial tools don't have to override; real tools always should. See [[components/providers/openai]] for how it lands in the wire payload.

`ToolContext` is the registry's concern — see [[components/core/tool-registry]].

## Methods

| Method | Returns | Contract |
|---|---|---|
| `name` | `&str` | Globally unique within a `ToolRegistry`. Lowercase snake-case. Stable across versions — renames are breaking changes. |
| `schema` | JSON Schema (Draft 2020-12) | Describes input shape. The agent loop sends this to the model. Must be deterministic and cheap (constant or lazy-init). |
| `category` | `ToolCategory` | One of `ReadOnly`, `Mutating`, `Network`. Drives permission prompts (see [[components/config/permissions]]): `ReadOnly` auto-approves, others prompt unless allowlisted. |
| `invoke` | `ToolResult` | Async. Takes JSON args validated against `schema()` by the registry before dispatch. `ctx` carries workspace root, exploration id, and permission state. Must not panic; errors return as `ToolResult::Err`. |

## Invariants

- Schema validation is the registry's job, not the tool's. Tools may assume `args` already matches `schema()`.
- Tools are pure modulo `ToolContext` — same `(args, ctx)` produces the same `ToolResult` on a quiescent filesystem. See [[invariants/explorations-are-isolated]].
- Tools must respect `ctx.workspace_root` — no escape to other explorations' worktrees.

## UiBridge

For the rare tool that genuinely needs to defer to the user mid-iteration — currently only [[tools/ask-user]] — `ToolContext` carries an optional `ui: Option<Arc<dyn UiBridge>>`. The trait is one method:

```rust
#[async_trait]
pub trait UiBridge: Send + Sync {
    async fn ask_user(
        &self,
        question: String,
        options: Vec<String>,
        allow_freeform: bool,
    ) -> anyhow::Result<String>;
}
```

The host provides the implementation: the GUI's `GuiBridge` posts the question into per-window state and `await`s a `tokio::sync::oneshot::Receiver` fulfilled by the user clicking Submit; CLI / headless contexts pass `ui: None`. Tools requiring user input MUST return `ToolResult::Err` with a descriptive message when `ctx.ui.is_none()` rather than block, retry, or hallucinate an answer.

A tool may `.await` on the bridge without harming concurrency — the agent loop already `tokio::spawn`s every tool invocation onto its own task. Concurrent native tools continue while one tool is parked on a user response.

## Implementors

- Generic: [[components/tools/fs]], [[components/tools/bash-runner]], [[components/tools/edit]]
- Rust: [[components/rust-tools/lsp]], [[components/rust-tools/cargo-runner]], [[components/rust-tools/syn-tools]]
- Spec: [[components/spec-tools/validate]], [[components/spec-tools/diff]]
- VCS: [[components/vcs/worktree-mgmt]]

## Why a single trait

Alternative considered: separate `ReadTool` / `MutatingTool` traits enforcing the category at type level. Rejected — the registry holds `Vec<Box<dyn Tool>>` and benefits from monomorphic dispatch; the runtime `category()` check is one branch in the permission layer and not on a hot path. See [[decisions/0007-roll-own-llm-provider-layer]] for the same reasoning applied to providers.
