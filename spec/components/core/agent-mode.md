---
id: agent-mode
kind: component
parent: overview
order: 7
implements: []
depends_on:
  
code:
  - crates/oxidant-core/src/agent_loop.rs
status: active
responsibility: |
  Define the two interaction modes the chat agent runs in — Plan and Implement — and the enforcement that hard-caps the tools and steering each mode sees. Plan is a guardrail mode: the agent investigates with read-only tools and then *describes* what it would do. Implement is the normal mode with full tool access. The toggle is a keyboard binding in the chat input ([[components/gui/chat-input-panel]]).
---

## The two modes

```rust
pub enum AgentMode {
    Plan,        // default
    Implement,
}
```

**Plan mode** (default at panel construction):
- The agent sees only `ToolCategory::ReadOnly` tools in the `ChatRequest.tools` list — `Mutating` and `Network` tools are filtered out before the request leaves the loop. The model can't pick a tool it can't see.
- The system prompt gains a Plan-specific suffix (verbatim text below) that tells the model its job is to describe, not act.
- Text-extracted tool calls ([[components/core/text-tool-call-extraction]]) for filtered-out tools fall through to the existing unknown-tool error path. No silent ignore.

**Implement mode**:
- The full registry is exposed to the model.
- No system-prompt suffix added. The model behaves as it does today.

There is no third "hybrid" mode in v1. The split is deliberately binary so the user can tell at a glance what the agent is allowed to do.

## Default and persistence

Plan is the default each time the chat panel is constructed. The mode is **not persisted** to settings — across restarts the chat boots back to Plan. Rationale: the safer side of an unfamiliar-state question wins, and the toggle is one keypress.

## Plan-mode system prompt suffix

Appended verbatim to whatever `AgentLoopConfig.system_prompt` already carries. Implementations MUST use this exact text so behaviour is stable across providers:

```
You are currently in PLAN MODE.

Use read-only tools (read files, grep, spec lookups, cargo_check, LSP queries, git log/diff/status, etc.) to investigate as much as you need. Then DESCRIBE the change you would make:
- the files you'd touch
- the substantive edits
- the order you'd do them in
- and why

Do NOT attempt to mutate files, git state, or the workspace — those tools are not exposed to you in this mode. If you reach for one, the call will fail. The user will switch you to IMPLEMENT mode when they are ready for you to act.
```

The format of the response itself is up to the model for now. A future revision may pin a structured format (numbered steps, file-touch list, diff-style previews) once the user has seen what models naturally produce.

## Edge cases recorded for completeness

- **Honor-system on conditional tools**: `rust_rename` is categorised `ReadOnly` because by default it only previews; passing `apply=true` makes it mutate. Plan mode keeps it visible. The system prompt explicitly forbids mutation; this is honor-system. A v2 may add per-tool gates on specific args.
- **`cargo_test` is `Mutating`** because it writes to `target/` and runs untrusted code from dependencies. It is hidden in Plan mode. Running tests as part of "thinking about a change" is reasonable but slightly out of scope; the user can flip to Implement, run tests, and flip back.
- **`bash` is `Mutating`** because any bash command could mutate. It is hidden in Plan mode. The user's allowlist via [[components/config/permissions]] does NOT override mode — permissions gate user-trust, mode hard-caps the tool surface.

## Out of scope for v1

- A per-tool override matrix for "looks read-only-ish".
- Auto-switching to Implement after the user types "go ahead" or similar.
- A separate transcript view that visually distinguishes proposed actions from executed ones.
- Mode-aware tool descriptions (e.g. rewriting `fs_write`'s description in Plan mode).
- Persistence in settings.
