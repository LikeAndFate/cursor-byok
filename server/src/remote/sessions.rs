//! Workspace authorization and capture of real hydrated request metadata.
use super::{
    auth, commands::LiveTarget, projection::bounded, MessageView, QuestionItem, QuestionOption,
    QuestionView, RemoteService, SessionSnapshot, SessionSummary,
};
use crate::{
    cursor::{conversation::RemoteState, protocol::proto::agent::v1 as pb},
    store::{now_ms, Store},
    Result,
};
use axum::http::StatusCode;

/// Conservative lexical identity: exact roots, never a prefix/parent permission.
pub(crate) fn workspace_key(path: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    if path.is_empty()
        || path.contains('\0')
        || path.split('/').any(|part| part == ".." || part == ".")
    {
        return None;
    }
    let rooted = path.starts_with('/')
        || path.as_bytes().get(1) == Some(&b':') && path.as_bytes().get(2) == Some(&b'/');
    if !rooted {
        return None;
    }
    let key = path.trim_end_matches('/').to_owned();
    Some(if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    })
}
impl RemoteService {
    pub(crate) async fn authorized(&self, snapshot: &SessionSnapshot) -> bool {
        let settings = self.inner.settings.read().await;
        !snapshot.summary.workspaces.is_empty()
            && snapshot.summary.workspaces.iter().all(|path| {
                workspace_key(path).is_some_and(|key| {
                    settings
                        .allowed_workspaces
                        .iter()
                        .any(|allowed| workspace_key(allowed).as_ref() == Some(&key))
                })
            })
    }
    pub(crate) async fn observed_session(&self, id: &str) -> bool {
        self.inner
            .registry
            .local_handles()
            .await
            .iter()
            .any(|handle| handle.conversation_id() == Some(id))
    }

