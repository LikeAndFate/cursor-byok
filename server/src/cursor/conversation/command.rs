//! Defines commands accepted by a Conversation runtime.

use tokio::sync::oneshot;

use crate::{
    cursor::{
        conversation::{RemoteCommand, RemoteResult, RemoteState, RemoteTarget},
        protocol::proto::agent::v1 as pb,
    },
    Error,
};

#[derive(Debug)]
pub enum RunFinish {
    TurnCompleted,
    Transport(TransportFinish),
}

#[derive(Debug)]
pub enum TransportFinish {
    Success,
    Failed(Error),
    Cancelled,
}

#[derive(Debug)]
pub enum TransportCommand {
    Append {
        seqno: i64,
        message: Box<pb::AgentClientMessage>,
    },
    RunFinished {
        generation: u64,
        finish: RunFinish,
    },
    RemoteState {
        result: oneshot::Sender<Option<RemoteState>>,
    },
    RemoteControl {
        target: RemoteTarget,
        command: RemoteCommand,
        result: oneshot::Sender<RemoteResult>,
    },
    Disconnect,
}
