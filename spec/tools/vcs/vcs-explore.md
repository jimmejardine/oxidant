```yaml
---
id: vcs-explore
kind: tool
parent: components/vcs/worktree-mgmt
order: 8
implements:
  - contracts/tool
depends_on:
  - components/vcs/worktree-mgmt
code:
  - crates/oxidant-vcs/src/tools/vcs_explore.rs
status: active
responsibility: |
  Spawn a new sub-exploration with its own worktree + branch + conversation. GUI-only — agent cannot spawn explorations to prevent uncontrolled chat birth.
---
```

`category`: `Mutating`. **GUI-only**: the registry refuses to dispatch this tool when the caller is the agent loop. Only the human (via the GUI button or context menu) can spawn explorations.

## Schema

```json
{
  "type": "object",
  "properties": {
    "base":         { "type": "string", "default": "HEAD", "description": "revspec to branch from" },
    "name":         { "type": "string", "description": "slug; auto-generated if absent" },
    "seed_prompt":  { "type": "string", "description": "first user message in the new exploration" }
  }
}
```

## Result

```json
{
  "exploration_id": "01J0...",
  "worktree":       "/repo/../.oxidant-worktrees/repo/explore-lsp-cache",
  "branch":         "oxidant/explore/lsp-cache-3f2"
}
```

## Why agent-gated

If the agent could spawn explorations from a tool call, one chat could birth many others uncontrollably — fork-bombing the user's machine and confusing the conversation graph. The user is always the decision-maker. See [[flows/spawn-exploration]] for the full flow.
