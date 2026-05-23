# Claude-specific guidance

Standing rules when working in this repo:

- **Spec is canonical.** For changes that cross a contract, responsibility, or invariant, edit the relevant file under `spec/` *first*, then implement against it. See `spec/decisions/0008-spec-is-canonical.md`. Bug fixes and internal refactors don't need a spec edit.
- **Tests run via nextest, not `cargo test`.** `cargo nextest run` for everything except doctests (`cargo test --doc`). See the Testing section in the README for the full cheatsheet.
- **Run `spec_validate` and `spec_diff` after non-trivial edits** to catch drift; CI gates this too.

Repo overview, environment, build / run / test, badges:

@README.md
