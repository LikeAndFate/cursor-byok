//! Verifies mobile remote-control commands stay bound to one active Run generation.
#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;

use std::sync::Arc;

use cursor_server::{
    cursor::{
        conversation::{RemoteAnswer, RemoteCommand, RemoteResult, RemoteTarget},
        prompting::{PromptAssets, PromptCompiler},
        protocol::{connect, proto::agent::v1 as pb},
        TransportCommand, TransportRegistry,
    },
    model::{RunId, Usage},
    provider::{FinishReason, ModelEvent},
};
use prost::Message;

#[tokio::test]
async fn remote_state_tracks_only_the_active_run_generation() {
    let (_directory, registry, provider) = registry().await;
    provider.push_pending();
    let handle = registry.get_or_create("remote-state").await.unwrap();
    let mut output = handle.subscribe();
    assert_eq!(handle.remote_state().await, None);
    start(&handle, "remote-state", "remote-state-conversation").await;
    wait_for_request_with_output(&handle, &provider, 1, &mut output).await;

    let state = handle.remote_state().await.expect("active remote state");
    assert!(state.target.run_id.as_str().starts_with("remote-state:"));
    assert_eq!(state.target.generation, 1);
    assert!(state.questions.is_empty());

    assert_eq!(
        handle
            .remote_command(
                RemoteTarget {
                    run_id: RunId::new("other-run"),
                    generation: state.target.generation,
                },
                RemoteCommand::Stop,
            )
            .await,
        RemoteResult::StaleTarget
    );
    assert!(handle.remote_state().await.is_some());
    registry.shutdown().await;
}

#[tokio::test]
async fn append_is_committed_once_and_emits_cursor_ui_without_using_bidi_seqno() {
    let (_directory, registry, provider) = registry().await;
    provider.push_pending();
    provider.push(text_response("continued remotely"));
    let handle = registry.get_or_create("remote-append").await.unwrap();
    let mut output = handle.subscribe();
    start(&handle, "remote-append", "remote-append-conversation").await;
    let mut append_seqno = wait_for_request_with_output(&handle, &provider, 1, &mut output).await;
    let target = handle.remote_state().await.unwrap().target;

    let command = RemoteCommand::AppendText {
        command_id: "mobile-command-1".into(),
        text: "message from phone".into(),
    };
    let applied = tokio::spawn({
        let handle = handle.clone();
        let target = target.clone();
        let command = command.clone();
        async move { handle.remote_command(target, command).await }
    });

    let mut saw_queued = false;
    let mut saw_appended = false;
    let mut saw_continued = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !(saw_queued && saw_appended && saw_continued) {
        assert!(tokio::time::Instant::now() < deadline);
        let frame = match tokio::time::timeout(std::time::Duration::from_millis(100), output.recv())
            .await
        {
            Ok(Some(frame)) => frame,
            Ok(None) => panic!("remote output closed"),
            Err(_) => continue,
        };
        let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
        assert_eq!(flags & connect::END_STREAM_FLAG, 0);
        let server = pb::AgentServerMessage::decode(payload).unwrap();
        if let Some(pb::agent_server_message::Message::InteractionUpdate(update)) = server.message {
            match update.message {
                Some(pb::interaction_update::Message::ContextInjectionState(update)) => {
                    saw_queued |= update.injection_id == "remote:mobile-command-1";
                }
                Some(pb::interaction_update::Message::UserMessageAppended(update)) => {
                    let user = update.user_message.unwrap();
                    saw_appended |= user.message_id == "remote:mobile-command-1"
                        && user.text == "message from phone";
                }
                Some(pb::interaction_update::Message::TextDelta(update)) => {
                    saw_continued |= update.text.contains("continued remotely");
                }
                _ => {}
            }
        }
        acknowledge_kv_at(&handle, &mut append_seqno, &frame).await;
    }
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), applied)
            .await
            .expect("append command did not commit")
            .unwrap(),
        RemoteResult::Applied
    );
    assert_eq!(
        handle.remote_command(target, command).await,
        RemoteResult::Duplicate
    );
    assert_eq!(provider.requests().len(), 2);
    let history = serde_json::to_string(&provider.requests()[1].history).unwrap();
    assert!(history.contains("message from phone"));
    registry.shutdown().await;
}

