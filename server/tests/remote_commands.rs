//! Commands never reach an absent or replaced run, and one command id executes at most once.
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
        transport::TransportRegistry,
    },
    model::ConversationId,
    remote::{RemoteDevice, RemoteService, RemoteSettings, SessionSnapshot, SessionSummary},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

const TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
async fn service() -> (tempfile::TempDir, RemoteService) {
    let (directory, store) = fixtures::temp_store().await;
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(PromptAssets::embedded().unwrap()),
    );
    let service = RemoteService::new(store.clone(), registry).await.unwrap();
    service
        .configure(RemoteSettings {
            public_origin: "https://dev.example.ts.net".into(),
            allowed_workspaces: vec!["D:/project".into()],
            ..Default::default()
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
    store
        .ensure_conversation(&ConversationId::new("conversation"))
        .await
        .unwrap();
    let mut snapshot = SessionSnapshot {
        summary: SessionSummary {
            id: "conversation".into(),
            title: "session".into(),
            workspaces: vec!["D:/project".into()],
            model: "model".into(),
            updated_at_ms: 1,
            status: "completed".into(),
            controllable: false,
        },
        seq: 0,
        epoch: "old".into(),
        target_ref: None,
        messages: vec![],
        questions: vec![],
        synchronized: true,
    };
    store.remote_save_snapshot(&mut snapshot).await.unwrap();
    (directory, service)
}
fn post(path: &str, body: Value) -> Request<Body> {
    Request::builder()
        .uri(path)
        .method("POST")
        .header("host", "dev.example.ts.net")
        .header("origin", "https://dev.example.ts.net")
        .header("cookie", format!("__Host-cursor-remote={TOKEN}"))
        .header("x-csrf-token", "csrf")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn finished_or_unknown_targets_are_rejected_without_execution() {
    let (_directory, service) = service().await;
    let response = service.router().oneshot(post("/api/sessions/conversation/commands", json!({"command_id": "c1", "target_ref": "unknown", "kind": "append_text", "text": "hello"}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(receipt["status"], "rejected");
    let response = service
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/commands/c1")
                .header("host", "dev.example.ts.net")
                .header("cookie", format!("__Host-cursor-remote={TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let receipt: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(receipt["reason"], "stale_target");
    let response = service.router().oneshot(post("/api/sessions/conversation/commands", json!({"command_id": "c1", "target_ref": "unknown", "kind": "append_text", "text": "hello"}))).await.unwrap();
    let receipt: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_ne!(receipt["status"], "accepted", "{receipt}");
}

#[tokio::test]
async fn a_reused_command_id_with_different_content_conflicts() {
    let (_directory, service) = service().await;
    let original = json!({"command_id": "same", "target_ref": "unknown", "kind": "append_text", "text": "first"});
    service
        .router()
        .oneshot(post("/api/sessions/conversation/commands", original))
        .await
        .unwrap();
    let response = service.router().oneshot(post("/api/sessions/conversation/commands", json!({"command_id": "same", "target_ref": "unknown", "kind": "append_text", "text": "second"}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn oversized_or_empty_appends_are_refused_before_any_receipt() {
    let (_directory, service) = service().await;
    for text in [String::new(), " ".into(), "x".repeat(16 * 1024 + 1)] {
        let response = service.router().oneshot(post("/api/sessions/conversation/commands", json!({"command_id": "big", "target_ref": "unknown", "kind": "append_text", "text": text}))).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
