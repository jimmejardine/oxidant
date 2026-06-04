// Realises spec/components/rust-tools/syn-tools.md plus the four model-
// facing syn tools: syn_find_items, syn_add_use, syn_add_derive,
// syn_rename_local. Complements the LSP layer: this is the fast,
// syntactic, write-capable tier — no semantic resolution, no subprocess.
//
// All transforms produce a WorkspaceEdit per contracts/workspace-edit and
// either return it as preview (apply=false) or route through
// oxidant_tools::apply (apply=true), reusing the substrate's atomic
// temp-file + rename + syn-parse rollback.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use proc_macro2::Span;
use quote::ToTokens;
use serde::Deserialize;
use serde_json::{Value, json};
use syn::spanned::Spanned;
use syn::{Attribute, File as SynFile, Item, Visibility};

use oxidant_core::{Tool, ToolCategory, ToolContext, ToolResult};
use oxidant_tools::{ApplyResult, Position as OxPos, Range as OxRange, TextEdit, WorkspaceEdit};

// ---------------------------------------------------------------- helpers

fn load_file(workspace_root: &Path, rel: &Path) -> Result<(PathBuf, String), String> {
    let abs = workspace_root.join(rel);
    let canonical = dunce::canonicalize(&abs)
        .map_err(|e| format!("canonicalise {} failed: {e}", abs.display()))?;
    let content = std::fs::read_to_string(&canonical)
        .map_err(|e| format!("read {} failed: {e}", canonical.display()))?;
    Ok((canonical, content))
}

/// Byte offset → LSP Position (0-indexed line, 0-indexed UTF-16 character).
fn byte_to_position(source: &str, byte: usize) -> OxPos {
    let byte = byte.min(source.len());
    let mut line: u32 = 0;
    let mut last_newline_byte: usize = 0;
    for (i, b) in source.as_bytes()[..byte].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            last_newline_byte = i + 1;
        }
    }
    let line_slice = &source[last_newline_byte..byte];
    let character: u32 = line_slice.chars().map(|c| c.len_utf16() as u32).sum();
    OxPos { line, character }
}

fn range_from_byte_range(source: &str, range: std::ops::Range<usize>) -> OxRange {
    OxRange {
        start: byte_to_position(source, range.start),
        end: byte_to_position(source, range.end),
    }
}

fn span_byte_range(span: Span) -> std::ops::Range<usize> {
    span.byte_range()
}

fn visibility_string(vis: &Visibility) -> &'static str {
    match vis {
        Visibility::Public(_) => "pub",
        Visibility::Restricted(_) => "pub(restricted)",
        Visibility::Inherited => "private",
    }
}

fn parse_rust(source: &str, file: &Path) -> Result<SynFile, String> {
    syn::parse_file(source).map_err(|e| format!("syn::parse_file({}) failed: {e}", file.display()))
}

fn apply_or_preview(
    workspace_root: &Path,
    file_rel: &Path,
    source: &str,
    edits: Vec<TextEdit>,
    apply: bool,
) -> Result<(Value, Option<ApplyResult>), String> {
    let workspace_edit_json = json!({
        "changes": {
            file_rel.to_string_lossy().replace('\\', "/"): edits
                .iter()
                .map(|e| json!({
                    "range": {
                        "start": { "line": e.range.start.line, "character": e.range.start.character },
                        "end":   { "line": e.range.end.line,   "character": e.range.end.character }
                    },
                    "new_text": e.new_text,
                    "expected_text": e.expected_text,
                }))
                .collect::<Vec<_>>()
        }
    });
    if !apply {
        return Ok((workspace_edit_json, None));
    }
    let mut changes: HashMap<PathBuf, Vec<TextEdit>> = HashMap::new();
    changes.insert(file_rel.to_path_buf(), edits);
    let _ = source; // (the substrate re-reads the file; we passed `source` for context only)
    let apply_result = oxidant_tools::apply(workspace_root, WorkspaceEdit { changes })
        .map_err(|e| e.to_string())?;
    Ok((workspace_edit_json, Some(apply_result)))
}

fn render_apply_result(ar: &Option<ApplyResult>) -> Value {
    match ar {
        None => Value::Null,
        Some(ar) => json!({
            "ok": true,
            "files": ar.files.iter().map(|f| json!({
                "path": f.path.to_string_lossy().replace('\\', "/"),
                "edits_applied": f.edits_applied,
            })).collect::<Vec<_>>(),
        }),
    }
}

