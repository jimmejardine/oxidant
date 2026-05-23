// Realises spec/components/gui/transcript-tab.md.
//
// Scrollable centre tab rendering the Conversation. Streams the live
// assistant turn (text, thinking, tool calls) in place. Markdown
// rendering via egui_commonmark; rich tool-call cards.

use egui::{Color32, RichText, ScrollArea};

use oxidant_core::{ContentBlock, Message, ToolResultContent};

use crate::app::{LiveTurn, SharedState};

pub struct TranscriptPanel;

impl TranscriptPanel {
    pub fn render(&self, ui: &mut egui::Ui, state: &SharedState) {
        ScrollArea::vertical()
            .auto_shrink([false; 2])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for msg in &state.exploration.conversation.messages {
                    render_message(ui, msg);
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
                        .color(Color32::DARK_GRAY)
                        .small()
                    };
                    ui.label(summary);
                }
                if state.exploration.conversation.is_empty() && state.live_turn.is_none() {
                    ui.add_space(80.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("oxidant")
                                .size(32.0)
                                .color(Color32::DARK_GRAY),
                        );
                        ui.label(
                            RichText::new("Type a prompt below and press Ctrl+Enter to send.")
                                .color(Color32::GRAY),
                        );
                    });
                }
            });
    }
}

fn render_message(ui: &mut egui::Ui, msg: &Message) {
    match msg {
        Message::User { content } => {
            ui.label(RichText::new("user").color(Color32::LIGHT_BLUE).strong());
            for block in content {
                render_block(ui, block);
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
                    ui.label(
                        RichText::new(format!("[{sr:?}]"))
                            .color(Color32::DARK_GRAY)
                            .small(),
                    );
                }
                if let Some(u) = usage {
                    ui.label(
                        RichText::new(format!("in {} out {}", u.input_tokens, u.output_tokens))
                            .color(Color32::DARK_GRAY)
                            .small(),
                    );
                }
            });
            for block in content {
                render_block(ui, block);
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
            ui.label(
                RichText::new(format!("tool_result ({call_id})"))
                    .color(header_color)
                    .small(),
            );
            let body = match content {
                ToolResultContent::Text(s) => s.clone(),
                ToolResultContent::Json(v) => {
                    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
                }
            };
            tool_result_block(ui, &body);
        }
    }
}

fn render_block(ui: &mut egui::Ui, block: &ContentBlock) {
    match block {
        ContentBlock::Text(s) => {
            ui.label(s);
        }
        ContentBlock::Thinking(s) => {
            ui.collapsing(
                RichText::new("thinking").color(Color32::DARK_GRAY).small(),
                |ui| {
                    ui.label(s);
                },
            );
        }
        ContentBlock::ToolUse { id, name, input } => {
            let pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
            ui.collapsing(
                RichText::new(format!("tool_use · {name} ({id})")).color(Color32::YELLOW),
                |ui| {
                    ui.code(&pretty);
                },
            );
        }
        ContentBlock::Image { .. } => {
            ui.label(RichText::new("[image — not rendered in MVP]").color(Color32::DARK_GRAY));
        }
    }
}

fn render_live_turn(ui: &mut egui::Ui, turn: &LiveTurn) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("assistant")
                .color(Color32::LIGHT_GREEN)
                .strong(),
        );
        ui.spinner();
    });
    if !turn.thinking.is_empty() {
        ui.collapsing(
            RichText::new("thinking").color(Color32::DARK_GRAY).small(),
            |ui| {
                ui.label(&turn.thinking);
            },
        );
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

fn tool_result_block(ui: &mut egui::Ui, body: &str) {
    ScrollArea::vertical()
        .max_height(180.0)
        .auto_shrink([false; 2])
        .id_salt(body.len())
        .show(ui, |ui| {
            ui.code(body);
        });
}