#[tokio::test]
async fn old_target_cannot_append_to_a_replacement_run() {
    let (_directory, registry, provider) = registry().await;
    provider.push_pending();
    provider.push_pending();
    let handle = registry.get_or_create("remote-replace").await.unwrap();
    let mut output = handle.subscribe();
    start(&handle, "first-run", "remote-replace-conversation").await;
    let mut append_seqno = wait_for_request_with_output(&handle, &provider, 1, &mut output).await;
    let old = handle.remote_state().await.unwrap().target;

    start_with_seqno(
        &handle,
        &mut append_seqno,
        "second-run",
        "remote-replace-conversation",
    )
    .await;
    wait_for_request_from_seqno(&handle, &provider, 2, &mut output, &mut append_seqno).await;
    let current = handle.remote_state().await.unwrap().target;
    assert_ne!(old, current);
    assert_eq!(
        handle
            .remote_command(
                old,
                RemoteCommand::AppendText {
                    command_id: "late-command".into(),
                    text: "must not leak".into(),
                },
            )
            .await,
        RemoteResult::StaleTarget
    );
    assert!(!serde_json::to_string(&provider.requests()[1].history)
        .unwrap()
        .contains("must not leak"));
    registry.shutdown().await;
}

#[tokio::test]
async fn invalid_question_answer_does_not_consume_and_first_valid_answer_wins() {
    let (_directory, registry, provider) = registry().await;
    provider.push(tool_response(
        "ask-call",
        "AskQuestion",
        r#"{"title":"Choice","questions":[{"id":"q1","prompt":"Pick","options":[{"id":"a","label":"A"},{"id":"b","label":"B"}],"allow_multiple":false}]}"#,
    ));
    provider.push(text_response("answered"));
    let handle = registry.get_or_create("remote-answer").await.unwrap();
    let mut output = handle.subscribe();
    start(&handle, "remote-answer", "remote-answer-conversation").await;

    let mut append_seqno = 1;
    let question = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(state) = handle.remote_state().await {
                if let Some(question) = state.questions.first() {
                    break (state.target, question.clone());
                }
            }
            if let Ok(Some(frame)) =
                tokio::time::timeout(std::time::Duration::from_millis(20), output.recv()).await
            {
                acknowledge_kv_at(&handle, &mut append_seqno, &frame).await;
            }
        }
    })
    .await
    .expect("AskQuestion did not become remotely visible");
    let invalid = RemoteCommand::AnswerQuestion {
        interaction_id: question.1.interaction_id,
        tool_call_id: question.1.tool_call_id.clone(),
        answers: vec![RemoteAnswer {
            question_id: "q1".into(),
            selected_option_ids: vec!["missing".into()],
            freeform_text: String::new(),
        }],
    };
    assert_eq!(
        handle.remote_command(question.0.clone(), invalid).await,
        RemoteResult::InvalidInput
    );
    assert_eq!(handle.remote_state().await.unwrap().questions.len(), 1);

    let valid = RemoteCommand::AnswerQuestion {
        interaction_id: question.1.interaction_id,
        tool_call_id: question.1.tool_call_id,
        answers: vec![RemoteAnswer {
            question_id: "q1".into(),
            selected_option_ids: vec!["a".into()],
            freeform_text: String::new(),
        }],
    };
    assert_eq!(
        handle
            .remote_command(question.0.clone(), valid.clone())
            .await,
        RemoteResult::Applied
    );
    assert_eq!(
        handle.remote_command(question.0, valid).await,
        RemoteResult::AlreadyResolved
    );
    registry.shutdown().await;
}