// ---------------------------------------------------------------- syn_find_items

pub struct SynFindItems;

#[derive(Deserialize)]
struct FindItemsArgs {
    file: String,
    kind: String,
    #[serde(default)]
    name_pattern: Option<String>,
}

#[async_trait]
impl Tool for SynFindItems {
    fn name(&self) -> &str {
        "syn_find_items"
    }
    fn description(&self) -> &str {
        "Parse a Rust file via syn and return top-level items matching a kind (fn|struct|enum|trait|impl|mod|const|static|type) and optional name pattern. Returns name, kind, LSP-style range, visibility."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "kind"],
            "properties": {
                "file":         { "type": "string" },
                "kind":         {
                    "type": "string",
                    "enum": ["fn", "struct", "enum", "trait", "impl", "mod", "const", "static", "type"]
                },
                "name_pattern": { "type": "string", "description": "substring or /regex/" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: FindItemsArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let rel = Path::new(&args.file).to_path_buf();
        let (canonical, source) = match load_file(&workspace, &rel) {
            Ok(t) => t,
            Err(e) => return ToolResult::Err(e),
        };
        let parsed = match parse_rust(&source, &canonical) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };

        let matcher = match args.name_pattern.as_deref() {
            None => NameMatcher::Any,
            Some(s) => match NameMatcher::compile(s) {
                Ok(m) => m,
                Err(e) => return ToolResult::Err(e),
            },
        };

        let mut items = Vec::<Value>::new();
        for item in &parsed.items {
            let (name, kind_str, vis) = match item {
                Item::Fn(f) => (f.sig.ident.to_string(), "fn", visibility_string(&f.vis)),
                Item::Struct(s) => (s.ident.to_string(), "struct", visibility_string(&s.vis)),
                Item::Enum(e) => (e.ident.to_string(), "enum", visibility_string(&e.vis)),
                Item::Trait(t) => (t.ident.to_string(), "trait", visibility_string(&t.vis)),
                Item::Impl(i) => (impl_label(i), "impl", "private"),
                Item::Mod(m) => (m.ident.to_string(), "mod", visibility_string(&m.vis)),
                Item::Const(c) => (c.ident.to_string(), "const", visibility_string(&c.vis)),
                Item::Static(s) => (s.ident.to_string(), "static", visibility_string(&s.vis)),
                Item::Type(t) => (t.ident.to_string(), "type", visibility_string(&t.vis)),
                _ => continue,
            };
            if kind_str != args.kind {
                continue;
            }
            if !matcher.matches(&name) {
                continue;
            }
            let range = range_from_byte_range(&source, span_byte_range(item.span()));
            items.push(json!({
                "name": name,
                "kind": kind_str,
                "range": {
                    "start": { "line": range.start.line, "character": range.start.character },
                    "end":   { "line": range.end.line,   "character": range.end.character }
                },
                "visibility": vis,
            }));
        }
        ToolResult::Ok(json!({
            "items": items,
            "count": items.len(),
        }))
    }
}

fn impl_label(i: &syn::ItemImpl) -> String {
    let target = i.self_ty.to_token_stream().to_string();
    match &i.trait_ {
        Some((_, path, _)) => {
            let t = path.to_token_stream().to_string();
            format!("impl {t} for {target}")
        }
        None => format!("impl {target}"),
    }
}

enum NameMatcher {
    Any,
    Substring(String),
    Regex(regex::Regex),
}

impl NameMatcher {
    fn compile(s: &str) -> Result<NameMatcher, String> {
        if let Some(inner) = s.strip_prefix('/').and_then(|t| t.strip_suffix('/')) {
            return regex::Regex::new(inner)
                .map(NameMatcher::Regex)
                .map_err(|e| format!("invalid regex {inner:?}: {e}"));
        }
        Ok(NameMatcher::Substring(s.to_string()))
    }
    fn matches(&self, name: &str) -> bool {
        match self {
            NameMatcher::Any => true,
            NameMatcher::Substring(s) => name.contains(s.as_str()),
            NameMatcher::Regex(r) => r.is_match(name),
        }
    }
}

// ---------------------------------------------------------------- syn_add_use

pub struct SynAddUse;

#[derive(Deserialize)]
struct AddUseArgs {
    file: String,
    path: String,
    #[serde(default)]
    apply: Option<bool>,
}

