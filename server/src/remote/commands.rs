//! Durable command receipts, strict live targets, and no automatic replay.
use super::{auth, CommandInput, CommandKind, CommandReceipt, RemoteService};
use crate::cursor::{
    conversation::{RemoteAnswer, RemoteCommand, RemoteQuestion, RemoteResult, RemoteTarget},
    transport::TransportHandle,
};
use axum::http::{HeaderMap, StatusCode};
use std::collections::HashMap;

#[derive(Clone)]
pub(crate) struct LiveTarget {
    pub handle: TransportHandle,
    pub target: RemoteTarget,
    pub reference: String,
    pub questions: HashMap<String, RemoteQuestion>,
}
impl LiveTarget {
    pub fn new(handle: TransportHandle, target: RemoteTarget) -> Self {
        Self {
            handle,
            target,
            reference: auth::secret(),
            questions: HashMap::new(),
        }
    }
}
impl RemoteService {
    pub(crate) async fn command(
        &self,
        headers: &HeaderMap,
        id: &str,
        input: CommandInput,
    ) -> std::result::Result<CommandReceipt, StatusCode> {
        if input.command_id.is_empty()
            || input.command_id.len() > 128
            || input.target_ref.len() > 128
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        if let CommandKind::AppendText { text } = &input.kind {
            if text.trim().is_empty() || text.len() > 16 * 1024 {
                return Err(StatusCode::BAD_REQUEST);
            }
        }
        let payload = serde_json::to_string(&(id, &input)).map_err(|_| StatusCode::BAD_REQUEST)?;
        let digest = hex::encode(auth::hash(&payload));
        let _gate = self.inner.command_gate.lock().await;
        let (device, _) = self.authenticate(headers, true).await?;
        let snapshot = self.snapshot(id).await?;
        let slot = self
            .inner
            .command_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
        let (receipt, fresh) = self
            .inner
            .store
            .remote_accept_command(&device.id, id, &input.command_id, &digest)
            .await
            .map_err(|error| match error {
                crate::Error::Protocol(_) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            })?;
        if !fresh {
            return Ok(receipt);
        }
        let target = self
            .inner
            .targets
            .lock()
            .await
            .get(id)
            .cloned()
            .filter(|target| {
                target.reference == input.target_ref
                    && snapshot.target_ref.as_ref() == Some(&target.reference)
                    && snapshot.summary.controllable
                    && snapshot.synchronized
            });
        let Some(target) = target else {
            self.inner
                .store
                .remote_finish_command(
                    &device.id,
                    &input.command_id,
                    "rejected",
                    Some("stale_target"),
                )
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            return Ok(CommandReceipt {
                command_id: input.command_id,
                status: "rejected".into(),
                reason: Some("stale_target".into()),
            });
        };
        let active = self
            .inner
            .store
            .conversation(&crate::model::ConversationId::new(id))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if active
            .as_ref()
            .and_then(|conversation| conversation.active_run_id.as_ref())
            != Some(&target.target.run_id)
        {
            self.inner
                .store
                .remote_finish_command(&device.id, &input.command_id, "rejected", Some("run_ended"))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            return Ok(CommandReceipt {
                command_id: input.command_id,
                status: "rejected".into(),
                reason: Some("run_ended".into()),
            });
        }
        let command = match input.kind {
            CommandKind::AppendText { text } => RemoteCommand::AppendText {
                command_id: format!("{}:{}", device.id, input.command_id),
                text,
            },
            CommandKind::Stop => RemoteCommand::Stop,
            CommandKind::AnswerQuestion {
                question_ref,
                answers,
            } => {
                let Some(question) = target.questions.get(&question_ref) else {
                    self.inner
                        .store
                        .remote_finish_command(
                            &device.id,
                            &input.command_id,
                            "rejected",
                            Some("question_changed"),
                        )
                        .await
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                    return Ok(CommandReceipt {
                        command_id: input.command_id,
                        status: "rejected".into(),
                        reason: Some("question_changed".into()),
                    });
                };
                RemoteCommand::AnswerQuestion {
                    interaction_id: question.interaction_id,
                    tool_call_id: question.tool_call_id.clone(),
                    answers: answers
                        .into_iter()
                        .map(|answer| RemoteAnswer {
                            question_id: answer.question_id,
                            selected_option_ids: answer.selected_option_ids,
                            freeform_text: answer.freeform_text,
                        })
                        .collect(),
                }
            }
        };
        let service = self.clone();
        let device_id = device.id;
        let command_id = input.command_id;
        // Accepted work outlives the HTTP request, but never the original target.
        tokio::spawn(async move {
            let _slot = slot;
            let gate = service.inner.command_gate.lock().await;
            let enabled = service.inner.settings.read().await.enabled;
            let valid_device = service
                .inner
                .store
                .remote_devices()
                .await
                .is_ok_and(|devices| {
                    devices
                        .iter()
                        .any(|device| device.id == device_id && device.revoked_at_ms.is_none())
                });
            let allowed = enabled
                && valid_device
                && service.observed_session(id_from_target(&target)).await
                && service.snapshot(id_from_target(&target)).await.is_ok();
            // The authorization decision is made once at dispatch; cancellation/revocation never waits for model completion.
            drop(gate);
            let stopping = matches!(command, RemoteCommand::Stop);
            let result = if enabled && valid_device && allowed {
                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    target.handle.remote_command(target.target.clone(), command),
                )
                .await
                .ok()
            } else {
                Some(RemoteResult::StaleTarget)
            };
            let result = if stopping && result == Some(RemoteResult::Applied) {
                let ended = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    loop {
                        if target.handle.remote_display().ended {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                })
                .await;
                if ended.is_ok() {
                    result
                } else {
                    None
                }
            } else {
                result
            };
            let (status, reason) = match result {
                Some(RemoteResult::Applied | RemoteResult::Duplicate) => ("applied", None),
                Some(RemoteResult::InvalidInput) => ("rejected", Some("invalid_input")),
                Some(RemoteResult::AlreadyResolved) => ("rejected", Some("already_resolved")),
                Some(RemoteResult::RunEnded | RemoteResult::RunClosing) => {
                    ("rejected", Some("run_ended"))
                }
                Some(RemoteResult::StaleTarget) => ("rejected", Some("stale_target")),
                None => (
                    "outcome_unknown",
                    Some("execution_confirmation_unavailable"),
                ),
            };
            if let Err(error) = service
                .inner
                .store
                .remote_finish_command(&device_id, &command_id, status, reason)
                .await
            {
                tracing::warn!(%error, "remote command receipt persistence failed");
            }
            service.inner.changed.notify_waiters();
        });
        Ok(receipt)
    }
}
fn id_from_target(target: &LiveTarget) -> &str {
    target.handle.conversation_id().unwrap_or_default()
}
