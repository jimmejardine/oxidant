// Realises spec/components/gui/transcript-tab.md.
//
// Scrollable centre tab rendering the Conversation. Streams the live
// assistant turn (text, thinking, tool calls) in place. Markdown
// rendering via egui_commonmark; rich tool-call cards.

use egui::{Color32, CursorIcon, Id, RichText, ScrollArea, Sense};

use oxidant_core::{ContentBlock, Message, ToolResultContent};

use crate::app::{LiveTurn, SharedState};
use crate::theme;

/// Anything longer than this collapses by default. Sized to roughly the
/// width of a typical chat input — short user prompts and one-liner
/// tool results stay expanded, paragraphs collapse.
const SUMMARY_LIMIT: usize = 120;

pub struct TranscriptPanel;

impl TranscriptPanel {
    pub fn render(&self, ui: &mut egui::Ui, state: &SharedState) {
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (msg_idx, msg) in state.exploration.conversation.messages.iter().enumerate() {
                    render_message(ui, msg_idx, msg);
                    ui.add_space(8.0);
                }
                if let Some(turn) = &state.live_turn {
                    render_live_turn(ui, turn);
                }
                if let Some(o) = &state.last_outcome {
                    ui.add_space(4.0);
                    let summary = if let Some(err) = &o.error {
                        RichText::new(format!("error: {err}")).color(Color32::RED)
                    } else {
                        RichText::new(format!(
                            "done · {} iterations · {} tool calls · in {} / out {}",
                            o.iterations, o.tool_calls, o.usage.input_tokens, o.usage.output_tokens
                        ))
                        .color(theme::muted_text())
                    };
                    ui.label(summary);
                }
                if state.exploration.conversation.is_empty() && state.live_turn.is_none() {
                    ui.add_space(80.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("oxidant")
                                .size(32.0)
                                .color(theme::faint_text()),
                        );
                        ui.label(
                            RichText::new("Type a prompt below and press Ctrl+Enter to send.")
                                .color(theme::muted_text()),
                        );
                    });
                }
            });
    }
}

fn render_message(ui: &mut egui::Ui, msg_idx: usize, msg: &Message) {
    match msg {
        Message::User { content } => {
            ui.label(RichText::new("user").color(Color32::LIGHT_BLUE).strong());
            for (b_idx, block) in content.iter().enumerate() {
                render_block(ui, msg_idx, b_idx, block);
            }
        }
        Message::Assistant {
            content,
            stop_reason,
            usage,
        } => {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("assistant")
                        .color(Color32::LIGHT_GREEN)
                        .strong(),
                );
                if let Some(sr) = stop_reason {
                    ui.label(RichText::new(format!("[{sr:?}]")).color(theme::muted_text()));
                }
                if let Some(u) = usage {
                    ui.label(
                        RichText::new(format!("in {} out {}", u.input_tokens, u.output_tokens))
                            .color(theme::muted_text()),
                    );
                }
            });
            for (b_idx, block) in content.iter().enumerate() {
                render_block(ui, msg_idx, b_idx, block);
            }
        }
        Message::ToolResult {
            call_id,
            content,
            is_error,
        } => {
            let header_color = if *is_error {
                Color32::RED
            } else {
                Color32::YELLOW
            };
            ui.label(RichText::new(format!("tool_result ({call_id})")).color(header_color));
            let body = match content {
                ToolResultContent::Text(s) => s.clone(),
                ToolResultContent::Json(v) => {
                    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
                }
            };
            let id = Id::new(("tool_result", msg_idx));
            collapsible_code(ui, id, &body);
        }
    }
}