#[tokio::test]
async fn remote_stop_aborts_running_tool_before_end_stream() {
    let (_directory, registry, provider) = registry().await;
    provider.push(tool_response("read-call", "Read", r#"{"path":"/tmp/a"}"#));
    let handle = registry.get_or_create("remote-stop").await.unwrap();
    let mut output = handle.subscribe();
    start(&handle, "remote-stop", "remote-stop-conversation").await;

    let mut append_seqno = 1;
    let (target, exec_id) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let frame = output
                .recv()
                .await
                .expect("output closed before tool start");
            let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
            assert_eq!(flags & connect::END_STREAM_FLAG, 0);
            let server = pb::AgentServerMessage::decode(payload).unwrap();
            if let Some(pb::agent_server_message::Message::ExecServerMessage(exec)) = server.message
            {
                break (handle.remote_state().await.unwrap().target, exec.id);
            }
            acknowledge_kv_at(&handle, &mut append_seqno, &frame).await;
        }
    })
    .await
    .expect("Read tool did not start");

    assert_eq!(
        handle.remote_command(target, RemoteCommand::Stop).await,
        RemoteResult::Applied
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut saw_abort = false;
        loop {
            let frame = output
                .recv()
                .await
                .expect("output closed before canceled EndStream");
            let (flags, payload) = connect::decode_frames(&frame).unwrap().pop().unwrap();
            if flags & connect::END_STREAM_FLAG != 0 {
                assert!(saw_abort, "tool abort must precede canceled EndStream");
                break;
            }
            let server = pb::AgentServerMessage::decode(payload).unwrap();
            if let Some(pb::agent_server_message::Message::ExecServerControlMessage(control)) =
                server.message
            {
                if let Some(pb::exec_server_control_message::Message::Abort(abort)) =
                    control.message
                {
                    assert_eq!(abort.id, exec_id);
                    saw_abort = true;
                }
            }
        }
    })
    .await
    .expect("remote stop did not finish");
}

#[tokio::test]
async fn append_after_run_ended_is_rejected_and_never_reaches_a_later_run() {
    let (_directory, registry, provider) = registry().await;
    let release = provider.push_gated(text_response("finished"));
    let handle = registry.get_or_create("remote-ended").await.unwrap();
    let mut output = handle.subscribe();
    start(&handle, "remote-ended", "remote-ended-conversation").await;
    wait_for_request_with_output(&handle, &provider, 1, &mut output).await;
    let target = handle.remote_state().await.unwrap().target;
    release.notify_one();

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let frame = output.recv().await.expect("transport closed");
            let (flags, _) = connect::decode_frames(&frame).unwrap().pop().unwrap();
            if flags & connect::END_STREAM_FLAG != 0 || handle.remote_state().await.is_none() {
                break;
            }
            acknowledge_kv(&handle, &frame).await;
        }
    })
    .await
    .expect("run did not end");

    assert!(matches!(
        handle
            .remote_command(
                target,
                RemoteCommand::AppendText {
                    command_id: "after-end".into(),
                    text: "must not continue".into(),
                },
            )
            .await,
        RemoteResult::RunClosing | RemoteResult::RunEnded
    ));
    assert_eq!(provider.requests().len(), 1);
}

async fn registry() -> (
    tempfile::TempDir,
    TransportRegistry,
    fake_provider::FakeProvider,
) {
    let (directory, store) = fixtures::temp_store().await;
    let assets = PromptAssets::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("prompt/cursor")
            .as_path(),
    )
    .unwrap();
    let provider = fake_provider::FakeProvider::default();
    let registry = TransportRegistry::new(
        store,
        Arc::new(provider.clone()),
        PromptCompiler::new(assets),
    );
    (directory, registry, provider)
}

async fn start(
    handle: &cursor_server::cursor::TransportHandle,
    run_id: &str,
    conversation_id: &str,
) {
    handle
        .command(TransportCommand::Append {
            seqno: if run_id == "second-run" { 1 } else { 0 },
            message: Box::new(client_run(run_id, conversation_id)),
        })
        .await
        .unwrap();
}

async fn start_with_seqno(
    handle: &cursor_server::cursor::TransportHandle,
    seqno: &mut i64,
    run_id: &str,
    conversation_id: &str,
) {
    handle
        .command(TransportCommand::Append {
            seqno: *seqno,
            message: Box::new(client_run(run_id, conversation_id)),
        })
        .await
        .unwrap();
    *seqno += 1;
}