#[async_trait]
impl Tool for SynAddUse {
    fn name(&self) -> &str {
        "syn_add_use"
    }
    fn description(&self) -> &str {
        "Insert `use <path>;` at the right location in a Rust file (after existing use clauses, or after inner attributes). Returns a WorkspaceEdit; apply=true routes through the workspace-edit substrate. If the exact path is already imported, returns `skipped_reason: \"already_imported\"` with no edit."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "path"],
            "properties": {
                "file":  { "type": "string" },
                "path":  { "type": "string", "description": "e.g. crate::foo::Bar or serde::Serialize" },
                "apply": { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: AddUseArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let rel = Path::new(&args.file).to_path_buf();
        let (canonical, source) = match load_file(&workspace, &rel) {
            Ok(t) => t,
            Err(e) => return ToolResult::Err(e),
        };
        let parsed = match parse_rust(&source, &canonical) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };

        let normalised_path = args.path.split_whitespace().collect::<String>();
        let already_imported = parsed.items.iter().any(|i| match i {
            Item::Use(u) => {
                let txt = u.to_token_stream().to_string().replace(' ', "");
                txt.contains(&normalised_path)
            }
            _ => false,
        });
        if already_imported {
            return ToolResult::Ok(json!({
                "workspace_edit": { "changes": {} },
                "applied":        false,
                "skipped_reason": "already_imported",
            }));
        }

        // Find insertion byte offset: after the last existing top-level
        // `use` item, else after the last inner attribute / module doc,
        // else at the start of file.
        let insert_byte = pick_use_insertion_point(&parsed, &source);
        let new_use = if normalised_path.ends_with(';') {
            format!("use {};\n", normalised_path.trim_end_matches(';'))
        } else {
            format!("use {};\n", normalised_path)
        };
        let range = range_from_byte_range(&source, insert_byte..insert_byte);
        let edits = vec![TextEdit {
            range,
            new_text: new_use,
            expected_text: None,
        }];

        let apply = args.apply.unwrap_or(false);
        let (workspace_edit, applied) =
            match apply_or_preview(&workspace, &rel, &source, edits, apply) {
                Ok(p) => p,
                Err(e) => return ToolResult::Err(e),
            };
        ToolResult::Ok(json!({
            "workspace_edit": workspace_edit,
            "applied":        applied.is_some(),
            "skipped_reason": Value::Null,
            "substrate":      render_apply_result(&applied),
        }))
    }
}

fn pick_use_insertion_point(file: &SynFile, source: &str) -> usize {
    let last_use_end = file
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Use(u) => Some(span_byte_range(u.span()).end),
            _ => None,
        })
        .max();
    if let Some(end) = last_use_end {
        return offset_past_newline(source, end);
    }
    let last_inner_attr_end = file
        .attrs
        .iter()
        .map(|a| span_byte_range(a.span()).end)
        .max();
    if let Some(end) = last_inner_attr_end {
        return offset_past_newline(source, end);
    }
    skip_leading_comments(source)
}

