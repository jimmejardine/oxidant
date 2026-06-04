```yaml
id: fix-diagnostic
kind: flow
parent: overview
order: 1
status: active
responsibility: |
  End-to-end loop demonstrating how Rust tooling, the spec graph, and the edit substrate compose to fix a compiler diagnostic.
depends_on:
  - tools/cargo/cargo-check
  - tools/lsp/rust-hover
  - tools/lsp/rust-find-references
  - tools/edit/apply-edits
  - tools/spec/spec-for-file
  - tools/spec/spec-read
  - tools/spec/spec-diff
  - tools/cargo/cargo-test
  - tools/vcs/vcs-commit
  - components/tools/workspace-edit-substrate
```

# Fix a Rust compiler diagnostic

The canonical end-to-end loop demonstrating how Rust tooling, the spec graph, and the edit substrate compose. This is the bar for "the agent did the right thing" when handed a build failure.

## Trigger

The user reports a build failure, or `cargo_check` is auto-run after some prior edit and surfaces diagnostics.

## Steps

1. **Diagnostics.** Call [[tools/cargo/cargo-check]] (or read its cached result). Get back `CompilerMessage[]` with structured `level`, `code`, `spans`, optional `suggestion.replacement`. No text scraping.

2. **Locate the affected component.** For each diagnostic span's `file`, call [[tools/spec/spec-for-file]] to find which `components/*` spec covers it. Read that spec via [[tools/spec/spec-read]] to recover its declared `responsibility` and `depends_on`.

3. **Choose a fix.** Two paths:
   - **Compiler suggests.** If the diagnostic carries a `suggestion.replacement`, treat the span+replacement as a ready-made edit; skip to step 5.
   - **No suggestion.** Inspect the span. Use [[tools/lsp/rust-hover]] on the failing expression to recover the inferred type and definition. Use [[tools/lsp/rust-find-references]] if the fix needs to touch callers. Compose a `WorkspaceEdit`.

4. **Sanity-check against spec.** Compare the proposed edit to the component spec's `responsibility` and the file's owning [[contracts/tool]] / [[contracts/provider]] / [[contracts/workspace-edit]] contracts. If the fix breaks a contract method's signature, the spec must change too — surface that as a planned spec edit before code.

5. **Apply.** Submit the WorkspaceEdit via [[tools/edit/apply-edits]]. The substrate ([[components/tools/workspace-edit-substrate]]) handles atomicity, optimistic-concurrency, and post-edit syn parse. If syn rejects the result, the substrate rolls back; loop back to step 3.

6. **Verify.** Re-run [[tools/cargo/cargo-check]]. Expect zero errors. If new errors appeared, treat them as the new trigger and loop.

7. **Drift check.** Run [[tools/spec/spec-diff]] on any contract whose owning code was touched. If drift is reported, edit the spec and loop step 5–6.

8. **Test.** Run [[tools/cargo/cargo-test]] for the affected package. Failures route back to step 1 with the failing test as the new diagnostic source.

9. **Commit.** Stage the spec + code changes together via [[tools/vcs/vcs-commit]] — never spec without code, never code without spec when the spec graph was touched.

## Invariants preserved

- [[invariants/edits-are-atomic]] — step 5 either applies the whole WorkspaceEdit or none of it
- [[invariants/rust-files-parse-after-edit]] — step 5 rolls back on syn failure
- Spec-and-code move together (see [[decisions/0008-spec-is-canonical]])

## Common failure modes

- **Cycle of diagnostics.** Each fix introduces a new error. Mitigation: bound retries to 3 before asking the user.
- **Contract drift caught mid-fix.** A signature change that satisfies the compiler breaks the contract spec. Update the contract spec first, re-think the fix.
- **Edit succeeds but tests fail.** Treated as a new diagnostic (step 8), not as a rollback; the code compiles, the design is wrong.