fn client_run(run_id: &str, conversation_id: &str) -> pb::AgentClientMessage {
    pb::AgentClientMessage {
        message: Some(pb::agent_client_message::Message::RunRequest(
            pb::AgentRunRequest {
                action: Some(pb::ConversationAction {
                    action: Some(pb::conversation_action::Action::UserMessageAction(
                        pb::UserMessageAction {
                            user_message: Some(pb::UserMessage {
                                text: "start".into(),
                                message_id: format!("user-{run_id}"),
                                mode: pb::AgentMode::Agent as i32,
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
                conversation_id: Some(conversation_id.into()),
                run_id: Some(run_id.into()),
                requested_model: Some(pb::RequestedModel {
                    model_id: "test-model".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    }
}

async fn wait_for_request_with_output(
    handle: &cursor_server::cursor::TransportHandle,
    provider: &fake_provider::FakeProvider,
    count: usize,
    output: &mut tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
) -> i64 {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut seqno = 1;
    while provider.requests().len() < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "provider did not start"
        );
        if let Ok(Some(frame)) =
            tokio::time::timeout(std::time::Duration::from_millis(20), output.recv()).await
        {
            acknowledge_kv_at(handle, &mut seqno, &frame).await;
        }
    }
    seqno
}

async fn wait_for_request_from_seqno(
    handle: &cursor_server::cursor::TransportHandle,
    provider: &fake_provider::FakeProvider,
    count: usize,
    output: &mut tokio::sync::mpsc::UnboundedReceiver<bytes::Bytes>,
    seqno: &mut i64,
) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while provider.requests().len() < count {
        assert!(
            tokio::time::Instant::now() < deadline,
            "provider did not start"
        );
        if let Ok(Some(frame)) =
            tokio::time::timeout(std::time::Duration::from_millis(20), output.recv()).await
        {
            acknowledge_kv_at(handle, seqno, &frame).await;
        }
    }
}

fn text_response(text: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::Start {
            model_call_id: format!("call-{text}"),
        },
        ModelEvent::TextStart,
        ModelEvent::TextDelta(text.into()),
        ModelEvent::TextEnd,
        ModelEvent::Usage(Usage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            total_tokens: Some(2),
            ..Default::default()
        }),
        ModelEvent::Done(FinishReason::Stop),
    ]
}

fn tool_response(call_id: &str, name: &str, arguments: &str) -> Vec<ModelEvent> {
    vec![
        ModelEvent::Start {
            model_call_id: format!("model-{call_id}"),
        },
        ModelEvent::ToolCallStart {
            index: 0,
            call_id: call_id.into(),
            name: name.into(),
        },
        ModelEvent::ToolCallArgumentsDelta {
            index: 0,
            delta: arguments.into(),
        },
        ModelEvent::ToolCallEnd { index: 0 },
        ModelEvent::Done(FinishReason::ToolUse),
    ]
}

async fn acknowledge_kv(handle: &cursor_server::cursor::TransportHandle, frame: &bytes::Bytes) {
    let mut seqno = 1;
    acknowledge_kv_at(handle, &mut seqno, frame).await;
}

async fn acknowledge_kv_at(
    handle: &cursor_server::cursor::TransportHandle,
    seqno: &mut i64,
    frame: &bytes::Bytes,
) {
    let (_, payload) = connect::decode_frames(frame).unwrap().pop().unwrap();
    let server = pb::AgentServerMessage::decode(payload).unwrap();
    if let Some(pb::agent_server_message::Message::KvServerMessage(kv)) = server.message {
        handle
            .command(TransportCommand::Append {
                seqno: *seqno,
                message: Box::new(pb::AgentClientMessage {
                    message: Some(pb::agent_client_message::Message::KvClientMessage(
                        pb::KvClientMessage {
                            id: kv.id,
                            message: Some(pb::kv_client_message::Message::SetBlobResult(
                                pb::SetBlobResult { error: None },
                            )),
                        },
                    )),
                }),
            })
            .await
            .unwrap();
        *seqno += 1;
    }
}
