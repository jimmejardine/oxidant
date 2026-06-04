```yaml
---
id: tool-permission-check
kind: flow
parent: overview
order: 9
status: active
responsibility: |
  How the permission engine gates a tool call: decision matrix (denylist → trust → allowlist → category default) and the contract for what Allow / Deny / Prompt mean to the tool registry.
depends_on:
  - components/config/permissions
  - components/core/tool-registry
  - contracts/tool
  - decisions/0002-no-built-in-sandbox
---
```

# Decide whether a tool call may proceed

Permissions are oxidant's only line of defence between the model and the host. There is no sandbox ([[decisions/0002-no-built-in-sandbox]]); the engine described here is the whole control surface.

## Trigger

[[components/core/tool-registry]]::invoke is called with `(name, args, ctx)`. Before invoking the tool's `invoke()` method, the registry consults [[components/config/permissions]]::`PermissionEngine::decide(tool, args)`.

## Inputs to the decision

- **Tool's `category()`** (`ReadOnly` / `Mutating` / `Network`) — declared by the tool, never inferred.
- **Tool's `name()`** and, for `bash`, the proposed command string lifted from `args`.
- **PermissionsSettings** loaded at startup or live-edited via [[flows/edit-settings]]:
  - `auto_approve_readonly: bool`
  - `allowlist: Vec<String>` — patterns that pre-approve
  - `denylist: Vec<String>` — patterns that force Deny
- **Runtime `PermissionState`** — currently just a `trust_mode: bool` (off by default).

## Decision matrix (top-down; first match wins)

1. **Denylist.** Any pattern in `denylist` that matches the tool call → `Deny`. Wins over everything below, including `trust_mode`. Defaults include `bash:rm -rf*` and `bash:rm -fr*`.

2. **Trust mode.** If `trust_mode` is on, return `Allow` regardless of category, allowlist, or `auto_approve_readonly`. Intended for the user explicitly typing "you can do anything for the next few turns" — denylist still applies as a safety net.

3. **Allowlist.** Any pattern in `allowlist` that matches → `Allow`. Pattern forms:
   - **Exact tool name**: `"fs_write"` matches the `fs_write` tool.
   - **bash glob**: `"bash:cargo *"` matches any `bash` invocation whose command starts with `cargo `.
   - **bash regex**: `"bash:/^git /"` matches any `bash` command matching the regex.
   The defaults include `bash:ls *`, `bash:pwd`, `bash:cat *`, `bash:cargo check*`, `bash:cargo test*` — common reads that don't warrant a prompt every time.

4. **Category default.**
   - `ReadOnly` + `auto_approve_readonly == true` → `Allow`.
   - `ReadOnly` + `auto_approve_readonly == false` → `Prompt`.
   - `Mutating` → `Prompt`.
   - `Network` → `Prompt`.

## What the registry does with the verdict

- **`Allow`** → call `tool.invoke(args, ctx).await`, return the `ToolResult` as-is.
- **`Deny`** → return `ToolResult::Err("permission denied: <reason>")` without invoking. Counts as a tool call for the agent loop's iteration accounting and the post-edit hook's `any_mutating` flag if the category was Mutating (the model attempted a mutation — the post-edit drift check still makes sense).
- **`Prompt`** → in the GUI: surface a modal asking the user to Allow once / Allow always (adds to allowlist) / Deny. The agent task awaits the user's choice via a oneshot channel. CLI / headless contexts have no prompt surface yet; MVP behaviour is to treat `Prompt` as `Deny` and return the error so the model isn't blocked indefinitely.

## Worked examples

| Tool call | Category | Setting | Verdict |
|---|---|---|---|
| `fs_read` of a file under workspace | ReadOnly | `auto_approve_readonly=true` (default) | Allow |
| `bash` running `ls -la` | Mutating | matches `bash:ls *` in default allowlist | Allow |
| `bash` running `rm -rf target/` | Mutating | matches `bash:rm -rf*` in default denylist | Deny |
| `apply_edits` to a file under workspace | Mutating | no allowlist match, `trust_mode=false` | Prompt |
| Same `apply_edits` after the user toggled trust mode | Mutating | `trust_mode=true`, no denylist match | Allow |
| `bash` running `cargo test --all` | Mutating | matches `bash:cargo test*` | Allow |

## Invariants

- Denylist always wins. There is no setting that lets `trust_mode` or `allowlist` override denylist; that's a deliberate safety net so a user can lock out `rm -rf` even while granting otherwise-broad trust.
- The engine never mutates state. Adding a pattern to the allowlist via "Allow always" is a settings edit ([[flows/edit-settings]]), not an engine side-effect — there's one source of truth for what's allowed.
- `Prompt` requires a UI in the loop. Headless contexts must either pre-allowlist what the agent will do or accept that every Prompt becomes a Deny. This is the friction that drives users to allowlist patterns explicitly rather than relying on prompts.

## See also

- [[components/config/permissions]] — implementation of the engine
- [[flows/edit-settings]] — how the allowlist / denylist are mutated
- [[decisions/0002-no-built-in-sandbox]] — why permissions are the only line of defence
