//! Strict allowlisted phone API and separate local management API.
use super::{CommandInput, RemoteService, RemoteSettings};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use serde::Deserialize;
const SECURITY: [(header::HeaderName, &str); 4] = [
    (header::CACHE_CONTROL, "no-store"),
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (header::REFERRER_POLICY, "no-referrer"),
    (header::HeaderName::from_static("x-frame-options"), "DENY"),
];
fn response(status: StatusCode, body: impl IntoResponse) -> Response {
    let mut response = body.into_response();
    *response.status_mut() = status;
    for (name, value) in SECURITY {
        response.headers_mut().insert(name, value.parse().unwrap());
    }
    response
}
fn failure(status: StatusCode, message: &str) -> Response {
    response(status, Json(serde_json::json!({"error": message})))
}
fn map(
    result: std::result::Result<Response, StatusCode>,
    messages: impl Fn(StatusCode) -> &'static str,
) -> Response {
    match result {
        Ok(response) => response,
        Err(status) => failure(status, messages(status)),
    }
}
fn denied(status: StatusCode) -> &'static str {
    match status {
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::CONFLICT => "conflict",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        StatusCode::BAD_REQUEST => "invalid_request",
        _ => "unavailable",
    }
}
fn body_failure(status: StatusCode) -> &'static str {
    if status.is_client_error() {
        denied(status)
    } else {
        "unavailable"
    }
}

