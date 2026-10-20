//! Snapshot-based SSE with bounded memory and authorization rechecked on every update.
use super::RemoteService;
use axum::{
    http::{HeaderMap, StatusCode},
    response::sse::{Event, KeepAlive, Sse},
};
use std::{convert::Infallible, time::Duration};

impl RemoteService {
    pub(crate) async fn events(
        &self,
        headers: HeaderMap,
        id: String,
        after: i64,
    ) -> std::result::Result<
        Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>,
        StatusCode,
    > {
        self.authenticate(&headers, false).await?;
        let first = self.snapshot(&id).await?;
        let oldest = self
            .inner
            .store
            .remote_oldest_event(&id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .unwrap_or(first.seq);
        let reset = after > first.seq || after > 0 && after < oldest.saturating_sub(1);
        let service = self.clone();
        let stop = self.inner.runtime.lock().await.listener_stop.clone();
        let stream = async_stream::stream! {
            let mut last = first.clone();
            let mut seq = first.seq;
            let first_event = Event::default().event(if reset { "reset" } else { "snapshot" }).id(first.seq.to_string()).json_data(&first);
            if let Ok(event) = first_event { yield Ok(event); }
            let mut timer = tokio::time::interval(Duration::from_millis(250));
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! { _ = stop.cancelled() => break, _ = timer.tick() => {}, _ = service.inner.changed.notified() => {} }
                if !service.inner.settings.read().await.enabled || service.authenticate(&headers, false).await.is_err() { break; }
                let Ok(snapshot) = service.snapshot(&id).await else { break; };
                if snapshot.seq != seq || snapshot != last {
                    let kind = if snapshot.seq == last.seq && snapshot != last { "reset" } else { "snapshot" };
                    seq = snapshot.seq;
                    last = snapshot.clone();
                    if let Ok(event) = Event::default().event(kind).id(seq.to_string()).json_data(&snapshot) { yield Ok(event); }
                }
            }
        };
        Ok(Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("heartbeat"),
        ))
    }
}
