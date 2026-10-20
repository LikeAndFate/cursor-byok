//! Allowlisted output projection. Never serializes raw protobuf or model history.
use super::MessageView;
use crate::cursor::protocol::{connect, proto::agent::v1 as pb};
use prost::Message;

const MESSAGE_LIMIT: usize = 64 * 1024;
const TOTAL_LIMIT: usize = 256 * 1024;
#[derive(Clone, Default)]
pub(crate) struct DisplayState {
    pub messages: Vec<MessageView>,
    pub ended: bool,
    pub failed: bool,
    pub cancelled: bool,
    pub gap: bool,
    serial: u64,
}
impl DisplayState {
    pub fn new_turn(&mut self) {
        self.ended = false;
        self.failed = false;
        self.cancelled = false;
        self.serial += 1;
    }
    pub fn frame(&mut self, frame: &[u8]) {
        if frame.len() > 2 * 1024 * 1024 {
            self.gap = true;
            return;
        }
        let Ok(frames) = connect::decode_frames(frame) else {
            self.gap = true;
            return;
        };
        for (flags, payload) in frames {
            if flags & connect::END_STREAM_FLAG != 0 {
                self.ended = true;
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&payload) {
                    if let Some(code) = value.pointer("/error/code").and_then(|v| v.as_str()) {
                        self.cancelled = code == "canceled";
                        self.failed = !self.cancelled;
                    }
                }
                continue;
            }
            let Ok(message) = pb::AgentServerMessage::decode(payload) else {
                self.gap = true;
                continue;
            };
            let Some(pb::agent_server_message::Message::InteractionUpdate(update)) =
                message.message
            else {
                continue;
            };
            use pb::interaction_update::Message as M;
            match update.message {
                Some(M::TextDelta(delta)) => self.append(
                    if delta.is_server_notice {
                        "notice"
                    } else {
                        "assistant"
                    },
                    None,
                    &delta.text,
                ),
                Some(M::UserMessageAppended(update)) => {
                    if let Some(user) = update.user_message {
                        self.upsert(format!("user:{}", user.message_id), "user", &user.text);
                    }
                }
                Some(M::ToolCallStarted(update)) => self.upsert(
                    format!("tool:{}", update.call_id),
                    "tool",
                    &format!(
                        "{} — running{}",
                        tool_name(update.tool_call.as_ref()),
                        tool_summary(update.tool_call.as_ref())
                    ),
                ),
                Some(M::ToolCallCompleted(update)) => self.append(
                    "tool",
                    Some(format!("tool:{}", update.call_id)),
                    &tool_completion(update.tool_call.as_ref()),
                ),
                Some(M::ShellOutputDelta(update)) => match update.event {
                    Some(pb::shell_output_delta_update::Event::Stdout(delta)) => {
                        self.append("tool", None, &delta.data)
                    }
                    Some(pb::shell_output_delta_update::Event::Stderr(delta)) => {
                        self.append("tool", None, &delta.data)
                    }
                    Some(pb::shell_output_delta_update::Event::Exit(exit)) => self.append(
                        "tool",
                        None,
                        &format!(
                            "\nexit code: {}{}",
                            exit.code,
                            if exit.aborted { " (aborted)" } else { "" }
                        ),
                    ),
                    _ => {}
                },
                Some(M::ToolCallDelta(update)) => {
                    if let Some(pb::tool_call_delta::Delta::ShellToolCallDelta(delta)) =
                        update.tool_call_delta.and_then(|d| d.delta)
                    {
                        let text = match delta.delta {
                            Some(pb::shell_tool_call_delta::Delta::Stdout(d)) => d.content,
                            Some(pb::shell_tool_call_delta::Delta::Stderr(d)) => d.content,
                            None => String::new(),
                        };
                        self.append("tool", Some(format!("tool:{}", update.call_id)), &text);
                    }
                }
                Some(M::TurnEnded(_)) => self.ended = true,
                Some(M::StepStarted(_)) => self.ended = false,
                _ => {}
            }
        }
        self.bound();
    }
    fn upsert(&mut self, id: String, kind: &str, text: &str) {
        if self.messages.iter().any(|m| m.id == id) {
            return;
        }
        let (text, truncated) = bounded(text, MESSAGE_LIMIT);
        self.messages.push(MessageView {
            id,
            kind: kind.into(),
            text,
            truncated,
        });
    }
    fn append(&mut self, kind: &str, id: Option<String>, text: &str) {
        let index = match id.as_ref() {
            Some(id) => self.messages.iter().position(|m| &m.id == id),
            None => self
                .messages
                .len()
                .checked_sub(1)
                .filter(|index| self.messages[*index].kind == kind),
        };
        if let Some(index) = index {
            let message = &mut self.messages[index];
            let remaining = MESSAGE_LIMIT.saturating_sub(message.text.len());
            let (text, truncated) = bounded(text, remaining);
            message.text.push_str(&text);
            message.truncated |= truncated;
        } else {
            self.serial += 1;
            self.upsert(
                id.unwrap_or_else(|| format!("output:{}", self.serial)),
                kind,
                text,
            );
        }
    }
    fn bound(&mut self) {
        let mut size: usize = self.messages.iter().map(|m| m.text.len()).sum();
        let mut omitted = false;
        while self.messages.len() > 127 || size > TOTAL_LIMIT - 256 {
            size -= self.messages.remove(0).text.len();
            omitted = true;
        }
        if omitted
            && !self
                .messages
                .iter()
                .any(|message| message.id == "history-limit")
        {
            self.messages.insert(
                0,
                MessageView {
                    id: "history-limit".into(),
                    kind: "notice".into(),
                    text: "Earlier display content omitted by size limit.".into(),
                    truncated: true,
                },
            );
        }
    }
}
pub(crate) fn bounded(text: &str, limit: usize) -> (String, bool) {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    // Strip terminal controls; rendering is plain text, never terminal/HTML execution.
    (
        text[..end]
            .chars()
            .filter(|c| !c.is_control() || matches!(c, '\n' | '\t' | '\r'))
            .collect(),
        end < text.len(),
    )
}
fn tool_summary(tool: Option<&pb::ToolCall>) -> String {
    use pb::tool_call::Tool;
    let summary = match tool.and_then(|tool| tool.tool.as_ref()) {
        Some(Tool::ShellToolCall(call)) => call.description.as_deref().unwrap_or_default(),
        Some(Tool::ReadToolCall(call)) => call
            .args
            .as_ref()
            .map(|args| args.path.as_str())
            .unwrap_or_default(),
        Some(Tool::EditToolCall(call)) => call
            .args
            .as_ref()
            .map(|args| args.path.as_str())
            .unwrap_or_default(),
        Some(Tool::TaskToolCall(call)) => call
            .args
            .as_ref()
            .map(|args| args.description.as_str())
            .unwrap_or_default(),
        _ => "",
    };
    if summary.is_empty() {
        String::new()
    } else {
        format!("\n{}", bounded(summary, 1024).0)
    }
}
fn tool_completion(tool: Option<&pb::ToolCall>) -> String {
    let mut text = format!("\n{} — completed", tool_name(tool));
    if let Some(pb::tool_call::Tool::ShellToolCall(call)) = tool.and_then(|tool| tool.tool.as_ref())
    {
        if let Some(pb::shell_result::Result::Success(result)) = call
            .result
            .as_ref()
            .and_then(|result| result.result.as_ref())
        {
            text.push_str(&format!(" (exit {})\n", result.exit_code));
            text.push_str(&bounded(&result.stdout, 32 * 1024).0);
            text.push_str(&bounded(&result.stderr, 32 * 1024).0);
        }
    }
    text
}

