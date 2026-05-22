---
id: tool-registry
kind: component
parent: overview
order: 4
implements: []
depends_on:
  - contracts/tool
code:
  - crates/oxidant-core/src/registry.rs
status: active
responsibility: |
  Hold the set of available tools, validate model input against each schema before dispatch, and route invocations to the right impl.
---

## Struct

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn register(&mut self, tool: Arc<dyn Tool>);
    pub fn schemas(&self) -> Vec<(String, serde_json::Value)>;       // legacy, for system prompts
    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn Tool>>;      // agent loop uses this
    pub async fn invoke(&self, name: &str, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}
```

`iter()` is what the agent loop walks to build the `tools:` field of each provider request — it needs `name() + description() + schema()` per tool, all from [[contracts/tool]], in one pass without cloning into intermediate tuples.

## Dispatch flow

1. Look up `name`. If absent → `ToolResult::Err("unknown tool: ...")`.
2. Validate `args` against `tool.schema()` using `jsonschema` crate. Invalid → `ToolResult::Err` with the validation report.
3. Check `tool.category()` against `ctx.permission_state`. If not auto-allowed and not allowlisted → emit a permission-prompt request via `ctx.permission_channel` and await user response.
4. Call `tool.invoke(args, ctx).await`. Catch panic → `ToolResult::Err`.
5. Return.

## Schema generation

`ToolRegistry::schemas()` produces the tool list the provider sends to the model. For Anthropic this lands in the `tools:` array; for OpenAI in `tools: [{ type: "function", function: { ... } }]`. The translation lives in `oxidant-providers`, not here.

## Permission integration

The registry does not know whether a category requires a prompt — `ctx.permission_state` carries that. See [[components/config/permissions]].