    pub(crate) async fn snapshot(
        &self,
        id: &str,
    ) -> std::result::Result<SessionSnapshot, StatusCode> {
        let mut snapshot = self
            .inner
            .store
            .remote_load_session(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        if !self.authorized(&snapshot).await {
            return Err(StatusCode::NOT_FOUND);
        }
        if snapshot.epoch != self.inner.epoch
            || !self.inner.settings.read().await.enabled
            || !self.inner.targets.lock().await.contains_key(id)
        {
            snapshot.summary.controllable = false;
            snapshot.target_ref = None;
            snapshot.questions.clear();
        }
        snapshot.epoch = self.inner.epoch.clone();
        Ok(snapshot)
    }
    pub(crate) async fn refresh(&self) -> Result<()> {
        let handles = self.inner.registry.local_handles().await;
        for (id, request_id, display) in self.inner.registry.take_remote_finished() {
            if let Some(mut snapshot) = self.inner.store.remote_load_session(&id).await? {
                merge_display(&mut snapshot, &request_id, &display);
                snapshot.summary.status = if display.cancelled {
                    "cancelled"
                } else if display.failed {
                    "failed"
                } else if display.ended {
                    "completed"
                } else {
                    "unavailable"
                }
                .into();
                snapshot.summary.controllable = false;
                snapshot.target_ref = None;
                snapshot.questions.clear();
                snapshot.synchronized = !display.gap;
                snapshot.epoch = self.inner.epoch.clone();
                snapshot.summary.updated_at_ms = now_ms();
                self.inner.store.remote_save_snapshot(&mut snapshot).await?;
                self.inner.changed.notify_waiters();
            }
        }
        for handle in handles {
            if handle.parent().is_some() {
                continue;
            }
            let Some(id) = handle.conversation_id() else {
                continue;
            };
            let Some(mut snapshot) = self.inner.store.remote_load_session(id).await? else {
                continue;
            };
            let before = snapshot.clone();
            let display = handle.remote_display();
            merge_display(&mut snapshot, handle.request_id(), &display);
            let state =
                tokio::time::timeout(std::time::Duration::from_millis(100), handle.remote_state())
                    .await;
            if let Ok(Some(state)) = &state {
                let active = self
                    .inner
                    .store
                    .conversation(&crate::model::ConversationId::new(id))
                    .await?;
                if active
                    .as_ref()
                    .and_then(|conversation| conversation.active_run_id.as_ref())
                    != Some(&state.target.run_id)
                {
                    continue;
                }
            }
            match state {
                Ok(Some(state)) if !display.ended && !display.gap => {
                    self.install_target(&mut snapshot, handle.clone(), state)
                        .await
                }
                Ok(None) => {
                    snapshot.summary.controllable = false;
                    snapshot.target_ref = None;
                    snapshot.questions.clear();
                    if snapshot.summary.status == "running"
                        || snapshot.summary.status == "waiting_question"
                    {
                        snapshot.summary.status = if display.cancelled {
                            "cancelled"
                        } else if display.failed {
                            "failed"
                        } else if display.ended {
                            "completed"
                        } else {
                            "unavailable"
                        }
                        .into();
                    }
                }
                _ => {
                    snapshot.summary.controllable = false;
                    snapshot.target_ref = None;
                    snapshot.questions.clear();
                }
            }
            snapshot.epoch = self.inner.epoch.clone();
            snapshot.synchronized = !display.gap;
            if before != snapshot {
                snapshot.summary.updated_at_ms = now_ms();
                self.inner.store.remote_save_snapshot(&mut snapshot).await?;
                self.inner.changed.notify_waiters();
            }
        }
        // Closed transports are not execution evidence, even if the database says running.
        let ids: std::collections::HashSet<String> = self
            .inner
            .registry
            .local_handles()
            .await
            .iter()
            .filter_map(|h| h.conversation_id().map(str::to_owned))
            .collect();
        let mut targets = self.inner.targets.lock().await;
        targets.retain(|id, _| ids.contains(id));
        Ok(())
    }
    async fn install_target(
        &self,
        snapshot: &mut SessionSnapshot,
        handle: crate::cursor::transport::TransportHandle,
        state: RemoteState,
    ) {
        let mut targets = self.inner.targets.lock().await;
        let target = targets
            .entry(snapshot.summary.id.clone())
            .or_insert_with(|| LiveTarget::new(handle.clone(), state.target.clone()));
        if target.target != state.target || target.handle.request_id() != handle.request_id() {
            *target = LiveTarget::new(handle, state.target.clone());
        }
        target.questions.clear();
        snapshot.questions = state
            .questions
            .into_iter()
            .filter(|question| {
                prost::Message::encoded_len(&question.args) <= 32 * 1024
                    && question.args.questions.len() <= 32
                    && question.args.questions.iter().all(|q| {
                        q.id.len() <= 128
                            && q.options.len() <= 64
                            && q.options.iter().all(|o| o.id.len() <= 128)
                    })
            })
            .take(2)
            .map(|question| {
                let reference = format!(
                    "{}:{}:{}",
                    target.reference,
                    question.interaction_id,
                    hex::encode(auth::hash(&format!(
                        "{}:{}",
                        question.tool_call_id,
                        hex::encode(prost::Message::encode_to_vec(&question.args))
                    )))
                );
                let view = QuestionView {
                    r#ref: reference.clone(),
                    title: bounded(&question.args.title, 1024).0,
                    questions: question
                        .args
                        .questions
                        .iter()
                        .take(32)
                        .map(|q| QuestionItem {
                            id: q.id.clone(),
                            prompt: bounded(&q.prompt, 4096).0,
                            allow_multiple: q.allow_multiple,
                            options: q
                                .options
                                .iter()
                                .take(64)
                                .map(|option| QuestionOption {
                                    id: option.id.clone(),
                                    label: bounded(&option.label, 2048).0,
                                })
                                .collect(),
                        })
                        .collect(),
                };
                target.questions.insert(reference, question);
                view
            })
            .collect();
        snapshot.target_ref = Some(target.reference.clone());
        snapshot.summary.controllable = true;
        snapshot.summary.status = if snapshot.questions.is_empty() {
            "running"
        } else {
            "waiting_question"
        }
        .into();
    }
}
fn merge_display(snapshot: &mut SessionSnapshot, request_id: &str, display: &super::DisplayState) {
    for mut message in display.messages.clone() {
        if !message.id.starts_with("user:") {
            message.id = format!("{request_id}:{}", message.id);
        }
        if let Some(existing) = snapshot.messages.iter_mut().find(|m| m.id == message.id) {
            *existing = message;
        } else {
            snapshot.messages.push(message);
        }
    }
    let mut omitted = false;
    while snapshot.messages.len() > 127
        || snapshot
            .messages
            .iter()
            .map(|m| m.text.len())
            .sum::<usize>()
            > 256 * 1024 - 256
    {
        snapshot.messages.remove(0);
        omitted = true;
    }
    if omitted {
        snapshot.messages.insert(
            0,
            MessageView {
                id: "display-history-limit".into(),
                kind: "notice".into(),
                text: "Earlier display content omitted by size limit.".into(),
                truncated: true,
            },
        );
    }
}

/// Called after request-context hydration; failures must never fail the Cursor run.
pub(crate) async fn record_request(
    store: &Store,
    id: &str,
    request: &pb::AgentRunRequest,
    context: &pb::RequestContext,
    model: &str,
) -> Result<()> {
    if request.subagent_type_name.is_some() {
        return Ok(());
    }
    let roots = context
        .env
        .as_ref()
        .map(|env| env.workspace_paths.clone())
        .unwrap_or_default();
    let (roots, roots_known) = if roots.len() > 64
        || roots
            .iter()
            .any(|root| root.len() > 4096 || workspace_key(root).is_none())
    {
        (Vec::new(), false)
    } else {
        (roots, true)
    };
    let user = request
        .action
        .as_ref()
        .and_then(|a| a.action.as_ref())
        .and_then(|a| match a {
            pb::conversation_action::Action::UserMessageAction(a) => a.user_message.as_ref(),
            _ => None,
        });
    let mut snapshot = store
        .remote_load_session(id)
        .await?
        .unwrap_or_else(|| SessionSnapshot {
            summary: SessionSummary {
                id: id.into(),
                title: user
                    .map(|u| bounded(&u.text, 160).0)
                    .unwrap_or_else(|| "Cursor session".into()),
                workspaces: Vec::new(),
                model: model.into(),
                updated_at_ms: now_ms(),
                status: "unavailable".into(),
                controllable: false,
            },
            seq: 0,
            epoch: String::new(),
            target_ref: None,
            messages: Vec::new(),
            questions: Vec::new(),
            synchronized: false,
        });
    // The background projector commits independently; retry against the latest row instead of
    // overwriting a newer sequence, and never widen the authorized workspace set.
    for attempt in 0..4 {
        merge_request(&mut snapshot, &roots, roots_known, model, user);
        match store.remote_save_snapshot(&mut snapshot).await {
            Ok(()) => return Ok(()),
            Err(error) if attempt < 3 => {
                tracing::debug!(%error, "retrying remote presentation metadata merge");
                let Some(latest) = store.remote_load_session(id).await? else {
                    continue;
                };
                snapshot = latest;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn merge_request(
    snapshot: &mut SessionSnapshot,
    roots: &[String],
    roots_known: bool,
    model: &str,
    user: Option<&pb::UserMessage>,
) {
    snapshot.summary.workspaces = if !roots_known {
        Vec::new()
    } else {
        let mut all_roots = snapshot.summary.workspaces.clone();
        all_roots.extend_from_slice(roots);
        all_roots.sort();
        all_roots.dedup();
        all_roots
    };
    snapshot.summary.model = model.into();
    snapshot.summary.updated_at_ms = now_ms();
    snapshot.summary.controllable = false;
    snapshot.target_ref = None;
    snapshot.questions.clear();
    if let Some(user) = user {
        let message_id = format!("user:{}", user.message_id);
        if !snapshot.messages.iter().any(|m| m.id == message_id) {
            let (text, truncated) = bounded(&user.text, 64 * 1024);
            snapshot.messages.push(MessageView {
                id: message_id,
                kind: "user".into(),
                text,
                truncated,
            });
        }
    }
    let mut omitted = false;
    while snapshot.messages.len() > 127
        || snapshot
            .messages
            .iter()
            .map(|m| m.text.len())
            .sum::<usize>()
            > 256 * 1024 - 256
    {
        snapshot.messages.remove(0);
        omitted = true;
    }
    if omitted {
        snapshot.messages.insert(
            0,
            MessageView {
                id: "display-history-limit".into(),
                kind: "notice".into(),
                text: "Earlier display content omitted by size limit.".into(),
                truncated: true,
            },
        );
    }
}
