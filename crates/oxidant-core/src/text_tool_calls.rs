// Realises spec/components/core/text-tool-call-extraction.md.
//
// Recovers tool calls that the model emitted as literal text inside its
// reply rather than via the provider's native tool-use mechanism. Two
// envelopes are recognised:
//
//   A) Hermes / Qwen3 JSON-in-tag:
//        <tool_call>
//        {"name": "foo", "arguments": {...}}
//        </tool_call>
//
//   B) smolagents / Qwen3-Coder function-and-parameters:
//        <tool_call>
//        <function=foo>
//        <parameter=key>value</parameter>
//        </function>
//        </tool_call>
//
// Outer `<tool_call>` wrappers and ```xml … ``` code fences are both
// optional.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedToolCall {
    pub name: String,
    /// Stringified JSON of the arguments object, matching the format the
    /// streaming `ToolUseInputDelta` events accumulate into.
    pub arguments_json: String,
}

#[derive(Debug, Default)]
pub struct ExtractionResult {
    /// Text with the recognised tool-call envelopes removed. Each
    /// envelope is replaced by a single newline so the surrounding
    /// prose stays intact.
    pub stripped_text: String,
    /// Calls discovered, in source order.
    pub calls: Vec<ExtractedToolCall>,
}

/// Quick gate: does the text contain any of the opening tokens we know
/// how to parse? Caller can skip the expensive walk when this returns
/// false.
pub fn looks_like_text_tool_call(text: &str) -> bool {
    text.contains("<tool_call>") || text.contains("<function=")
}

/// Extract every tool-call envelope from `text`. Returns the cleaned
/// text plus the list of calls in source order.
///
/// Per the spec, parse failures leave the offending block in the text
/// rather than dropping it silently — the user can re-prompt or fix
/// the agent state by hand.
pub fn extract(text: &str) -> ExtractionResult {
    let mut result = ExtractionResult::default();
    let mut cursor = 0;
    let bytes = text.as_bytes();
    while let Some(open_rel) = find_envelope_open(&text[cursor..]) {
        let open_abs = cursor + open_rel.start;
        // Carry over everything before the opening token.
        result.stripped_text.push_str(&text[cursor..open_abs]);

        match find_envelope_close(text, open_abs, open_rel.kind) {
            Some(close_abs) => {
                let body_start = open_abs + open_rel.open_token_len;
                let body = strip_code_fences(text[body_start..close_abs.body_end].trim());
                let block_end = close_abs.block_end;
                if let Some(call) = parse_body(body) {
                    result.calls.push(call);
                    result.stripped_text.push('\n');
                } else {
                    // Parse failure — keep the raw block so the user
                    // sees something went wrong.
                    tracing::warn!(
                        "text_tool_calls: failed to parse envelope body, leaving in text. body={body:?}"
                    );
                    result.stripped_text.push_str(&text[open_abs..block_end]);
                }
                cursor = block_end;
            }
            None => {
                // No close tag found — bail out and dump the remainder.
                result.stripped_text.push_str(&text[open_abs..]);
                cursor = bytes.len();
                break;
            }
        }
    }
    if cursor < bytes.len() {
        result.stripped_text.push_str(&text[cursor..]);
    }
    result
}

// ---------------------------------------------------------------- envelope finding

#[derive(Copy, Clone, PartialEq, Eq)]
enum EnvelopeKind {
    /// `<tool_call>…</tool_call>`
    ToolCall,
    /// Bare `<function=NAME>…</function>` with no outer wrapper.
    Function,
}

struct OpenMatch {
    start: usize,
    open_token_len: usize,
    kind: EnvelopeKind,
}

struct CloseMatch {
    /// Index of the close token's first byte (end of envelope body).
    body_end: usize,
    /// Index just past the close token (resume cursor here).
    block_end: usize,
}

fn find_envelope_open(text: &str) -> Option<OpenMatch> {
    let tc = text.find("<tool_call>");
    let fn_ = text.find("<function=");
    match (tc, fn_) {
        (Some(a), Some(b)) if a <= b => Some(OpenMatch {
            start: a,
            open_token_len: "<tool_call>".len(),
            kind: EnvelopeKind::ToolCall,
        }),
        (Some(a), None) => Some(OpenMatch {
            start: a,
            open_token_len: "<tool_call>".len(),
            kind: EnvelopeKind::ToolCall,
        }),
        (_, Some(b)) => Some(OpenMatch {
            start: b,
            open_token_len: 0, // body includes the <function=NAME>… as-is
            kind: EnvelopeKind::Function,
        }),
        (None, None) => None,
    }
}

