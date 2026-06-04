```yaml
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

The **canonical** form is a fenced YAML block at the very top of the file — a ` ```yaml … ``` ` (or bare ` ``` ` / `~~~`) fence whose contents are the YAML, with **no `---` delimiters**:

````
```yaml
id: foo
kind: component
…
```

<body>
````

The fence makes raw-markdown viewers (GitHub, mdbook, `egui_commonmark` in our own file tabs) render the header as a code block; the fence already delimits the block, so the old `---` markers are redundant. The YAML runs to the closing fence. Missing frontmatter → error. Empty → empty `FrontmatterRecord` minus `id`/`kind` → validation error downstream. A single blank line between the closing ` ``` ` and the body's first prose line is consumed so body line numbers match what a reader sees in the editor.

**Tolerated legacy shapes** (the parser still accepts them, but they aren't the canonical form): an inner `---`/`...` pair inside the fence (stripped on parse), and a bare unfenced `---`…`---` block.

The one-shot `wrap_frontmatter` binary under `crates/oxidant-spec-tools/src/bin/` normalises any spec to the canonical fenced form — stripping inner `---`, wrapping bare blocks — and is idempotent on files already canonical.

## `tests:` field

Optional list per [[decisions/0011-specs-claim-their-tests]]. Each entry is one of:

- `<repo-relative-path>::<fn_name>` — a single test function. The path is the file containing `#[test] fn fn_name`; the validator's inventory pass produces matching ids.
- `<repo-relative-path>` — shorthand claiming every `#[test]` in that file.

Both forms parse to `TestRef`. The parser does not verify the path or function exists — that's the validator's job (`unresolved_test`). Many-to-many is allowed: the same test may appear in multiple specs' `tests:` lists.

## Body ref extraction

Naive regex `\[\[([^\]]+)\]\]` capturing the inner text. Refs inside fenced code blocks (``` or `~~~`) are ignored (the parser tracks fence state). Refs inside single-backtick inline code spans on the same line are also ignored, so documentation literals like `` `[[ref]]` `` don't register as real refs. Line/column captured for diagnostics from [[components/spec-tools/validate]].

## Why a separate component

The same parsing is needed by [[components/spec-tools/index-db]], [[components/spec-tools/validate]], [[components/spec-tools/diff]], and [[components/spec-tools/search-index]]. Centralising avoids drift in what counts as a valid spec file.