fn offset_past_newline(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut i = start.min(bytes.len());
    while i < bytes.len() && bytes[i] != b'\n' {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'\n' {
        i + 1
    } else {
        i
    }
}

fn skip_leading_comments(source: &str) -> usize {
    // Skip over //! and // lines at the top of the file.
    let mut i = 0;
    let bytes = source.as_bytes();
    while i < bytes.len() {
        let line_start = i;
        while i < bytes.len() && bytes[i] != b'\n' {
            i += 1;
        }
        let line = &source[line_start..i];
        let trimmed = line.trim_start();
        let keep_skipping =
            trimmed.starts_with("//") || trimmed.starts_with("#!") || trimmed.is_empty();
        if !keep_skipping {
            return line_start;
        }
        if i < bytes.len() && bytes[i] == b'\n' {
            i += 1;
        }
    }
    i
}

// ---------------------------------------------------------------- syn_add_derive

pub struct SynAddDerive;

#[derive(Deserialize)]
struct AddDeriveArgs {
    file: String,
    type_name: String,
    derive: DeriveArg,
    #[serde(default)]
    apply: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DeriveArg {
    One(String),
    Many(Vec<String>),
}

#[async_trait]
impl Tool for SynAddDerive {
    fn name(&self) -> &str {
        "syn_add_derive"
    }
    fn description(&self) -> &str {
        "Add a #[derive(...)] entry to a struct or enum, merging with any existing derive attribute (dedup, preserve original order). `derive` may be a single name or an array."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "type_name", "derive"],
            "properties": {
                "file":      { "type": "string" },
                "type_name": { "type": "string" },
                "derive": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "array", "items": { "type": "string" } }
                    ]
                },
                "apply": { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: AddDeriveArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let rel = Path::new(&args.file).to_path_buf();
        let (canonical, source) = match load_file(&workspace, &rel) {
            Ok(t) => t,
            Err(e) => return ToolResult::Err(e),
        };
        let parsed = match parse_rust(&source, &canonical) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };
        let new_derives: Vec<String> = match &args.derive {
            DeriveArg::One(s) => vec![s.clone()],
            DeriveArg::Many(v) => v.clone(),
        };

        // Find the target type and its attributes + leading byte position.
        let target = find_type_target(&parsed, &args.type_name);
        let (attrs, type_start) = match target {
            Ok(t) => t,
            Err(e) => return ToolResult::Err(e),
        };

        let merged_into_existing;
        let edits = if let Some(existing_attr) = attrs.iter().find(|a| a.path().is_ident("derive"))
        {
            // Merge into the existing derive attribute.
            let existing_list = match parse_derive_list(existing_attr) {
                Ok(l) => l,
                Err(e) => return ToolResult::Err(e),
            };
            let combined = dedup_preserve_order(&existing_list, &new_derives);
            let replacement = format!("#[derive({})]", combined.join(", "));
            let attr_range = span_byte_range(existing_attr.span());
            let edit = TextEdit {
                range: range_from_byte_range(&source, attr_range),
                new_text: replacement,
                expected_text: None,
            };
            merged_into_existing = true;
            vec![edit]
        } else {
            // Insert a new #[derive(...)] line immediately before the type.
            // We anchor at the start of the line containing type_start.
            let line_start = line_start_byte(&source, type_start);
            let indent = &source[line_start..type_start];
            let new_attr = format!("{indent}#[derive({})]\n", new_derives.join(", "));
            let range = range_from_byte_range(&source, line_start..line_start);
            merged_into_existing = false;
            vec![TextEdit {
                range,
                new_text: new_attr,
                expected_text: None,
            }]
        };

        let apply = args.apply.unwrap_or(false);
        let (workspace_edit, applied) =
            match apply_or_preview(&workspace, &rel, &source, edits, apply) {
                Ok(p) => p,
                Err(e) => return ToolResult::Err(e),
            };
        ToolResult::Ok(json!({
            "workspace_edit": workspace_edit,
            "applied":        applied.is_some(),
            "merged_into_existing": merged_into_existing,
            "substrate":      render_apply_result(&applied),
        }))
    }
}

fn find_type_target<'a>(
    file: &'a SynFile,
    type_name: &str,
) -> Result<(&'a [Attribute], usize), String> {
    let mut found: Vec<(&[Attribute], usize)> = Vec::new();
    for item in &file.items {
        match item {
            Item::Struct(s) if s.ident == type_name => {
                let start = span_byte_range(s.span()).start;
                found.push((s.attrs.as_slice(), start));
            }
            Item::Enum(e) if e.ident == type_name => {
                let start = span_byte_range(e.span()).start;
                found.push((e.attrs.as_slice(), start));
            }
            _ => {}
        }
    }
    match found.len() {
        0 => Err(format!("no struct or enum named {type_name:?} in file")),
        1 => Ok(found.into_iter().next().unwrap()),
        n => Err(format!(
            "{n} structs/enums named {type_name:?} in file; ambiguous"
        )),
    }
}

fn parse_derive_list(attr: &Attribute) -> Result<Vec<String>, String> {
    // #[derive(A, B, C)] — extract the comma-separated paths.
    let meta = &attr.meta;
    if let syn::Meta::List(list) = meta {
        let tokens = list.tokens.to_string();
        // tokens look like "A , B , C" — split on ',' and trim whitespace.
        let names: Vec<String> = tokens
            .split(',')
            .map(|s| s.split_whitespace().collect::<String>())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(names)
    } else {
        Err("existing #[derive] attribute is not in list form".into())
    }
}

