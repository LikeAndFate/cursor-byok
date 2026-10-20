//! Public, bounded presentation types for the phone client.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub port: u16,
    pub public_origin: String,
    pub allowed_workspaces: Vec<String>,
}
impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 43127,
            public_origin: String::new(),
            allowed_workspaces: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteDevice {
    pub id: String,
    pub name: String,
    pub created_at_ms: i64,
    pub last_seen_at_ms: Option<i64>,
    pub revoked_at_ms: Option<i64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct PendingPairing {
    pub id: String,
    pub device_name: String,
    pub expires_at_ms: i64,
    pub confirmed: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct PairingTicket {
    pub secret: String,
    pub url: String,
    pub expires_at_ms: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct RemoteProject {
    pub path: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RemoteStatus {
    pub settings: RemoteSettings,
    pub running: bool,
    pub error: Option<String>,
    pub pairings: Vec<PendingPairing>,
    pub devices: Vec<RemoteDevice>,
    pub projects: Vec<RemoteProject>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub workspaces: Vec<String>,
    pub model: String,
    pub updated_at_ms: i64,
    pub status: String,
    pub controllable: bool,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct MessageView {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct QuestionItem {
    pub id: String,
    pub prompt: String,
    pub allow_multiple: bool,
    pub options: Vec<QuestionOption>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct QuestionView {
    pub r#ref: String,
    pub title: String,
    pub questions: Vec<QuestionItem>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SessionSnapshot {
    #[serde(flatten)]
    pub summary: SessionSummary,
    pub seq: i64,
    pub epoch: String,
    pub target_ref: Option<String>,
    pub messages: Vec<MessageView>,
    pub questions: Vec<QuestionView>,
    pub synchronized: bool,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct CommandReceipt {
    pub command_id: String,
    pub status: String,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommandAnswer {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub freeform_text: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CommandKind {
    AppendText {
        text: String,
    },
    Stop,
    AnswerQuestion {
        question_ref: String,
        answers: Vec<CommandAnswer>,
    },
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CommandInput {
    pub command_id: String,
    pub target_ref: String,
    #[serde(flatten)]
    pub kind: CommandKind,
}