fn find_envelope_close(
    text: &str,
    open_abs: usize,
    kind: EnvelopeKind,
) -> Option<CloseMatch> {
    let close_tag = match kind {
        EnvelopeKind::ToolCall => "</tool_call>",
        EnvelopeKind::Function => "</function>",
    };
    let search_start = open_abs + 1;
    let rel = text[search_start..].find(close_tag)?;
    let body_end = search_start + rel;
    let block_end = body_end + close_tag.len();
    Some(CloseMatch {
        body_end,
        block_end,
    })
}

fn strip_code_fences(body: &str) -> &str {
    // Some models wrap the envelope body in ```xml … ``` or ```json … ```.
    let trimmed = body.trim();
    let after_open = trimmed
        .strip_prefix("```xml")
        .or_else(|| trimmed.strip_prefix("```json"))
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|s| s.trim_start())
        .unwrap_or(trimmed);
    after_open
        .strip_suffix("```")
        .map(|s| s.trim_end())
        .unwrap_or(after_open)
}

// ---------------------------------------------------------------- body parsing

fn parse_body(body: &str) -> Option<ExtractedToolCall> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') {
        parse_json_body(trimmed)
    } else if trimmed.starts_with("<function=") {
        parse_xml_body(trimmed)
    } else if let Some(inner) = strip_outer_tool_call(trimmed) {
        // The body was wrapped in another <tool_call> for some reason.
        parse_body(inner)
    } else {
        None
    }
}

fn strip_outer_tool_call(s: &str) -> Option<&str> {
    let inner = s.strip_prefix("<tool_call>")?;
    inner.strip_suffix("</tool_call>").map(|t| t.trim())
}

fn parse_json_body(body: &str) -> Option<ExtractedToolCall> {
    let v: Value = serde_json::from_str(body).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    // Accept both "arguments" (Hermes) and "parameters" (some Qwen variants).
    let args = v
        .get("arguments")
        .or_else(|| v.get("parameters"))
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    let arguments_json = serde_json::to_string(&args).ok()?;
    Some(ExtractedToolCall {
        name,
        arguments_json,
    })
}

fn parse_xml_body(body: &str) -> Option<ExtractedToolCall> {
    let after_eq = body.strip_prefix("<function=")?;
    let name_end = after_eq.find('>')?;
    let name = after_eq[..name_end].trim().to_string();
    if name.is_empty() {
        return None;
    }
    let inner = &after_eq[name_end + 1..];
    // Trim a trailing </function> if present (in Format B without an
    // outer tool_call wrapper, the close tag is part of the body the
    // caller hands us).
    let inner = inner.strip_suffix("</function>").unwrap_or(inner).trim();

    let mut map = serde_json::Map::new();
    for (key, value) in walk_parameters(inner) {
        map.insert(key, Value::String(value));
    }
    let arguments_json = serde_json::to_string(&Value::Object(map)).ok()?;
    Some(ExtractedToolCall {
        name,
        arguments_json,
    })
}

