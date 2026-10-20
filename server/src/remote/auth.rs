//! Computer-confirmed one-use pairing and opaque, hashed device credentials.
use super::{PairingTicket, PendingPairing, RemoteDevice, RemoteService};
use crate::{store::now_ms, Error, Result};
use axum::http::{header, HeaderMap, StatusCode};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub(crate) const COOKIE: &str = "__Host-cursor-remote";
const LIFETIME_MS: i64 = 5 * 60 * 1000;
#[derive(Default)]
pub(crate) struct Pairings {
    tickets: HashMap<String, i64>,
    pub pending: HashMap<String, Pending>,
    attempts: Vec<i64>,
}
pub(crate) struct Pending {
    pub view: PendingPairing,
    secret_hash: Vec<u8>,
}
impl Pairings {
    pub fn prune(&mut self) {
        let now = now_ms();
        self.tickets.retain(|_, expires| *expires > now);
        self.pending
            .retain(|_, pending| pending.view.expires_at_ms > now);
        self.attempts.retain(|at| now - *at < 60_000);
    }
    pub(crate) fn permit(&mut self) -> bool {
        self.prune();
        if self.attempts.len() >= 60 {
            return false;
        }
        self.attempts.push(now_ms());
        true
    }
}
// Two independent UUIDv4 values supply 244 random bits without a new dependency.
pub(crate) fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub(crate) fn hash(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}
/// Loopback authority used by direct local smoke tests of the remote listener.
fn loopback_authority(host: &str) -> bool {
    let (name, port) = host.rsplit_once(':').unwrap_or((host, ""));
    let name = name.trim_start_matches('[').trim_end_matches(']');
    (name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1")
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

impl RemoteService {
    pub async fn create_pairing(&self) -> Result<PairingTicket> {
        let settings = self.inner.settings.read().await;
        if settings.public_origin.is_empty() {
            return Err(Error::Config(
                "configure the private HTTPS origin first".into(),
            ));
        }
        let mut state = self.inner.pairings.lock().await;
        state.prune();
        if state.tickets.len() + state.pending.len() >= 32 {
            return Err(Error::Config("too many pending pairings".into()));
        }
        let secret = secret();
        let expires_at_ms = now_ms() + LIFETIME_MS;
        state
            .tickets
            .insert(hex::encode(hash(&secret)), expires_at_ms);
        Ok(PairingTicket {
            url: format!("{}/mobile/#/pair?ticket={secret}", settings.public_origin),
            secret,
            expires_at_ms,
        })
    }
    pub async fn confirm_pairing(&self, id: &str) -> Result<()> {
        let mut state = self.inner.pairings.lock().await;
        state.prune();
        let pending = state
            .pending
            .get_mut(id)
            .ok_or_else(|| Error::Config("pairing expired or missing".into()))?;
        pending.view.confirmed = true;
        Ok(())
    }
    pub async fn revoke_device(&self, id: &str) -> Result<()> {
        let _gate = self.inner.command_gate.lock().await;
        self.inner.store.remote_revoke_device(id).await?;
        self.inner.changed.notify_waiters();
        Ok(())
    }
    pub(crate) async fn redeem(
        &self,
        ticket: &str,
        device_name: &str,
    ) -> std::result::Result<(String, String), StatusCode> {
        if ticket.len() > 256 || device_name.trim().is_empty() || device_name.len() > 128 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let mut state = self.inner.pairings.lock().await;
        if !state.permit() {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        let expiry = state
            .tickets
            .remove(&hex::encode(hash(ticket)))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let id = uuid::Uuid::new_v4().to_string();
        let secret = secret();
        state.pending.insert(
            id.clone(),
            Pending {
                view: PendingPairing {
                    id: id.clone(),
                    device_name: device_name.trim().to_owned(),
                    expires_at_ms: expiry,
                    confirmed: false,
                },
                secret_hash: hash(&secret),
            },
        );
        Ok((id, secret))
    }
    pub(crate) async fn complete(
        &self,
        id: &str,
        key: &str,
    ) -> std::result::Result<Option<String>, StatusCode> {
        if id.len() > 128 || key.len() > 256 {
            return Err(StatusCode::BAD_REQUEST);
        }
        let mut state = self.inner.pairings.lock().await;
        state.prune();
        let pending = match state.pending.get(id) {
            Some(pending) if same(&pending.secret_hash, &hash(key)) => pending,
            _ => {
                if !state.permit() {
                    return Err(StatusCode::TOO_MANY_REQUESTS);
                }
                return Err(StatusCode::UNAUTHORIZED);
            }
        };
        if !pending.view.confirmed {
            return Ok(None);
        }
        let token = secret();
        let device = RemoteDevice {
            id: uuid::Uuid::new_v4().to_string(),
            name: pending.view.device_name.clone(),
            created_at_ms: now_ms(),
            last_seen_at_ms: None,
            revoked_at_ms: None,
        };
        self.inner
            .store
            .remote_insert_device(&device, &hash(&token), &secret())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.pending.remove(id);
        Ok(Some(format!(
            "{COOKIE}={token}; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=31536000"
        )))
    }
    pub(crate) async fn authenticate(
        &self,
        headers: &HeaderMap,
        csrf: bool,
    ) -> std::result::Result<(RemoteDevice, String), StatusCode> {
        let token = headers
            .get(header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                value
                    .split(';')
                    .find_map(|part| part.trim().strip_prefix(&format!("{COOKIE}=")))
            })
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if token.len() != 64 {
            return Err(StatusCode::UNAUTHORIZED);
        }
        let identity = self
            .inner
            .store
            .remote_auth_device(&hash(token))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let Some((device, key)) = identity else {
            if !self.inner.pairings.lock().await.permit() {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
            return Err(StatusCode::UNAUTHORIZED);
        };
        if csrf
            && !headers
                .get("x-csrf-token")
                .and_then(|h| h.to_str().ok())
                .is_some_and(|value| same(value.as_bytes(), key.as_bytes()))
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok((device, key))
    }
    pub(crate) async fn check_origin(
        &self,
        headers: &HeaderMap,
        write: bool,
    ) -> std::result::Result<(), StatusCode> {
        let settings = self.inner.settings.read().await;
        let expected =
            url::Url::parse(&settings.public_origin).map_err(|_| StatusCode::FORBIDDEN)?;
        let authority = match expected.port() {
            Some(port) => format!("{}:{port}", expected.host_str().unwrap_or_default()),
            None => expected.host_str().unwrap_or_default().to_owned(),
        };
        // Tailscale Serve preserves the public Host header. Also accept the loopback authority so a
        // direct local smoke test works; cookies and Origin are still enforced independently.
        let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
        if host != Some(authority.as_str()) && !host.is_some_and(loopback_authority) {
            return Err(StatusCode::FORBIDDEN);
        }
        let origin = headers.get(header::ORIGIN).and_then(|h| h.to_str().ok());
        if (write || origin.is_some()) && origin != Some(settings.public_origin.as_str()) {
            return Err(StatusCode::FORBIDDEN);
        }
        if write
            && headers
                .get("sec-fetch-site")
                .is_some_and(|h| h != "same-origin" && h != "none")
        {
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(())
    }
}