pub(crate) fn router(service: RemoteService) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Redirect::permanent("/mobile/") }),
        )
        .route("/api/status", get(status))
        .route("/api/pairings/redeem", post(redeem))
        .route("/api/pairings/complete", post(complete))
        .route("/api/sessions", get(sessions))
        .route("/api/sessions/{id}/snapshot", get(snapshot))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/commands", post(command))
        .route("/api/commands/{id}", get(receipt))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(32 * 1024))
        .with_state(service)
}
async fn not_found() -> Response {
    failure(StatusCode::NOT_FOUND, "not_found")
}
/// Every remote route passes the origin check. Security headers are added only to API
/// responses: the phone asset router owns its own CSP and cache policy so hashed bundles
/// stay immutable, while API state stays uncacheable.
pub(crate) async fn guard(
    State(service): State<RemoteService>,
    request: Request,
    next: Next,
) -> Response {
    if let Err(status) = service
        .check_origin(
            request.headers(),
            request.method() != axum::http::Method::GET
                && request.method() != axum::http::Method::HEAD,
        )
        .await
    {
        return failure(status, denied(status));
    }
    let api = request.uri().path().starts_with("/api/");
    let mut result = next.run(request).await;
    if api {
        for (name, value) in SECURITY {
            result.headers_mut().insert(name, value.parse().unwrap());
        }
    }
    result
}
pub(crate) fn management_router(service: RemoteService) -> Router {
    Router::new()
        .route("/__byok-api__/api/remote", get(manage).put(update))
        .route("/__byok-api__/api/remote/pairings", post(pairing))
        .route(
            "/__byok-api__/api/remote/pairings/{id}/confirm",
            post(confirm),
        )
        .route("/__byok-api__/api/remote/devices/{id}", delete(revoke))
        .with_state(service)
}
async fn manage(State(service): State<RemoteService>) -> Response {
    response(StatusCode::OK, Json(service.status().await))
}
async fn update(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Json(settings): Json<RemoteSettings>,
) -> Response {
    if let Err(error) = same_origin(&headers) {
        return failure(error, "forbidden");
    }
    match service.configure(settings).await {
        Ok(()) => response(StatusCode::OK, Json(service.status().await)),
        Err(error) => failure(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}
async fn pairing(State(service): State<RemoteService>, headers: HeaderMap) -> Response {
    if let Err(error) = same_origin(&headers) {
        return failure(error, "forbidden");
    }
    match service.create_pairing().await {
        Ok(ticket) => response(StatusCode::OK, Json(ticket)),
        Err(error) => failure(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}
async fn confirm(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(error) = same_origin(&headers) {
        return failure(error, "forbidden");
    }
    match service.confirm_pairing(&id).await {
        Ok(()) => response(StatusCode::OK, Json(serde_json::json!({"confirmed": true}))),
        Err(error) => failure(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}
async fn revoke(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(error) = same_origin(&headers) {
        return failure(error, "forbidden");
    }
    match service.revoke_device(&id).await {
        Ok(()) => response(StatusCode::OK, Json(serde_json::json!({"revoked": true}))),
        Err(error) => failure(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}
/// The local management API keeps its existing loopback CORS boundary and rejects foreign browser origins.
fn same_origin(headers: &HeaderMap) -> std::result::Result<(), StatusCode> {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin = origin.to_str().map_err(|_| StatusCode::FORBIDDEN)?;
    if origin == "tauri://localhost" {
        return Ok(());
    }
    let url = url::Url::parse(origin).map_err(|_| StatusCode::FORBIDDEN)?;
    match url.host_str() {
        Some("localhost" | "127.0.0.1" | "::1" | "tauri.localhost") => Ok(()),
        _ => Err(StatusCode::FORBIDDEN),
    }
}
async fn status(State(service): State<RemoteService>, headers: HeaderMap) -> Response {
    match service.authenticate(&headers, false).await {
        Ok((device, csrf)) => response(
            StatusCode::OK,
            Json(serde_json::json!({
                "device_id": device.id, "csrf_token": csrf, "epoch": service.inner.epoch,
                "host_name": host_name(), "version": env!("CARGO_PKG_VERSION"),
            })),
        ),
        Err(status) => failure(status, denied(status)),
    }
}
fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "computer".into())
}
#[derive(Deserialize)]
struct Redeem {
    ticket: String,
    device_name: String,
}
async fn redeem(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Json(input): Json<Redeem>,
) -> Response {
    if let Err(status) = service.check_origin(&headers, true).await {
        return failure(status, denied(status));
    }
    match service.redeem(&input.ticket, &input.device_name).await {
        Ok((id, secret)) => response(
            StatusCode::OK,
            Json(serde_json::json!({"id": id, "secret": secret})),
        ),
        Err(status) => failure(status, denied(status)),
    }
}
#[derive(Deserialize)]
struct Complete {
    id: String,
    secret: String,
}
async fn complete(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Json(input): Json<Complete>,
) -> Response {
    if let Err(status) = service.check_origin(&headers, true).await {
        return failure(status, denied(status));
    }
    match service.complete(&input.id, &input.secret).await {
        Ok(Some(cookie)) => {
            let mut response = response(StatusCode::OK, Json(serde_json::json!({"paired": true})));
            response
                .headers_mut()
                .insert(header::SET_COOKIE, cookie.parse().unwrap());
            response
        }
        Ok(None) => response(
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"pending": true})),
        ),
        Err(status) => failure(status, denied(status)),
    }
}
#[derive(Deserialize)]
struct SessionsQuery {
    before: Option<i64>,
    limit: Option<u32>,
}
async fn sessions(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Query(query): Query<SessionsQuery>,
) -> Response {
    if let Err(status) = service.authenticate(&headers, false).await {
        return failure(status, denied(status));
    }
    let list = match service
        .inner
        .store
        .remote_list_sessions(query.before, query.limit.unwrap_or(50).min(50))
        .await
    {
        Ok(list) => list,
        Err(_) => return failure(StatusCode::INTERNAL_SERVER_ERROR, "unavailable"),
    };
    let mut sessions = Vec::new();
    for snapshot in list {
        if service.authorized(&snapshot).await {
            let mut summary = snapshot.summary;
            summary.controllable &= snapshot.synchronized
                && snapshot.epoch == service.inner.epoch
                && service.inner.targets.lock().await.contains_key(&summary.id);
            sessions.push(summary);
        }
    }
    response(
        StatusCode::OK,
        Json(serde_json::json!({"sessions": sessions})),
    )
}
async fn snapshot(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(status) = service.authenticate(&headers, false).await {
        return failure(status, denied(status));
    }
    match service.snapshot(&id).await {
        Ok(snapshot) => response(StatusCode::OK, Json(snapshot)),
        Err(status) => failure(status, denied(status)),
    }
}
#[derive(Deserialize)]
struct EventsQuery {
    after: Option<i64>,
}
async fn events(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Response {
    map(
        service
            .events(headers, id, query.after.unwrap_or_default().max(0))
            .await
            .map(IntoResponse::into_response),
        body_failure,
    )
}
async fn command(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<CommandInput>,
) -> Response {
    if let Err(status) = service.check_origin(&headers, true).await {
        return failure(status, denied(status));
    }
    match service.command(&headers, &id, input).await {
        Ok(receipt) => response(StatusCode::OK, Json(receipt)),
        Err(status) => failure(status, denied(status)),
    }
}
async fn receipt(
    State(service): State<RemoteService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let device = match service.authenticate(&headers, false).await {
        Ok((device, _)) => device,
        Err(status) => return failure(status, denied(status)),
    };
    match service
        .inner
        .store
        .remote_command_receipt(&device.id, &id)
        .await
    {
        Ok(Some(receipt)) => response(StatusCode::OK, Json(receipt)),
        Ok(None) => failure(StatusCode::NOT_FOUND, "not_found"),
        Err(_) => failure(StatusCode::INTERNAL_SERVER_ERROR, "unavailable"),
    }
}