fn dedup_preserve_order(existing: &[String], new: &[String]) -> Vec<String> {
    let mut out = existing.to_vec();
    for n in new {
        if !out.contains(n) {
            out.push(n.clone());
        }
    }
    out
}

fn line_start_byte(source: &str, byte: usize) -> usize {
    let bytes = source.as_bytes();
    let byte = byte.min(bytes.len());
    let mut i = byte;
    while i > 0 && bytes[i - 1] != b'\n' {
        i -= 1;
    }
    i
}

// ---------------------------------------------------------------- syn_rename_local

pub struct SynRenameLocal;

#[derive(Deserialize)]
struct RenameLocalArgs {
    file: String,
    span: RenameSpanArg,
    new_name: String,
    #[serde(default)]
    apply: Option<bool>,
}

#[derive(Deserialize)]
struct RenameSpanArg {
    start: PositionArg,
    end: PositionArg,
}

#[derive(Deserialize)]
struct PositionArg {
    line: u32,
    character: u32,
}

#[async_trait]
impl Tool for SynRenameLocal {
    fn name(&self) -> &str {
        "syn_rename_local"
    }
    fn description(&self) -> &str {
        "Rename a local binding (variable, parameter, local fn) within its enclosing function. Syntactic only — does not handle shadowing across nested scopes; cross-file or scope-aware renames should use rust_rename instead."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["file", "span", "new_name"],
            "properties": {
                "file":     { "type": "string" },
                "span": {
                    "type": "object",
                    "required": ["start", "end"],
                    "properties": {
                        "start": {
                            "type": "object",
                            "required": ["line", "character"],
                            "properties": {
                                "line":      { "type": "integer", "minimum": 0 },
                                "character": { "type": "integer", "minimum": 0 }
                            }
                        },
                        "end": {
                            "type": "object",
                            "required": ["line", "character"],
                            "properties": {
                                "line":      { "type": "integer", "minimum": 0 },
                                "character": { "type": "integer", "minimum": 0 }
                            }
                        }
                    }
                },
                "new_name": { "type": "string", "minLength": 1 },
                "apply":    { "type": "boolean", "default": false }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::ReadOnly
    }
    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let args: RenameLocalArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::Err(format!("invalid args: {e}")),
        };
        if !is_valid_rust_ident(&args.new_name) {
            return ToolResult::Err(format!(
                "{:?} is not a valid Rust identifier",
                args.new_name
            ));
        }
        let workspace = PathBuf::from(ctx.workspace_root.as_std_path());
        let rel = Path::new(&args.file).to_path_buf();
        let (canonical, source) = match load_file(&workspace, &rel) {
            Ok(t) => t,
            Err(e) => return ToolResult::Err(e),
        };
        let parsed = match parse_rust(&source, &canonical) {
            Ok(p) => p,
            Err(e) => return ToolResult::Err(e),
        };

        let start_byte = lsp_to_byte(&source, args.span.start.line, args.span.start.character);
        let end_byte = lsp_to_byte(&source, args.span.end.line, args.span.end.character);
        let identifier = source[start_byte..end_byte].trim();
        if identifier.is_empty() || !is_valid_rust_ident(identifier) {
            return ToolResult::Err(format!(
                "span [{}..{}] does not point at a Rust identifier (got {:?})",
                start_byte, end_byte, identifier
            ));
        }

        // Find the enclosing fn item.
        let Some((fn_start, fn_end)) = find_enclosing_fn_byte_range(&parsed, start_byte) else {
            return ToolResult::Err(
                "binding is not inside a function — syn_rename_local only handles function-local bindings".into(),
            );
        };

        // Walk identifier tokens within the fn body and collect occurrences
        // whose text equals `identifier`. This is the syntactic, no-scope
        // rename — the spec's documented limitation.
        let body = &source[fn_start..fn_end];
        let mut occurrences: Vec<std::ops::Range<usize>> = Vec::new();
        for (local_start, _) in body.match_indices(identifier) {
            let absolute_start = fn_start + local_start;
            let absolute_end = absolute_start + identifier.len();
            if !is_ident_boundary(&source, absolute_start, absolute_end) {
                continue;
            }
            occurrences.push(absolute_start..absolute_end);
        }
        if occurrences.is_empty() {
            return ToolResult::Err(format!(
                "no occurrences of identifier {identifier:?} inside enclosing fn"
            ));
        }

