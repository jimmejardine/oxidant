```yaml
---
id: permissions
kind: component
parent: overview
order: 2
implements: []
depends_on:
  - components/config/settings
  - contracts/tool
code:
  - crates/oxidant-config/src/permissions.rs
status: active
responsibility: |
  Decide whether each tool call should auto-approve, auto-deny, or prompt the user; mirror Claude Code's allowlist UX.
---
```

The single layer of safety between the agent and the user's filesystem. **Not** a sandbox — see [[decisions/0002-no-built-in-sandbox]].

## Decision matrix

```
┌──────────────┬───────────────────────────────────────┐
│ category     │ default behaviour                     │
├──────────────┼───────────────────────────────────────┤
│ ReadOnly     │ auto-approve                          │
│ Mutating     │ prompt (unless allowlisted)           │
│ Network      │ prompt (unless allowlisted)           │
└──────────────┴───────────────────────────────────────┘
```

User can switch a session into "trust mode" temporarily — every tool auto-approves. Surfaced as a banner in the GUI so it's visible.

## Allowlist / denylist matching

For bash specifically, patterns match against the command line:
- `cargo check*` → allow
- `rm *` → always prompt (deny if denylist)
- Substrings, glob, or full regex (`/.../`) supported.

For other tools, the allowlist is keyed by tool name:
- `apply_edits` → allow
- `fs_write` → prompt

## Prompt UX

A modal dialog in the exploration's window:
```
┌─────────────────────────────────────────────┐
│ Allow tool call?                            │
│                                             │
│ Tool:  fs_write                             │
│ File:  crates/oxidant-core/src/main.rs      │
│                                             │
│ [Allow once] [Allow for session] [Deny]     │
│ [☐ Add 'fs_write' to allowlist]             │
└─────────────────────────────────────────────┘
```

## "Add to allowlist" updates per-repo settings

Checking the box updates `<worktree>/.oxidant/oxidant.toml`. Per-user version available in the user's config dialog.
