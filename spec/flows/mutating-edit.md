```yaml
id: mutating-edit
kind: flow
parent: overview
order: 6
status: active
responsibility: |
  Anatomy of a Mutating tool call: permission gate, atomic apply through the workspace-edit substrate, syn-validation for .rs files, and the spec_diff post-edit hook that closes the spec-driven design loop.
depends_on:
  - components/core/agent-loop
  - components/core/tool-registry
  - components/tools/workspace-edit-substrate
  - contracts/workspace-edit
  - contracts/tool
  - components/config/permissions
  - tools/spec/spec-diff
  - tools/edit/apply-edits
  - tools/edit/edit-string
  - invariants/edits-are-atomic
  - invariants/rust-files-parse-after-edit
```

# Apply a Mutating tool call and check for drift

The "the agent changed code" path. Distinguished from a ReadOnly call by three extra responsibilities: permission gating, atomic apply with rollback, and the post-edit hook that flags spec drift before the next iteration.

## Trigger

Step 6 of [[flows/conversation-turn]]: the model emitted a `ToolUseStart` for a tool whose `category()` is `Mutating` (or `Network`). The registry dispatches it like any other tool call; this flow is what happens inside that dispatch and what the agent loop does after.

## Steps

1. **Permission gate.** [[components/core/tool-registry]] consults the [[components/config/permissions]] engine — see [[flows/tool-permission-check]] for the decision matrix. Outcomes:
   - `Allow` → continue to step 2.
   - `Deny` → return `ToolResult::Err("denied: …")`, skip steps 2–4, still count as a Mutating call for step 5.
   - `Prompt` → GUI surfaces a prompt (future); MVP behaviour falls back to Deny when no UI is present.

2. **Tool::invoke runs.** The tool builds a `WorkspaceEdit` per [[contracts/workspace-edit]] and either:
   - **Direct-apply tools** ([[tools/edit/edit-string]], [[tools/edit/apply-edits]]) submit the edit to [[components/tools/workspace-edit-substrate]]::apply themselves and return a structured `ApplyResult`.
   - **Smart tools** ([[tools/lsp/rust-rename]], [[tools/syn/syn-add-use]], [[tools/syn/syn-add-derive]], clippy-fix flows) construct a `WorkspaceEdit` and either route it through the same substrate (when `apply=true`) or return it as a preview for the agent to apply explicitly (when `apply=false`).

3. **Substrate apply.** For each file in the edit:
   - Normalise LSP-style ranges to byte ranges, sort, check overlaps.
   - Optional `expected_text` check on original bytes — concurrency guard.
   - For `.rs` files: build the new content in memory, run `syn::parse_file`. If it fails → `ApplyError::SynParseFailed`, no file touched.
   - Write to a temp file, fsync, atomic rename into place.
   - If any rename fails partway through, restore the backups for already-renamed files. Either every file in the edit applies, or none does ([[invariants/edits-are-atomic]]).

4. **Result back to the loop.** The tool returns `ToolResult::Ok(value)` or `Err(message)`. The agent loop pushes this as a User message with `ToolResultContent::Json` or `Text(is_error=true)`. The model sees it on the next iteration either way.

5. **Post-edit hook.** After all tool calls in this turn are dispatched, [[components/core/agent-loop]] checks `any_mutating`. If true and `config.post_edit_check_tool` names a registered ReadOnly tool (default: `spec_diff`):
   - Invoke it with empty args.
   - Format the result into a synthetic User message (`"# Post-edit check: spec_diff\n\n…"`).
   - Append to the conversation — the model sees the drift report on the next iteration and either fixes it or explains why it's acceptable.
   - Increment `outcome.post_edit_checks_fired`.

6. **Loop continues.** Control returns to step 3 of [[flows/conversation-turn]] — next iteration, fresh provider call, model now has the tool result + any post-edit hook output in context.

## Invariants preserved

- [[invariants/edits-are-atomic]] — substrate either applies the whole `WorkspaceEdit` or none of it.
- [[invariants/rust-files-parse-after-edit]] — syn-parse gate prevents committing broken Rust.
- [[invariants/explorations-are-isolated]] — substrate refuses paths that escape `workspace_root`.

## Common failure modes

- **Edit refused by syn.** The model proposed bytes that don't parse. Rolled back; tool returns `Err`, the loop sees that error and typically retries with a smaller / corrected edit.
- **`expected_text` mismatch.** The file changed since the model read it (another tool call, external edit). Rolled back; the model should re-read and rebuild the edit.
- **Post-edit hook reports drift.** Not a failure — the working state. The agent is expected to either edit the spec to match or revise the code on the next iteration. If neither happens, drift persists and `spec_diff --strict` in CI ([[flows/spec-ci-gate]]) will block the merge.

## See also

- [[components/tools/workspace-edit-substrate]] — the apply path
- [[tools/spec/spec-diff]] — the post-edit hook's payload
- [[decisions/0008-spec-is-canonical]] — why drift detection is wired into the loop