        let edits: Vec<TextEdit> = occurrences
            .iter()
            .map(|r| TextEdit {
                range: range_from_byte_range(&source, r.clone()),
                new_text: args.new_name.clone(),
                expected_text: Some(identifier.to_string()),
            })
            .collect();
        let occurrences_count = edits.len();

        let apply = args.apply.unwrap_or(false);
        let (workspace_edit, applied) =
            match apply_or_preview(&workspace, &rel, &source, edits, apply) {
                Ok(p) => p,
                Err(e) => return ToolResult::Err(e),
            };
        ToolResult::Ok(json!({
            "workspace_edit": workspace_edit,
            "applied":        applied.is_some(),
            "occurrences":    occurrences_count,
            "substrate":      render_apply_result(&applied),
        }))
    }
}

fn lsp_to_byte(source: &str, line: u32, character: u32) -> usize {
    let mut current_line: u32 = 0;
    let mut byte: usize = 0;
    let bytes = source.as_bytes();
    while current_line < line && byte < bytes.len() {
        if bytes[byte] == b'\n' {
            current_line += 1;
        }
        byte += 1;
    }
    // Walk by UTF-16 code units along this line.
    let line_slice = &source[byte..];
    let line_end = line_slice
        .as_bytes()
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(line_slice.len());
    let line_text = &line_slice[..line_end];
    let mut utf16_count: u32 = 0;
    let mut byte_in_line = 0;
    for ch in line_text.chars() {
        if utf16_count >= character {
            break;
        }
        utf16_count += ch.len_utf16() as u32;
        byte_in_line += ch.len_utf8();
    }
    byte + byte_in_line
}

fn find_enclosing_fn_byte_range(file: &SynFile, byte: usize) -> Option<(usize, usize)> {
    for item in &file.items {
        if let Item::Fn(f) = item {
            let range = span_byte_range(f.span());
            if range.start <= byte && byte < range.end {
                return Some((range.start, range.end));
            }
        }
        // Also dive into impl bodies — common case for methods.
        if let Item::Impl(i) = item {
            for impl_item in &i.items {
                if let syn::ImplItem::Fn(f) = impl_item {
                    let range = span_byte_range(f.span());
                    if range.start <= byte && byte < range.end {
                        return Some((range.start, range.end));
                    }
                }
            }
        }
    }
    None
}

fn is_ident_boundary(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    let prev = if start > 0 {
        Some(bytes[start - 1])
    } else {
        None
    };
    let next = if end < bytes.len() {
        Some(bytes[end])
    } else {
        None
    };
    let is_ident_char = |b: u8| b == b'_' || b.is_ascii_alphanumeric();
    !prev.map(is_ident_char).unwrap_or(false) && !next.map(is_ident_char).unwrap_or(false)
}

