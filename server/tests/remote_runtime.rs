//! End-to-end projection and lifecycle checks with a fake provider and real transport actor.
#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use cursor_server::{
    cursor::{
        prompting::{PromptAssets, PromptCompiler},
        protocol::{connect, proto::agent::v1 as pb},
        transport::{TransportHandle, TransportRegistry},
        TransportCommand,
    },
    remote::{RemoteDevice, RemoteService, RemoteSettings, SessionSnapshot},
    store::Store,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

const TOKEN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
struct Fixture {
    _directory: tempfile::TempDir,
    store: Store,
    registry: TransportRegistry,
    service: RemoteService,
    handle: TransportHandle,
    ack: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn start() -> Self {
        let (directory, store) = fixtures::temp_store().await;
        let provider = fake_provider::FakeProvider::default();
        provider.push_pending();
        let registry = TransportRegistry::new(
            store.clone(),
            Arc::new(provider),
            PromptCompiler::new(PromptAssets::embedded().unwrap()),
        );
        let service = RemoteService::new(store.clone(), registry.clone())
            .await
            .unwrap();
        let reserved = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = reserved.local_addr().unwrap().port();
        drop(reserved);
        service
            .configure(RemoteSettings {
                enabled: true,
                port,
                public_origin: "https://dev.example.ts.net".into(),
                allowed_workspaces: vec!["D:/project".into()],
            })
            .await
            .unwrap();
        store
            .remote_insert_device(
                &RemoteDevice {
                    id: "phone".into(),
                    name: "phone".into(),
                    created_at_ms: 0,
                    last_seen_at_ms: None,
                    revoked_at_ms: None,
                },
                &Sha256::digest(TOKEN.as_bytes()),
                "csrf",
            )
            .await
            .unwrap();
        service.start().await;
        let handle = registry.get_or_create("remote-integration").await.unwrap();
        let mut output = handle.subscribe();
        let client = handle.clone();
        let ack = tokio::spawn(async move {
            let mut seqno = 1;
            while let Some(frame) = output.recv().await {
                let Ok(frames) = connect::decode_frames(&frame) else {
                    continue;
                };
                for (flags, payload) in frames {
                    if flags != 0 {
                        continue;
                    }
                    if let Ok(pb::AgentServerMessage {
                        message: Some(pb::agent_server_message::Message::KvServerMessage(kv)),
                        ..
                    }) = <pb::AgentServerMessage as prost::Message>::decode(payload)
                    {
                        let _ = client
                            .command(TransportCommand::Append {
                                seqno,
                                message: Box::new(pb::AgentClientMessage {
                                    message: Some(
                                        pb::agent_client_message::Message::KvClientMessage(
                                            pb::KvClientMessage {
                                                id: kv.id,
                                                message: Some(
                                                    pb::kv_client_message::Message::SetBlobResult(
                                                        pb::SetBlobResult { error: None },
                                                    ),
                                                ),
                                            },
                                        ),
                                    ),
                                }),
                            })
                            .await;
                        seqno += 1;
                    }
                }
            }
        });
        handle
            .command(TransportCommand::Append {
                seqno: 0,
                message: Box::new(pb::AgentClientMessage {
                    message: Some(pb::agent_client_message::Message::RunRequest(
                        pb::AgentRunRequest {
                            conversation_id: Some("remote-conversation".into()),
                            run_id: Some("cursor-run".into()),
                            requested_model: Some(pb::RequestedModel {
                                model_id: "test-model".into(),
                                ..Default::default()
                            }),
                            action: Some(pb::ConversationAction {
                                action: Some(pb::conversation_action::Action::UserMessageAction(
                                    pb::UserMessageAction {
                                        user_message: Some(pb::UserMessage {
                                            text: "real user text".into(),
                                            message_id: "u1".into(),
                                            mode: pb::AgentMode::Agent as i32,
                                            ..Default::default()
                                        }),
                                        request_context: Some(pb::RequestContext {
                                            env: Some(pb::RequestContextEnv {
                                                workspace_paths: vec!["D:/project".into()],
                                                ..Default::default()
                                            }),
                                            rules: vec![pb::CursorRule {
                                                content: "private rules".into(),
                                                ..Default::default()
                                            }],
                                            ..Default::default()
                                        }),
                                        ..Default::default()
                                    },
                                )),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    )),
                }),
            })
            .await
            .unwrap();
        Self {
            _directory: directory,
            store,
            registry,
            service,
            handle,
            ack,
        }
    }
    async fn active_snapshot(&self) -> SessionSnapshot {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(snapshot) = self
                    .store
                    .remote_load_session("remote-conversation")
                    .await
                    .unwrap()
                {
                    if snapshot.summary.controllable && snapshot.synchronized {
                        break snapshot;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("active projected snapshot")
    }
    async fn finish(self) {
        self.service.shutdown().await;
        self.registry.shutdown().await;
        self.ack.abort();
    }
}
fn request(path: &str, method: &str, value: Option<Value>) -> Request<Body> {
    Request::builder()
        .uri(path)
        .method(method)
        .header("host", "dev.example.ts.net")
        .header("origin", "https://dev.example.ts.net")
        .header("cookie", format!("__Host-cursor-remote={TOKEN}"))
        .header("x-csrf-token", "csrf")
        .header("content-type", "application/json")
        .body(value.map(|v| Body::from(v.to_string())).unwrap_or_default())
        .unwrap()
}
#[tokio::test]
async fn readonly_disconnect_and_remote_shutdown_leave_cursor_running() {
    let fixture = Fixture::start().await;
    let snapshot = fixture.active_snapshot().await;
    assert_eq!(snapshot.summary.workspaces, ["D:/project"]);
    assert!(snapshot
        .messages
        .iter()
        .any(|message| message.text == "real user text"));
    assert!(!serde_json::to_string(&snapshot)
        .unwrap()
        .contains("private rules"));
    let response = fixture
        .service
        .router()
        .oneshot(request(
            "/api/sessions/remote-conversation/events?after=0",
            "GET",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    fixture.service.shutdown().await;
    assert!(fixture.handle.remote_state().await.is_some());
    fixture.finish().await;
}
#[tokio::test]
async fn stop_receipt_and_terminal_snapshot_follow_actual_run_cancellation() {
    let fixture = Fixture::start().await;
    let snapshot = fixture.active_snapshot().await;
    let response = fixture
        .service
        .router()
        .oneshot(request(
            "/api/sessions/remote-conversation/commands",
            "POST",
            Some(json!({"command_id":"stop-1", "target_ref":snapshot.target_ref, "kind":"stop"})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(receipt["status"], "accepted");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let receipt = fixture
                .store
                .remote_command_receipt("phone", "stop-1")
                .await
                .unwrap()
                .unwrap();
            if receipt.status != "accepted" {
                assert_eq!(receipt.status, "applied");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snapshot = fixture
                .store
                .remote_load_session("remote-conversation")
                .await
                .unwrap()
                .unwrap();
            if snapshot.summary.status == "cancelled" {
                assert!(!snapshot.summary.controllable);
                assert!(snapshot.target_ref.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    fixture.finish().await;
}
#[tokio::test]
async fn occupied_fixed_remote_port_fails_closed_without_random_fallback() {
    let (_directory, store) = fixtures::temp_store().await;
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(PromptAssets::embedded().unwrap()),
    );
    let service = RemoteService::new(store, registry).await.unwrap();
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = occupied.local_addr().unwrap().port();
    service
        .configure(RemoteSettings {
            enabled: true,
            port,
            public_origin: "https://dev.example.ts.net".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    service.start().await;
    let status = service.status().await;
    assert!(!status.running);
    assert!(status.error.is_some());
    assert_eq!(status.settings.port, port);
    service.shutdown().await;
}
