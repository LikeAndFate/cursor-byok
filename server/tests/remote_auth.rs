//! Checks the remote listener's authentication and administration boundary.
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
    remote::{RemoteService, RemoteSettings},
};
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

async fn service() -> (tempfile::TempDir, RemoteService) {
    let (directory, store) = fixtures::temp_store().await;
    let registry = TransportRegistry::new(
        store.clone(),
        Arc::new(fake_provider::FakeProvider::default()),
        PromptCompiler::new(PromptAssets::embedded().unwrap()),
    );
    (
        directory,
        RemoteService::new(store, registry).await.unwrap(),
    )
}

fn request(path: &str, method: &str, body: Value) -> Request<Body> {
    Request::builder()
        .uri(path)
        .method(method)
        .header("host", "dev.example.ts.net")
        .header("origin", "https://dev.example.ts.net")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn defaults_do_not_enable_remote_access() {
    let (_directory, service) = service().await;
    assert_eq!(service.settings().await.unwrap(), RemoteSettings::default());
    assert!(!service.status().await.running);
}

#[tokio::test]
async fn private_router_has_no_management_or_cursor_routes() {
    let (_directory, service) = service().await;
    for path in [
        "/__byok-api__/api/models",
        "/byok/v1/models",
        "/agent.v1.AgentService/RunSSE",
        "/api/settings",
        "/api/desktop/open-external-url",
    ] {
        let response = service
            .router()
            .oneshot(request(path, "GET", Value::Null))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn unauthenticated_clients_cannot_read_sessions() {
    let (_directory, service) = service().await;
    service
        .configure(RemoteSettings {
            public_origin: "https://dev.example.ts.net".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let response = service
        .router()
        .oneshot(request("/api/sessions", "GET", Value::Null))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn pairing_requires_computer_confirmation_and_revocation_is_immediate() {
    let (_directory, service) = service().await;
    service
        .configure(RemoteSettings {
            public_origin: "https://dev.example.ts.net".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let ticket = service.create_pairing().await.unwrap();
    let response = service
        .router()
        .oneshot(request(
            "/api/pairings/redeem",
            "POST",
            json!({"ticket": ticket.secret, "device_name": "phone"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pending: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let complete_body = json!({"id": pending["id"], "secret": pending["secret"]});
    let response = service
        .router()
        .oneshot(request(
            "/api/pairings/complete",
            "POST",
            complete_body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(response.headers().get("set-cookie").is_none());
    service
        .confirm_pairing(pending["id"].as_str().unwrap())
        .await
        .unwrap();
    let response = service
        .router()
        .oneshot(request(
            "/api/pairings/complete",
            "POST",
            complete_body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("Secure")
            && cookie.contains("SameSite=Strict")
    );
    let session_cookie = cookie.split(';').next().unwrap();
    let response = service
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header("host", "dev.example.ts.net")
                .header("cookie", session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let device: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    service
        .revoke_device(device["device_id"].as_str().unwrap())
        .await
        .unwrap();
    let response = service
        .router()
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header("host", "dev.example.ts.net")
                .header("cookie", session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = service
        .router()
        .oneshot(request("/api/pairings/complete", "POST", complete_body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn cross_origin_pairing_is_rejected() {
    let (_directory, service) = service().await;
    service
        .configure(RemoteSettings {
            public_origin: "https://dev.example.ts.net".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mut input = request(
        "/api/pairings/redeem",
        "POST",
        json!({"ticket":"invalid", "device_name":"phone"}),
    );
    input
        .headers_mut()
        .insert("origin", "https://attacker.example".parse().unwrap());
    let response = service.router().oneshot(input).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