fn render_block(ui: &mut egui::Ui, msg_idx: usize, block_idx: usize, block: &ContentBlock) {
    let id = Id::new(("block", msg_idx, block_idx));
    match block {
        ContentBlock::Text(s) => {
            collapsible_text(ui, id, s);
        }
        ContentBlock::Thinking(s) => {
            collapsible_thinking(ui, id, s);
        }
        ContentBlock::ToolUse {
            id: tool_id,
            name,
            input,
        } => {
            let pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
            egui::CollapsingHeader::new(
                RichText::new(format!("tool_use · {name} ({tool_id})")).color(Color32::YELLOW),
            )
            .id_salt(id)
            .show(ui, |ui| {
                ui.code(&pretty);
            });
        }
        ContentBlock::Image { .. } => {
            ui.label(RichText::new("[image — not rendered in MVP]").color(theme::muted_text()));
        }
    }
}

fn render_live_turn(ui: &mut egui::Ui, turn: &LiveTurn) {
    // Per spec: the live turn never collapses while a token stream is in
    // flight — collapsing a paragraph mid-stream would make the viewport
    // dance. Render everything expanded.
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("assistant")
                .color(Color32::LIGHT_GREEN)
                .strong(),
        );
        ui.spinner();
    });
    if !turn.thinking.is_empty() {
        ui.collapsing(RichText::new("thinking").color(theme::muted_text()), |ui| {
            ui.label(&turn.thinking);
        });
    }
    if !turn.text.is_empty() {
        ui.label(&turn.text);
    }
    for tc in &turn.tool_calls {
        let title = if tc.finished {
            format!("tool_use · {} ({}) ✓", tc.name, tc.id)
        } else {
            format!("tool_use · {} ({}) …", tc.name, tc.id)
        };
        ui.collapsing(RichText::new(title).color(Color32::YELLOW), |ui| {
            ui.code(&tc.input_buffer);
        });
    }
}

// ---------------------------------------------------------------- helpers

