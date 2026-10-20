//! Isolated phone access to existing computer-owned Cursor sessions.
mod auth;
mod commands;
mod events;
mod projection;
mod routes;
mod runtime;
pub(crate) mod sessions;
mod types;

use crate::{cursor::transport::TransportRegistry, store::Store, Result};
pub(crate) use projection::DisplayState;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Notify, RwLock, Semaphore};
pub use types::*;

#[derive(Clone)]
pub struct RemoteService {
    pub(crate) inner: Arc<Inner>,
}
pub(crate) struct Inner {
    store: Store,
    registry: TransportRegistry,
    settings: RwLock<RemoteSettings>,
    epoch: String,
    pairings: Mutex<auth::Pairings>,
    runtime: Mutex<runtime::Runtime>,
    assets: parking_lot::RwLock<axum::Router>,
    changed: Notify,
    targets: Mutex<HashMap<String, commands::LiveTarget>>,
    command_gate: Mutex<()>,
    command_slots: Arc<Semaphore>,
}
impl RemoteService {
    pub async fn new(store: Store, registry: TransportRegistry) -> Result<Self> {
        let settings = store.remote_settings().await?;
        store.remote_restart().await?;
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                registry,
                settings: RwLock::new(settings),
                epoch: auth::secret(),
                pairings: Mutex::new(auth::Pairings::default()),
                runtime: Mutex::new(runtime::Runtime::default()),
                assets: parking_lot::RwLock::new(axum::Router::new()),
                changed: Notify::new(),
                targets: Mutex::new(HashMap::new()),
                command_gate: Mutex::new(()),
                command_slots: Arc::new(Semaphore::new(16)),
            }),
        })
    }
    pub async fn settings(&self) -> Result<RemoteSettings> {
        Ok(self.inner.settings.read().await.clone())
    }
    pub fn router(&self) -> axum::Router {
        routes::router(self.clone())
            .merge(self.inner.assets.read().clone())
            .route_layer(axum::middleware::from_fn_with_state(
                self.clone(),
                routes::guard,
            ))
    }
    pub fn management_router(&self) -> axum::Router {
        routes::management_router(self.clone())
    }
    /// Assets are installed before serve_on starts; no desktop API is ever merged here.
    pub fn set_assets(&self, assets: axum::Router) {
        *self.inner.assets.write() = assets;
    }
    pub async fn status(&self) -> RemoteStatus {
        let mut runtime = self.inner.runtime.lock().await;
        let running = runtime
            .listener
            .as_ref()
            .is_some_and(|task| !task.is_finished());
        if runtime.listener.is_some() && !running {
            runtime.error = Some("remote listener stopped".into());
        }
        let error = runtime.error.clone();
        drop(runtime);
        let mut pairings = self.inner.pairings.lock().await;
        pairings.prune();
        RemoteStatus {
            settings: self.inner.settings.read().await.clone(),
            running,
            error,
            pairings: pairings.pending.values().map(|p| p.view.clone()).collect(),
            devices: self.inner.store.remote_devices().await.unwrap_or_default(),
            projects: self
                .inner
                .store
                .remote_projects()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|path| RemoteProject { path })
                .collect(),
        }
    }
}
