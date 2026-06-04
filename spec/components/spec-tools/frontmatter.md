```yaml
---
id: frontmatter
kind: component
parent: overview
order: 1
implements: []
depends_on: []
code:
  - crates/oxidant-spec-tools/src/frontmatter.rs
tests:
  - crates/oxidant-spec-tools/src/frontmatter.rs
status: active
responsibility: |
  Parse YAML frontmatter and body `[[refs]]` from spec markdown files; produce typed FrontmatterRecord and SpecBody structs.
---
```

The lexer/parser layer underneath everything else in `oxidant-spec-tools`. Pure function: bytes in, structured record out.

## API

```rust
pub fn parse(content: &str) -> Result<SpecFile, ParseError>;

pub struct SpecFile {
    pub frontmatter: FrontmatterRecord,
    pub body: String,
    pub refs_in_body: Vec<RefMention>,  // every [[...]] found
}

pub struct FrontmatterRecord {
    pub id: String,
    pub kind: SpecKind,
    pub order: Option<i64>,
    pub parent: Option<String>,
    pub implements: Vec<String>,
    pub depends_on: Vec<String>,
    pub code: Vec<PathBuf>,
    pub tests: Vec<TestRef>,
    pub status: SpecStatus,
    pub responsibility: Option<String>,
    pub extras: serde_json::Value,       // unknown keys preserved for forward-compat
}

pub enum TestRef {
    Function { path: PathBuf, name: String },  // "crates/x/tests/y.rs::name"
    WholeFile { path: PathBuf },                // "crates/x/tests/y.rs"
}

pub struct RefMention {
    pub raw: String,             // e.g. "tools/edit/apply-edits"
    pub line: usize,
    pub column: usize,
}
```

## Frontmatter grammar

YAML between `---` markers at the very top of the file. Missing → error. Empty → empty `FrontmatterRecord` minus `id`/`kind` → validation error downstream.

The frontmatter block MAY be wrapped in a ` ```yaml … ``` ` (or bare ` ``` `) code fence so that raw-markdown viewers (GitHub, mdbook, `egui_commonmark` in our own file tabs) render the header as a code block instead of collapsing the leading `---` into a horizontal rule. Both shapes parse to the same `FrontmatterRecord`; the canonical form for new specs is the fenced one. A trailing blank line between the closing ` ``` ` and the body's first prose line is consumed so the body line numbers match what a reader sees in the editor.

The one-shot `wrap_frontmatter` binary under `crates/oxidant-spec-tools/src/bin/` adds fences to any spec that doesn't have them and is idempotent on those that do.

## `tests:` field

Optional list per [[decisions/0011-specs-claim-their-tests]]. Each entry is one of:

- `<repo-relative-path>::<fn_name>` — a single test function. The path is the file containing `#[test] fn fn_name`; the validator's inventory pass produces matching ids.
- `<repo-relative-path>` — shorthand claiming every `#[test]` in that file.

Both forms parse to `TestRef`. The parser does not verify the path or function exists — that's the validator's job (`unresolved_test`). Many-to-many is allowed: the same test may appear in multiple specs' `tests:` lists.

## Body ref extraction

Naive regex `\[\[([^\]]+)\]\]` capturing the inner text. Refs inside fenced code blocks (``` or `~~~`) are ignored (the parser tracks fence state). Refs inside single-backtick inline code spans on the same line are also ignored, so documentation literals like `` `[[ref]]` `` don't register as real refs. Line/column captured for diagnostics from [[components/spec-tools/validate]].

## Why a separate component

The same parsing is needed by [[components/spec-tools/index-db]], [[components/spec-tools/validate]], [[components/spec-tools/diff]], and [[components/spec-tools/search-index]]. Centralising avoids drift in what counts as a valid spec file.
