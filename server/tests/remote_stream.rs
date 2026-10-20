//! End-to-end streaming authorization and reset checks.
#[path = "support/fake_provider.rs"]
mod fake_provider;
#[path = "support/fixtures.rs"]
mod fixtures;
use axum::{body::Body, http::Request};
use cursor_server::{
    cursor::{
        prompting::{PromptAssets, PromptCompiler},
        transport::TransportRegistry,
    },
    model::ConversationId,
    remote::{RemoteDevice, RemoteService, RemoteSettings, SessionSnapshot, SessionSummary},
};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
const TOKEN: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
#[tokio::test]
async fn expired_cursor_resets_and_device_revocation_closes_existing_stream() {
    let (_directory, store) = fixtures::temp_store().await;
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(PromptAssets::embedded().unwrap()),
    );
    let service = RemoteService::new(store.clone(), registry).await.unwrap();
    service
        .configure(RemoteSettings {
            enabled: true,
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
    for _ in 0..3 {
        store.remote_save_snapshot(&mut snapshot).await.unwrap();
    }
    sqlx::query("DELETE FROM remote_events WHERE seq < 3")
        .execute(store.pool())
        .await
        .unwrap();
    let request = Request::builder()
        .uri("/api/sessions/conversation/events?after=1")
        .header("host", "dev.example.ts.net")
        .header("cookie", format!("__Host-cursor-remote={TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let response = service.router().oneshot(request).await.unwrap();
    assert_eq!(response.headers()["cache-control"], "no-store");
    let mut body = response.into_body();
    let first = next(&mut body).await.expect("first event");
    assert!(first.contains("event: reset"), "{first}");
    assert!(first.contains("id: 3"), "{first}");
    service.revoke_device("phone").await.unwrap();
    assert!(
        next(&mut body).await.is_none(),
        "revoked connection must close"
    );
}
async fn next(body: &mut Body) -> Option<String> {
    use axum::body::HttpBody;
    use std::future::poll_fn;
    let frame = tokio::time::timeout(
        Duration::from_secs(2),
        poll_fn(|cx| std::pin::Pin::new(&mut *body).poll_frame(cx)),
    )
    .await
    .expect("SSE response timeout")?;
    Some(String::from_utf8_lossy(&frame.unwrap().into_data().unwrap()).into_owned())
}
