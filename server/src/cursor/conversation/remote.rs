//! Defines the transport-local mobile remote-control API.

use crate::{cursor::protocol::proto::agent::v1 as pb, model::RunId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteTarget {
    pub run_id: RunId,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteCommand {
    AppendText {
        command_id: String,
        text: String,
    },
    Stop,
    AnswerQuestion {
        interaction_id: u32,
        tool_call_id: String,
        answers: Vec<RemoteAnswer>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAnswer {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
    pub freeform_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteQuestion {
    pub interaction_id: u32,
    pub tool_call_id: String,
    pub args: pb::AskQuestionArgs,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoteState {
    pub target: RemoteTarget,
    pub questions: Vec<RemoteQuestion>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteResult {
    Applied,
    Duplicate,
    RunEnded,
    RunClosing,
    StaleTarget,
    AlreadyResolved,
    InvalidInput,
}
