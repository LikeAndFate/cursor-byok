//! Separate fixed loopback listener; its lifecycle never disconnects Cursor.
use super::{RemoteService, RemoteSettings};
use crate::{Error, Result};
use std::{
    future::IntoFuture,
    net::{Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::{net::TcpListener, task::JoinHandle};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(crate) struct Runtime {
    pub listener: Option<JoinHandle<()>>,
    pub error: Option<String>,
    pub(crate) listener_stop: CancellationToken,
    observer: Option<JoinHandle<()>>,
    started: bool,
}
impl RemoteService {
    pub async fn configure(&self, mut settings: RemoteSettings) -> Result<()> {
        if settings.port == 0 {
            return Err(Error::Config(
                "remote port must be fixed and non-zero".into(),
            ));
        }
        settings.public_origin = settings
            .public_origin
            .trim()
            .trim_end_matches('/')
            .to_owned();
        if !settings.public_origin.is_empty() {
            let url = url::Url::parse(&settings.public_origin)
                .map_err(|_| Error::Config("invalid private HTTPS origin".into()))?;
            if url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(Error::Config(
                    "remote origin must be an HTTPS origin without path or credentials".into(),
                ));
            }
        } else if settings.enabled {
            return Err(Error::Config("private HTTPS origin is required".into()));
        }
        if settings.allowed_workspaces.len() > 256
            || settings
                .allowed_workspaces
                .iter()
                .any(|path| super::sessions::workspace_key(path).is_none())
        {
            return Err(Error::Config(
                "allowed workspaces must be explicit absolute roots".into(),
            ));
        }
        let _gate = self.inner.command_gate.lock().await;
        let origin_changed =
            self.inner.settings.read().await.public_origin != settings.public_origin;
        self.inner.store.set_remote_settings(&settings).await?;
        *self.inner.settings.write().await = settings;
        if origin_changed || !self.inner.settings.read().await.enabled {
            *self.inner.pairings.lock().await = super::auth::Pairings::default();
        }
        self.inner.changed.notify_waiters();
        self.reconcile_listener().await;
        Ok(())
    }
    pub async fn start(&self) {
        let mut runtime = self.inner.runtime.lock().await;
        if runtime.started {
            return;
        }
        runtime.started = true;
        let weak = std::sync::Arc::downgrade(&self.inner);
        runtime.observer = Some(tokio::spawn(async move {
            let mut timer = tokio::time::interval(Duration::from_millis(250));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                let service = RemoteService { inner };
                if !service.inner.settings.read().await.enabled {
                    continue;
                }
                if let Err(error) = service.refresh().await {
                    tracing::warn!(%error, "remote presentation synchronization failed");
                    service.inner.targets.lock().await.clear();
                    service.inner.changed.notify_waiters();
                }
            }
        }));
        drop(runtime);
        self.reconcile_listener().await;
    }
    async fn reconcile_listener(&self) {
        let mut runtime = self.inner.runtime.lock().await;
        if !runtime.started {
            return;
        }
        runtime.listener_stop.cancel();
        if let Some(task) = runtime.listener.take() {
            task.abort();
            let _ = task.await;
        }
        runtime.listener_stop = CancellationToken::new();
        runtime.error = None;
        self.inner.targets.lock().await.clear();
        let settings = self.inner.settings.read().await.clone();
        if !settings.enabled {
            return;
        }
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, settings.port));
        let listener = match TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                runtime.error = Some(format!("remote listener cannot bind {address}: {error}"));
                return;
            }
        };
        let stop = runtime.listener_stop.clone();
        let router = self.router();
        runtime.listener = Some(tokio::spawn(async move {
            let server = axum::serve(listener, router).into_future();
            tokio::select! { _ = stop.cancelled() => {}, result = server => if let Err(error) = result { tracing::warn!(%error, "remote listener stopped"); } }
        }));
    }
    pub async fn shutdown(&self) {
        let mut runtime = self.inner.runtime.lock().await;
        runtime.started = false;
        runtime.listener_stop.cancel();
        if let Some(task) = runtime.listener.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = runtime.observer.take() {
            task.abort();
            let _ = task.await;
        }
        self.inner.targets.lock().await.clear();
        self.inner.changed.notify_waiters();
    }
}