fn tool_name(tool: Option<&pb::ToolCall>) -> &'static str {
    use pb::tool_call::Tool;
    match tool.and_then(|tool| tool.tool.as_ref()) {
        Some(Tool::ShellToolCall(_)) => "Shell",
        Some(Tool::AskQuestionToolCall(_)) => "AskQuestion",
        Some(Tool::ReadToolCall(_)) => "Read",
        Some(Tool::EditToolCall(_)) => "Edit",
        Some(Tool::McpToolCall(_)) => "MCP",
        Some(Tool::TaskToolCall(_)) => "Task",
        _ => "Tool",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn interaction(message: pb::interaction_update::Message) -> bytes::Bytes {
        connect::encode_message(&pb::AgentServerMessage {
            ttft_breakdown: None,
            message: Some(pb::agent_server_message::Message::InteractionUpdate(
                pb::InteractionUpdate {
                    message: Some(message),
                },
            )),
        })
        .unwrap()
    }
    #[test]
    fn non_display_payloads_are_excluded_and_text_is_bounded() {
        let mut display = DisplayState::default();
        display.frame(&interaction(
            pb::interaction_update::Message::ThinkingDelta(pb::ThinkingDeltaUpdate {
                text: "private thinking".into(),
                ..Default::default()
            }),
        ));
        assert!(display.messages.is_empty());
        display.frame(&interaction(pb::interaction_update::Message::TextDelta(
            pb::TextDeltaUpdate {
                text: "中".repeat(MESSAGE_LIMIT),
                is_server_notice: false,
            },
        )));
        assert_eq!(display.messages.len(), 1);
        assert!(display.messages[0].text.len() <= MESSAGE_LIMIT);
        assert!(display.messages[0].truncated);
    }
    #[test]
    fn cancellation_is_distinct_from_success_and_new_turn_clears_terminal_state() {
        let mut display = DisplayState::default();
        display.frame(
            &connect::encode_error_end_stream(&connect::ConnectStreamError {
                code: connect::ConnectCode::Canceled,
                message: "cancelled".into(),
                details: vec![],
            })
            .unwrap(),
        );
        assert!(display.ended && display.cancelled && !display.failed);
        display.new_turn();
        assert!(!display.ended && !display.cancelled);
    }
}
