```yaml
id: spec-first-edit
kind: flow
parent: overview
order: 7
status: active
responsibility: |
  The ADR-0008 working agreement narrated step by step: for any change that crosses a contract, responsibility, or invariant, the spec edit happens before the code edit. Generalises add-tool to any spec, not just new tools.
depends_on:
  - decisions/0008-spec-is-canonical
  - tools/spec/spec-for-file
  - tools/spec/spec-read
  - tools/spec/spec-resolve-links
  - tools/spec/spec-validate
  - tools/spec/spec-diff
  - tools/edit/apply-edits
  - tools/cargo/cargo-check
  - tools/vcs/vcs-commit
```

# Edit spec first, then code

The standing rule from [[decisions/0008-spec-is-canonical]] expressed as a sequence. [[flows/add-tool]] is a specialisation of this flow for new tools; [[flows/fix-diagnostic]] is a specialisation for bug fixes that uncover spec drift. Use this one when:

- You're about to change a trait signature.
- You're about to change the responsibility of a component.
- You're moving code between files in a way that the `code:` frontmatter list will notice.
- You're adding or removing a `depends_on` edge in practice.

Skip this flow for: pure internal refactors that don't cross a contract, bug fixes inside a function body, dependency bumps, formatting passes. Those don't need a spec edit.

## Trigger

A planned change that you can articulate as "the spec currently says X, after this it'll say Y". If you can't articulate the spec delta, you probably don't need a spec edit — re-evaluate which bucket the change falls into.

## Steps

1. **Locate the affected specs.** Start from the code file(s) you're about to touch and call [[tools/spec/spec-for-file]] — every spec that declares the file in its `code:` frontmatter is in scope. Don't stop there: each of those specs has parents (contracts, invariants) and dependents (tools, GUI panels). [[tools/spec/spec-resolve-links]] gives you inbound + outbound edges in one call.

2. **Read what's there.** [[tools/spec/spec-read]] each candidate. Reconstruct the current declared shape — the responsibility, the trait, the depends_on graph. This is the "before" snapshot you'll diff against.

3. **Draft the spec edit.** Modify frontmatter and body in place. If the change crosses multiple specs, plan the edits as one cohesive batch — they'll commit together in step 8. Common shapes:
   - **Contract change.** Edit the trait declaration block in [[contracts/tool]] / [[contracts/provider]] / [[contracts/workspace-edit]]. Mention the rationale in the body; downstream tools' `implements:` and `depends_on:` may need updating too.
   - **Responsibility shift.** Edit the `responsibility:` frontmatter and the body's "Why this lives here" prose. If responsibility moves between components, update both specs and any `depends_on` between them.
   - **New `code:` path.** Add the path to the spec's `code:` list before creating the file; the validator's `missing_code_path` warning surfaces the gap until the file exists.

4. **Validate.** [[tools/spec/spec-validate]] over the whole tree. The expected warnings for an in-flight change:
   - `missing_code_path` for any new file you haven't created yet — accept temporarily.
   - `unresolved_ref` for any forward [[tools/spec/spec-validate]] you added — these should resolve as you finish editing other specs.
   - Anything else (`orphan`, `cycle`, `length_budget_exceeded`, `frontmatter_invalid_value`) is a real problem; fix before continuing.

5. **Implement the code change.** Now write or edit the Rust at the declared `code:` paths. Follow [[flows/mutating-edit]] for the apply path. Iterate until [[tools/cargo/cargo-check]] is clean.

6. **Drift check.** Run [[tools/spec/spec-diff]]. Expected result: no drift. If drift is reported, the spec and the code disagree — usually because a trait signature in the spec doesn't match the implementation. Decide which is right, edit the loser, loop back to step 5.

7. **Tests.** Run [[tools/cargo/cargo-test]] for the affected package. Failures are diagnostics in the [[flows/fix-diagnostic]] sense; loop until green.

8. **Commit together.** Stage the spec edits and the code edits as one commit via [[tools/vcs/vcs-commit]]. Reviewers see the spec delta and the code delta side by side — that's the whole point.

## Anti-patterns

- **Code first, spec later.** "I'll just write the code and update the spec in a follow-up." Don't — the spec edit is part of designing the change, not documenting it after the fact. Doing the spec edit forces you to confront the responsibility shift before the implementation locks in.
- **Spec edits without code.** Equally bad in the other direction. If the spec changes but no `code:` path moves, the spec was either documenting the existing code (which doesn't need to change) or claiming a capability that doesn't exist. Either way, look harder.
- **Bypassing the validator.** "I know it'll fail validate, I'll fix it later." `spec-ci-gate` will block the merge anyway ([[flows/spec-ci-gate]]); fixing locally is cheaper.

## See also

- [[flows/add-tool]] — the specialised version for new tools
- [[flows/fix-diagnostic]] — when a compiler error reveals spec drift
- [[flows/spec-ci-gate]] — what blocks merge if the spec edit was skipped