/// Walk `<parameter=KEY>VALUE</parameter>` pairs out of `inner`. VALUE is
/// trimmed of leading/trailing whitespace. Nested parameter tags are
/// not supported; we take the first `</parameter>` after the opener.
fn walk_parameters(inner: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = inner[cursor..].find("<parameter=") {
        let open_abs = cursor + rel;
        let after_eq = &inner[open_abs + "<parameter=".len()..];
        let Some(name_end) = after_eq.find('>') else {
            break;
        };
        let key = after_eq[..name_end].trim().to_string();
        let value_start = open_abs + "<parameter=".len() + name_end + 1;
        let Some(close_rel) = inner[value_start..].find("</parameter>") else {
            break;
        };
        let value_end = value_start + close_rel;
        let value = inner[value_start..value_end].trim().to_string();
        out.push((key, value));
        cursor = value_end + "</parameter>".len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_text_tool_call_finds_open_tokens() {
        assert!(looks_like_text_tool_call("Let me try. <tool_call> …"));
        assert!(looks_like_text_tool_call("<function=foo>…"));
        assert!(!looks_like_text_tool_call("Just plain text, no tools."));
    }

    #[test]
    fn extract_hermes_json_envelope() {
        let text = r#"Sure, doing it now. <tool_call>
{"name": "fs_read", "arguments": {"file": "README.md", "limit": 10}}
</tool_call> Done."#;
        let result = extract(text);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].name, "fs_read");
        let args: Value = serde_json::from_str(&result.calls[0].arguments_json).unwrap();
        assert_eq!(args["file"], "README.md");
        assert_eq!(args["limit"], 10);
        assert!(
            result.stripped_text.contains("Sure, doing it now."),
            "prose before the call should survive: {:?}",
            result.stripped_text
        );
        assert!(
            result.stripped_text.contains("Done."),
            "prose after the call should survive: {:?}",
            result.stripped_text
        );
        assert!(
            !result.stripped_text.contains("<tool_call>"),
            "envelope should be stripped"
        );
    }

    #[test]
    fn extract_qwen3_coder_xml_envelope() {
        let text = "\
            Let me trim it. <tool_call>\n\
            <function=fs_write>\n\
            <parameter=file>\n\
            spec/foo.md\n\
            </parameter>\n\
            <parameter=content>\n\
            ---\n\
            id: foo\n\
            ---\n\
            </parameter>\n\
            </function>\n\
            </tool_call>";
        let result = extract(text);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].name, "fs_write");
        let args: Value = serde_json::from_str(&result.calls[0].arguments_json).unwrap();
        assert_eq!(args["file"], "spec/foo.md");
        let content = args["content"].as_str().unwrap();
        assert!(content.starts_with("---\nid: foo"));
        assert!(!result.stripped_text.contains("<tool_call>"));
        assert!(!result.stripped_text.contains("<function="));
    }

    #[test]
    fn extract_handles_bare_function_envelope_without_outer_wrapper() {
        let text = "Now: <function=ping><parameter=host>1.1.1.1</parameter></function> end.";
        let result = extract(text);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].name, "ping");
        let args: Value = serde_json::from_str(&result.calls[0].arguments_json).unwrap();
        assert_eq!(args["host"], "1.1.1.1");
    }

    #[test]
    fn extract_multiple_envelopes_in_one_response() {
        let text = "\
            First: <tool_call>{\"name\":\"a\",\"arguments\":{}}</tool_call>\n\
            Then:  <tool_call>{\"name\":\"b\",\"arguments\":{}}</tool_call>\
        ";
        let result = extract(text);
        assert_eq!(result.calls.len(), 2);
        assert_eq!(result.calls[0].name, "a");
        assert_eq!(result.calls[1].name, "b");
    }

    #[test]
    fn extract_accepts_parameters_key_as_arguments_alias() {
        let text = r#"<tool_call>{"name":"foo","parameters":{"x":1}}</tool_call>"#;
        let result = extract(text);
        assert_eq!(result.calls.len(), 1);
        let args: Value = serde_json::from_str(&result.calls[0].arguments_json).unwrap();
        assert_eq!(args["x"], 1);
    }

    #[test]
    fn extract_strips_xml_code_fences_around_body() {
        let text = "\
            <tool_call>\n\
            ```xml\n\
            <function=hi><parameter=who>world</parameter></function>\n\
            ```\n\
            </tool_call>";
        let result = extract(text);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].name, "hi");
    }

    #[test]
    fn extract_leaves_malformed_envelope_in_text() {
        let text = "<tool_call>not-json-not-xml</tool_call>";
        let result = extract(text);
        assert!(result.calls.is_empty());
        assert!(
            result.stripped_text.contains("not-json-not-xml"),
            "malformed body should remain in text: {:?}",
            result.stripped_text
        );
    }

    #[test]
    fn extract_handles_unclosed_envelope_gracefully() {
        let text = "Hello <tool_call>oh no I forgot to close";
        let result = extract(text);
        assert!(result.calls.is_empty());
        assert!(result.stripped_text.contains("oh no"));
    }

    #[test]
    fn extract_short_circuits_when_no_open_token() {
        let text = "Just a regular reply, nothing to see here.";
        let result = extract(text);
        assert!(result.calls.is_empty());
        assert_eq!(result.stripped_text, text);
    }
}