fn is_valid_rust_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if first != '_' && !first.is_alphabetic() {
        return false;
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use serde_json::json;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    fn ctx_for(dir: &Path) -> ToolContext {
        ToolContext {
            workspace_root: Utf8PathBuf::from_path_buf(dunce::canonicalize(dir).unwrap()).unwrap(),
            exploration_id: "syn-test".into(),
            cancellation: CancellationToken::new(),
            ui: None,
        }
    }

    fn write_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    fn read_file(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap()
    }

    // ----- find_items -----

    #[tokio::test]
    async fn find_items_returns_fns_and_structs() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "pub struct Alpha;\nstruct Beta;\npub fn one() {}\nfn two() {}\n",
        );
        let ctx = ctx_for(dir.path());
        let v = match SynFindItems
            .invoke(json!({ "file": "src.rs", "kind": "struct" }), &ctx)
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(
            items
                .iter()
                .any(|i| i["name"] == "Alpha" && i["visibility"] == "pub")
        );
        assert!(
            items
                .iter()
                .any(|i| i["name"] == "Beta" && i["visibility"] == "private")
        );
    }

    #[tokio::test]
    async fn find_items_name_pattern_substring() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "fn run_test() {}\nfn run_query() {}\nfn other() {}\n",
        );
        let ctx = ctx_for(dir.path());
        let v = match SynFindItems
            .invoke(
                json!({ "file": "src.rs", "kind": "fn", "name_pattern": "run_" }),
                &ctx,
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        assert_eq!(v["items"].as_array().unwrap().len(), 2);
    }

    // ----- add_use -----

    #[tokio::test]
    async fn add_use_inserts_after_existing_uses() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "use std::io;\nuse std::fmt;\n\nfn main() {}\n",
        );
        let v = match SynAddUse
            .invoke(
                json!({ "file": "src.rs", "path": "std::path::PathBuf", "apply": true }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        assert_eq!(v["applied"], true);
        let updated = read_file(dir.path(), "src.rs");
        assert!(updated.contains("use std::path::PathBuf;"));
        // New use should be after the existing block.
        let new_use_pos = updated.find("use std::path::PathBuf;").unwrap();
        let main_pos = updated.find("fn main").unwrap();
        assert!(new_use_pos < main_pos);
    }

    #[tokio::test]
    async fn add_use_skips_when_already_imported() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "use serde::Serialize;\n\nfn main() {}\n",
        );
        let v = match SynAddUse
            .invoke(
                json!({ "file": "src.rs", "path": "serde::Serialize" }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        assert_eq!(v["skipped_reason"], "already_imported");
        assert_eq!(v["applied"], false);
    }

    // ----- add_derive -----

    #[tokio::test]
    async fn add_derive_inserts_new_attribute() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "pub struct Point { x: i32, y: i32 }\n",
        );
        let v = match SynAddDerive
            .invoke(
                json!({
                    "file": "src.rs",
                    "type_name": "Point",
                    "derive": ["Debug", "Clone"],
                    "apply": true
                }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        assert_eq!(v["merged_into_existing"], false);
        let updated = read_file(dir.path(), "src.rs");
        assert!(updated.contains("#[derive(Debug, Clone)]"));
        assert!(updated.contains("pub struct Point"));
    }

    #[tokio::test]
    async fn add_derive_merges_into_existing() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "#[derive(Debug)]\npub struct Point { x: i32 }\n",
        );
        let v = match SynAddDerive
            .invoke(
                json!({
                    "file": "src.rs",
                    "type_name": "Point",
                    "derive": ["Clone", "Debug", "Copy"],
                    "apply": true
                }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        assert_eq!(v["merged_into_existing"], true);
        let updated = read_file(dir.path(), "src.rs");
        // Original Debug preserved; new ones appended in order; deduped.
        assert!(
            updated.contains("#[derive(Debug, Clone, Copy)]"),
            "got: {updated}"
        );
    }

    #[tokio::test]
    async fn add_derive_errors_on_missing_type() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "src.rs", "pub struct Other;\n");
        let result = SynAddDerive
            .invoke(
                json!({ "file": "src.rs", "type_name": "Point", "derive": "Debug" }),
                &ctx_for(dir.path()),
            )
            .await;
        assert!(matches!(result, ToolResult::Err(_)));
    }

    // ----- rename_local -----

    #[tokio::test]
    async fn rename_local_renames_parameter_inside_fn() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "src.rs",
            "fn add(a: i32, b: i32) -> i32 {\n    let result = a + b;\n    result\n}\n",
        );
        // Line 0 char 7 = `a` of `add(a:`. Range is exactly the identifier.
        let v = match SynRenameLocal
            .invoke(
                json!({
                    "file": "src.rs",
                    "span": {
                        "start": { "line": 0, "character": 7 },
                        "end":   { "line": 0, "character": 8 }
                    },
                    "new_name": "lhs",
                    "apply": true
                }),
                &ctx_for(dir.path()),
            )
            .await
        {
            ToolResult::Ok(v) => v,
            ToolResult::Err(e) => panic!("{e}"),
        };
        assert_eq!(v["applied"], true);
        let updated = read_file(dir.path(), "src.rs");
        assert!(updated.contains("fn add(lhs: i32"), "got: {updated}");
        assert!(updated.contains("let result = lhs + b"), "got: {updated}");
        // `add` (function name) should NOT have been renamed.
        assert!(updated.contains("fn add"), "function name preserved");
    }

    #[tokio::test]
    async fn rename_local_rejects_invalid_ident() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "src.rs", "fn f(x: i32) -> i32 { x }\n");
        let result = SynRenameLocal
            .invoke(
                json!({
                    "file": "src.rs",
                    "span": {
                        "start": { "line": 0, "character": 5 },
                        "end":   { "line": 0, "character": 6 }
                    },
                    "new_name": "1bad"
                }),
                &ctx_for(dir.path()),
            )
            .await;
        assert!(matches!(result, ToolResult::Err(_)));
    }
}
