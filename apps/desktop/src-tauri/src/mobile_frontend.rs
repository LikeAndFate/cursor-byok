use std::path::{Component, Path};

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, Response, StatusCode, Uri},
    routing::get,
    Router,
};
use tauri::AppHandle;

const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self'; frame-ancestors 'none'";

pub fn router(app: AppHandle) -> Router {
    Router::new()
        .route("/mobile", get(asset))
        .route("/mobile/", get(asset))
        .route("/mobile/{*path}", get(asset))
        .with_state(app)
}

async fn asset(State(app): State<AppHandle>, uri: Uri) -> Response<Body> {
    let requested = uri.path().strip_prefix("/mobile/").unwrap_or("");
    if invalid_path(requested) {
        return response(
            StatusCode::BAD_REQUEST,
            Body::empty(),
            None,
            AssetKind::Other,
        );
    }
    let path = if requested.is_empty() {
        "mobile/index.html".to_owned()
    } else {
        format!("mobile/{requested}")
    };
    let Some(asset) = app.asset_resolver().get(path) else {
        return response(StatusCode::NOT_FOUND, Body::empty(), None, AssetKind::Other);
    };
    let mime = HeaderValue::from_str(&asset.mime_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    response(
        StatusCode::OK,
        Body::from(asset.bytes),
        Some(mime),
        asset_kind(requested),
    )
}

#[derive(Clone, Copy)]
enum AssetKind {
    Entry,
    Immutable,
    Other,
}

fn asset_kind(path: &str) -> AssetKind {
    if path.is_empty() || matches!(path, "index.html" | "sw.js" | "manifest.webmanifest") {
        AssetKind::Entry
    } else if path.starts_with("assets/") {
        AssetKind::Immutable
    } else {
        AssetKind::Other
    }
}

fn invalid_path(path: &str) -> bool {
    Path::new(path).components().any(|part| {
        matches!(
            part,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn response(
    status: StatusCode,
    body: Body,
    mime: Option<HeaderValue>,
    kind: AssetKind,
) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    if let Some(mime) = mime {
        headers.insert(header::CONTENT_TYPE, mime);
    }
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let cache = match kind {
        AssetKind::Entry => "no-store",
        AssetKind::Immutable => "public, max-age=31536000, immutable",
        AssetKind::Other => "no-cache",
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traversal_segments_are_rejected() {
        assert!(invalid_path("../index.html"));
        assert!(!invalid_path("assets/index.js"));
    }

    #[test]
    fn cache_policy_separates_entries_and_hashed_assets() {
        assert!(matches!(asset_kind("index.html"), AssetKind::Entry));
        assert!(matches!(asset_kind("sw.js"), AssetKind::Entry));
        assert!(matches!(
            asset_kind("manifest.webmanifest"),
            AssetKind::Entry
        ));
        assert!(matches!(
            asset_kind("assets/index-hash.js"),
            AssetKind::Immutable
        ));
    }
}
