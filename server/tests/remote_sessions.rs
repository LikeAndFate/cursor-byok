//! Phone reads are limited to explicitly authorized, known, complete workspace sets.
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
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn every_workspace_root_must_be_authorized_and_unknown_is_hidden() {
    let (_directory, store) = fixtures::temp_store().await;
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
    let token = "a".repeat(64);
    store
        .remote_insert_device(
            &RemoteDevice {
                id: "phone".into(),
                name: "phone".into(),
                created_at_ms: 0,
                last_seen_at_ms: None,
                revoked_at_ms: None,
            },
            &Sha256::digest(token.as_bytes()),
            "csrf",
        )
        .await
        .unwrap();
    for (id, roots) in [
        ("allowed", vec!["D:/project"]),
        ("multi", vec!["D:/project", "D:/secret"]),
        ("unknown", vec![]),
        ("prefix", vec!["D:/project-other"]),
    ] {
        store
            .ensure_conversation(&ConversationId::new(id))
            .await
            .unwrap();
        let mut snapshot = SessionSnapshot {
            summary: SessionSummary {
                id: id.into(),
                title: id.into(),
                workspaces: roots.into_iter().map(str::to_owned).collect(),
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
    }
    for (id, expected) in [
        ("allowed", StatusCode::OK),
        ("multi", StatusCode::NOT_FOUND),
        ("unknown", StatusCode::NOT_FOUND),
        ("prefix", StatusCode::NOT_FOUND),
    ] {
        let response = service
            .router()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/sessions/{id}/snapshot"))
                    .header("host", "dev.example.ts.net")
                    .header("cookie", format!("__Host-cursor-remote={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{id}");
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    let response = service
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("host", "dev.example.ts.net")
                .header("cookie", format!("__Host-cursor-remote={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(value["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(value["sessions"][0]["id"], "allowed");
}

#[tokio::test]
async fn asset_cache_policy_survives_the_remote_guard() {
    let (_directory, store) = fixtures::temp_store().await;
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
    service.set_assets(axum::Router::new().route(
        "/mobile/assets/app.js",
        axum::routing::get(|| async {
            (
                [(
                    axum::http::header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable",
                )],
                "body",
            )
        }),
    ));
    let assets = service
        .router()
        .oneshot(
            Request::builder()
                .uri("/mobile/assets/app.js")
                .header("host", "dev.example.ts.net")
                .header("sec-fetch-site", "cross-site")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(assets.status(), StatusCode::OK);
    assert_eq!(
        assets.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    let api = service
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/sessions")
                .header("host", "dev.example.ts.net")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(api.headers()["cache-control"], "no-store");
    assert_eq!(api.headers()["x-content-type-options"], "nosniff");
}
