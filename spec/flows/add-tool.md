```yaml
---
id: add-tool
kind: flow
parent: overview
order: 4
status: active
responsibility: |
  Spec-first ritual for adding a new model-facing tool: write the spec, validate, implement at the declared code path, register, test, flip status to active — spec and code committed together.
depends_on:
  - contracts/tool
  - components/core/tool-registry
  - components/spec-tools/validate
  - tools/spec/spec-validate
  - tools/edit/apply-edits
  - tools/cargo/cargo-check
  - tools/cargo/cargo-test
---
```

# Add a new model-facing tool

The canonical "extend oxidant" flow. Spec-driven by construction.

## Steps

1. **Spec first.** Create `spec/tools/<area>/<tool-name>.md` with:
   - Frontmatter: `kind: tool`, `parent: components/<owning-component>`, `implements: [contracts/tool]`, `depends_on:` (substrate/components used), `code:` (the future Rust file).
   - Body: one-paragraph purpose, the JSON schema as a code block, response shape, example invocation, "see also" cross-refs.
   - Set `status: draft` until implemented.

2. **Run [[tools/spec/spec-validate]].** Expect warnings only for `missing_code_path` (the Rust file doesn't exist yet) and possibly `unresolved_ref` if any forward refs. Other warnings are real problems — fix them.

3. **Choose owning component.** Most tools live under an existing component (`oxidant-tools`, `oxidant-rust-tools`, `oxidant-spec-tools`, `oxidant-vcs`). New cross-cutting tools may warrant a new component spec; if so, create that under `spec/components/<area>.md` before continuing.

4. **Implement.** Write the Rust file at the `code:` path. Implement the `Tool` trait per [[contracts/tool]]. Register the tool with the registry at startup ([[components/core/tool-registry]]).

5. **`cargo check`.** Resolve any diagnostics. If a contract signature drifted, follow [[flows/fix-diagnostic]] which loops back through the spec.

6. **Test.** Write a unit test that invokes the tool through the registry (not directly) — exercises schema validation as well. Run [[tools/cargo/cargo-test]] and resolve failures.

7. **Update parent component.** If the owning component spec doesn't already reference the new tool in body or `[[refs]]`, edit it to add the link. Re-run [[tools/spec/spec-validate]] for orphan + inbound-edge cleanliness.

8. **Flip status.** Update the tool spec's frontmatter `status: active`. Commit the spec edit and the code together via [[tools/vcs/vcs-commit]] — one PR, two coordinated changes per [[decisions/0008-spec-is-canonical]].

## Invariants

- The new tool must not violate [[invariants/explorations-are-isolated]] — `ToolContext::workspace_root` is the only safe scope.
- If it mutates the filesystem, it must produce a `WorkspaceEdit` and route through [[components/tools/workspace-edit-substrate]], not write directly.