/// Core collapsible primitive used by every body-bearing block in the
/// transcript. Collapsed view shows `▸ {summary_text}` on a single
/// clickable line. Expanded view shows a `▾` toggle on its own line
/// (no header) followed by `body_fn`'s output. See "Collapsible line
/// items" in spec/components/gui/transcript-tab.md — the no-header-
/// when-expanded rule is the whole point of this helper over egui's
/// stock `CollapsingHeader`.
fn collapsible_block(
    ui: &mut egui::Ui,
    id: Id,
    summary_text: RichText,
    body_fn: impl FnOnce(&mut egui::Ui),
) {
    let mut open: bool = ui.data_mut(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !open {
        // The whole "▸ summary" row should be one click target. Inner
        // labels are non-interactive; we then `interact` the resulting
        // horizontal area's rect with Sense::click so the entire row
        // (including the gap between the arrow and the text) reacts to
        // the click. Using the labels' own .clicked() would miss clicks
        // on the gap and force two separate hot zones.
        let resp = ui
            .horizontal(|ui| {
                ui.label(RichText::new("▸").color(theme::muted_text()));
                ui.label(summary_text);
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            open = true;
        }
    } else {
        let resp = ui
            .add(
                egui::Label::new(RichText::new("▾").color(theme::muted_text()))
                    .sense(Sense::click()),
            )
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.clicked() {
            open = false;
        }
        body_fn(ui);
    }
    ui.data_mut(|d| d.insert_temp(id, open));
}

/// Render `full` as either a plain label (when it already fits on one
/// line) or a collapsible block. When collapsed, only the summary
/// shows; when expanded, only the body — no duplicated first sentence.
fn collapsible_text(ui: &mut egui::Ui, id: Id, full: &str) {
    match summarize(full) {
        None => {
            ui.label(full);
        }
        Some(summary) => {
            let body = full.to_string();
            collapsible_block(ui, id, RichText::new(summary), move |ui| {
                ui.label(&body);
            });
        }
    }
}

/// Thinking blocks always collapse. Summary uses the muted band so it
/// reads as secondary; body inherits default text colour.
fn collapsible_thinking(ui: &mut egui::Ui, id: Id, full: &str) {
    let summary = summarize(full).unwrap_or_else(|| {
        if full.trim().is_empty() {
            "thinking".to_string()
        } else {
            full.to_string()
        }
    });
    let body = full.to_string();
    collapsible_block(
        ui,
        id,
        RichText::new(summary).color(theme::muted_text()),
        move |ui| {
            ui.label(&body);
        },
    );
}

/// Tool-result bodies: collapse to a one-line summary, scroll the full
/// body in a code area when expanded.
fn collapsible_code(ui: &mut egui::Ui, id: Id, body: &str) {
    match summarize(body) {
        None => {
            ui.code(body);
        }
        Some(summary) => {
            let body_owned = body.to_string();
            collapsible_block(
                ui,
                id,
                RichText::new(summary).color(theme::muted_text()),
                move |ui| {
                    ScrollArea::vertical()
                        .max_height(360.0)
                        .auto_shrink([false; 2])
                        .id_salt(id.with("scroll"))
                        .show(ui, |ui| {
                            ui.code(&body_owned);
                        });
                },
            );
        }
    }
}

/// Returns `Some(first_sentence_or_120_chars + "…")` when the block
/// is long enough to be worth collapsing; `None` when it already fits.
fn summarize(text: &str) -> Option<String> {
    if text.len() <= SUMMARY_LIMIT && !text.contains('\n') {
        return None;
    }
    Some(summarise_to_one_line(text))
}

fn summarise_to_one_line(text: &str) -> String {
    // First newline cuts the summary regardless of length.
    let nl = text.find('\n').unwrap_or(text.len());
    let head = &text[..nl];

    // First sentence terminator within the (truncated to NL) head.
    let terminator = head
        .char_indices()
        .find(|(_, c)| matches!(c, '.' | '!' | '?'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(head.len());

    // Whichever ends sooner: terminator, SUMMARY_LIMIT, or end-of-head.
    let cut = terminator.min(head.len()).min(SUMMARY_LIMIT);
    let head_summary = head[..cut].trim_end();

    let truncated = cut < text.len();
    if truncated {
        format!("{head_summary}…")
    } else {
        head_summary.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_returns_none_for_short_single_line() {
        assert!(summarize("hello world").is_none());
        assert!(summarize("a".repeat(SUMMARY_LIMIT).as_str()).is_none());
    }

    #[test]
    fn summarize_collapses_when_over_limit() {
        let s = "a".repeat(SUMMARY_LIMIT + 1);
        let summary = summarize(&s).unwrap();
        assert!(summary.ends_with('…'));
        assert!(summary.len() <= SUMMARY_LIMIT + "…".len());
    }

    #[test]
    fn summarize_collapses_when_multiline_even_if_short() {
        let s = "line one\nline two";
        let summary = summarize(s).unwrap();
        assert_eq!(summary, "line one…");
    }

    #[test]
    fn summarize_stops_at_sentence_terminator() {
        // Has to be long enough to trigger collapse in the first place
        // (> SUMMARY_LIMIT). The first sentence is short, so the
        // summary should stop at its period.
        let s = "First sentence here. ".to_string() + &"more more ".repeat(20);
        let summary = summarize(&s).unwrap();
        assert_eq!(summary, "First sentence here.…");
    }

    #[test]
    fn summarize_handles_question_and_exclamation() {
        let s = "Is it broken? Maybe, maybe not. ".repeat(5);
        let summary = summarize(&s).unwrap();
        assert!(summary.starts_with("Is it broken?"));
    }

    #[test]
    fn summarize_truncates_at_120_when_no_sentence_break() {
        let s: String = "abc def ghi ".repeat(20);
        let summary = summarize(&s).unwrap();
        // 120 chars + trailing "…" (after trimming whitespace at the cut).
        let body = summary.trim_end_matches('…');
        assert!(body.len() <= SUMMARY_LIMIT);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn summarize_first_line_of_multiline_is_kept_in_full_when_short() {
        let s = "Short first line.\nThen a much longer second line with lots of words.";
        let summary = summarize(s).unwrap();
        assert_eq!(summary, "Short first line.…");
    }
}
