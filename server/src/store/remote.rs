//! Persists the bounded mobile remote projection, devices, and command receipts.
use std::collections::BTreeSet;

use sqlx::{Row, Sqlite, Transaction};

use crate::{
    remote::{CommandReceipt, RemoteDevice, RemoteSettings, SessionSnapshot},
    Error, Result,
};

use super::{now_ms, Store};

const REMOTE_SETTINGS_KEY: &str = "mobile_remote";
const EVENT_RETENTION_MS: i64 = 24 * 60 * 60 * 1_000;
const MAX_EVENTS_PER_SESSION: i64 = 5_000;
const LAST_SEEN_WRITE_INTERVAL_MS: i64 = 60_000;
const MAX_SESSION_LIST_LIMIT: u32 = 50;

impl Store {
    pub async fn remote_settings(&self) -> Result<RemoteSettings> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value_json FROM service_settings WHERE setting_key = ?")
                .bind(REMOTE_SETTINGS_KEY)
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
            .map(|settings| settings.unwrap_or_default())
    }

    pub async fn set_remote_settings(&self, settings: &RemoteSettings) -> Result<()> {
        let value = serde_json::to_string(settings)?;
        let _write = self.writes.lock().await;
        sqlx::query(
            "INSERT INTO service_settings(setting_key, value_json, updated_at_ms)
             VALUES (?, ?, ?)
             ON CONFLICT(setting_key) DO UPDATE SET
                 value_json = excluded.value_json,
                 updated_at_ms = excluded.updated_at_ms",
        )
        .bind(REMOTE_SETTINGS_KEY)
        .bind(value)
        .bind(now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remote_insert_device(
        &self,
        device: &RemoteDevice,
        token_hash: &[u8],
        csrf: &str,
    ) -> Result<()> {
        let _write = self.writes.lock().await;
        sqlx::query(
            "INSERT INTO remote_devices
             (device_id, display_name, token_hash, csrf_token, created_at_ms, last_seen_at_ms, revoked_at_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&device.id)
        .bind(&device.name)
        .bind(token_hash)
        .bind(csrf)
        .bind(device.created_at_ms)
        .bind(device.last_seen_at_ms)
        .bind(device.revoked_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remote_auth_device(&self, hash: &[u8]) -> Result<Option<(RemoteDevice, String)>> {
        let row = sqlx::query(
            "SELECT device_id, display_name, created_at_ms, last_seen_at_ms, revoked_at_ms, csrf_token
             FROM remote_devices WHERE token_hash = ? AND revoked_at_ms IS NULL",
        )
        .bind(hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut device = RemoteDevice {
            id: row.get(0),
            name: row.get(1),
            created_at_ms: row.get(2),
            last_seen_at_ms: row.get(3),
            revoked_at_ms: row.get(4),
        };
        let csrf = row.get(5);
        let seen = now_ms();
        if device
            .last_seen_at_ms
            .is_none_or(|last_seen| seen - last_seen >= LAST_SEEN_WRITE_INTERVAL_MS)
        {
            let _write = self.writes.lock().await;
            let updated = sqlx::query(
                "UPDATE remote_devices SET last_seen_at_ms = ?
                 WHERE device_id = ? AND revoked_at_ms IS NULL
                   AND (last_seen_at_ms IS NULL OR last_seen_at_ms <= ?)",
            )
            .bind(seen)
            .bind(&device.id)
            .bind(seen - LAST_SEEN_WRITE_INTERVAL_MS)
            .execute(&self.pool)
            .await?
            .rows_affected();
            if updated == 1 {
                device.last_seen_at_ms = Some(seen);
            } else {
                let current: Option<Option<i64>> = sqlx::query_scalar(
                    "SELECT last_seen_at_ms FROM remote_devices
                     WHERE device_id = ? AND revoked_at_ms IS NULL",
                )
                .bind(&device.id)
                .fetch_optional(&self.pool)
                .await?;
                let Some(current) = current else {
                    return Ok(None);
                };
                device.last_seen_at_ms = current;
            }
        }
        Ok(Some((device, csrf)))
    }

    pub async fn remote_devices(&self) -> Result<Vec<RemoteDevice>> {
        let rows = sqlx::query(
            "SELECT device_id, display_name, created_at_ms, last_seen_at_ms, revoked_at_ms
             FROM remote_devices ORDER BY created_at_ms DESC, device_id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| RemoteDevice {
                id: row.get(0),
                name: row.get(1),
                created_at_ms: row.get(2),
                last_seen_at_ms: row.get(3),
                revoked_at_ms: row.get(4),
            })
            .collect())
    }

    pub async fn remote_revoke_device(&self, id: &str) -> Result<()> {
        let _write = self.writes.lock().await;
        sqlx::query(
            "UPDATE remote_devices SET revoked_at_ms = COALESCE(revoked_at_ms, ?)
             WHERE device_id = ?",
        )
        .bind(now_ms())
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remote_load_session(&self, id: &str) -> Result<Option<SessionSnapshot>> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT snapshot_json FROM remote_sessions WHERE conversation_id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    pub async fn remote_list_sessions(
        &self,
        before: Option<i64>,
        limit: u32,
    ) -> Result<Vec<SessionSnapshot>> {
        let limit = limit.clamp(1, MAX_SESSION_LIST_LIMIT);
        let rows = sqlx::query(
            "SELECT snapshot_json FROM remote_sessions
             WHERE (? IS NULL OR updated_at_ms < ?)
             ORDER BY updated_at_ms DESC, conversation_id ASC
             LIMIT ?",
        )
        .bind(before)
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_str(row.get::<&str, _>(0)).map_err(Into::into))
            .collect()
    }

    pub async fn remote_projects(&self) -> Result<Vec<String>> {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT workspaces_json FROM remote_sessions ORDER BY conversation_id ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut projects = BTreeSet::new();
        for value in rows {
            projects.extend(serde_json::from_str::<Vec<String>>(&value)?);
        }
        Ok(projects.into_iter().collect())
    }

    pub async fn remote_save_snapshot(&self, snapshot: &mut SessionSnapshot) -> Result<()> {
        let _write = self.writes.lock().await;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT last_seq FROM remote_sessions WHERE conversation_id = ?")
                .bind(&snapshot.summary.id)
                .fetch_optional(&mut *tx)
                .await?;
        match existing {
            Some(current) if current != snapshot.seq => {
                return Err(Error::Store(format!(
                    "stale remote snapshot for {}: expected seq {current}, received {}",
                    snapshot.summary.id, snapshot.seq
                )));
            }
            None if snapshot.seq != 0 => {
                return Err(Error::Store(format!(
                    "new remote snapshot for {} must start at seq 0, received {}",
                    snapshot.summary.id, snapshot.seq
                )));
            }
            _ => {}
        }
        let mut persisted = snapshot.clone();
        Self::remote_save_snapshot_tx(&mut tx, &mut persisted).await?;
        tx.commit().await?;
        snapshot.seq = persisted.seq;
        Ok(())
    }

    async fn remote_save_snapshot_tx(
        tx: &mut Transaction<'_, Sqlite>,
        snapshot: &mut SessionSnapshot,
    ) -> Result<()> {
        let next_seq = snapshot.seq + 1;
        let mut persisted = snapshot.clone();
        persisted.seq = next_seq;
        let snapshot_json = serde_json::to_string(&persisted)?;
        let workspaces_json = serde_json::to_string(&persisted.summary.workspaces)?;
        sqlx::query(
            "INSERT INTO remote_sessions
             (conversation_id, title, workspaces_json, model, snapshot_json, last_seq, updated_at_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(conversation_id) DO UPDATE SET
                 title = excluded.title,
                 workspaces_json = excluded.workspaces_json,
                 model = excluded.model,
                 snapshot_json = excluded.snapshot_json,
                 last_seq = excluded.last_seq,
                 updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&persisted.summary.id)
        .bind(&persisted.summary.title)
        .bind(workspaces_json)
        .bind(&persisted.summary.model)
        .bind(&snapshot_json)
        .bind(next_seq)
        .bind(persisted.summary.updated_at_ms)
        .execute(&mut **tx)
        .await?;
        let created_at_ms = now_ms();
        sqlx::query(
            "INSERT INTO remote_events(conversation_id, seq, payload_json, created_at_ms)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&persisted.summary.id)
        .bind(next_seq)
        .bind(&snapshot_json)
        .bind(created_at_ms)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "DELETE FROM remote_events
             WHERE conversation_id = ? AND created_at_ms < ?",
        )
        .bind(&persisted.summary.id)
        .bind(created_at_ms - EVENT_RETENTION_MS)
        .execute(&mut **tx)
        .await?;
        sqlx::query(
            "DELETE FROM remote_events
             WHERE conversation_id = ? AND seq <= (
                 SELECT COALESCE(MAX(seq), 0) - ? FROM remote_events WHERE conversation_id = ?
             )",
        )
        .bind(&persisted.summary.id)
        .bind(MAX_EVENTS_PER_SESSION)
        .bind(&persisted.summary.id)
        .execute(&mut **tx)
        .await?;
        snapshot.seq = next_seq;
        Ok(())
    }

    pub async fn remote_oldest_event(&self, id: &str) -> Result<Option<i64>> {
        Ok(
            sqlx::query_scalar("SELECT MIN(seq) FROM remote_events WHERE conversation_id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn remote_restart(&self) -> Result<()> {
        let _write = self.writes.lock().await;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        sqlx::query(
            "UPDATE remote_commands
             SET status = 'outcome_unknown', reason = 'server_restarted', completed_at_ms = ?
             WHERE status = 'accepted'",
        )
        .bind(now_ms())
        .execute(&mut *tx)
        .await?;

        let rows = sqlx::query(
            "SELECT snapshot_json, last_seq
             FROM remote_sessions ORDER BY conversation_id",
        )
        .fetch_all(&mut *tx)
        .await?;
        for row in rows {
            let mut snapshot: SessionSnapshot = serde_json::from_str(row.get(0))?;
            snapshot.seq = row.get(1);
            snapshot.summary.controllable = false;
            snapshot.target_ref = None;
            snapshot.questions.clear();
            snapshot.synchronized = false;
            if matches!(
                snapshot.summary.status.as_str(),
                "running" | "waiting_question"
            ) {
                snapshot.summary.status = "unavailable".into();
            }
            Self::remote_save_snapshot_tx(&mut tx, &mut snapshot).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn remote_accept_command(
        &self,
        device_id: &str,
        conversation_id: &str,
        command_id: &str,
        request_hash: &str,
    ) -> Result<(CommandReceipt, bool)> {
        let _write = self.writes.lock().await;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let existing = sqlx::query(
            "SELECT request_hash, status, reason FROM remote_commands
             WHERE device_id = ? AND command_id = ?",
        )
        .bind(device_id)
        .bind(command_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(row) = existing {
            let existing_hash: String = row.get(0);
            if existing_hash != request_hash {
                return Err(Error::Protocol(format!(
                    "command id {command_id} was reused with different content"
                )));
            }
            let receipt = CommandReceipt {
                command_id: command_id.into(),
                status: row.get(1),
                reason: row.get(2),
            };
            tx.commit().await?;
            return Ok((receipt, false));
        }

        sqlx::query(
            "INSERT INTO remote_commands
             (device_id, command_id, conversation_id, request_hash, status, reason, created_at_ms, completed_at_ms)
             VALUES (?, ?, ?, ?, 'accepted', NULL, ?, NULL)",
        )
        .bind(device_id)
        .bind(command_id)
        .bind(conversation_id)
        .bind(request_hash)
        .bind(now_ms())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((
            CommandReceipt {
                command_id: command_id.into(),
                status: "accepted".into(),
                reason: None,
            },
            true,
        ))
    }

    pub async fn remote_finish_command(
        &self,
        device_id: &str,
        command_id: &str,
        status: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        if !matches!(status, "applied" | "rejected" | "outcome_unknown") {
            return Err(Error::Protocol(format!(
                "invalid completed command status: {status}"
            )));
        }
        let _write = self.writes.lock().await;
        let changed = sqlx::query(
            "UPDATE remote_commands SET status = ?, reason = ?, completed_at_ms = ?
             WHERE device_id = ? AND command_id = ?",
        )
        .bind(status)
        .bind(reason)
        .bind(now_ms())
        .bind(device_id)
        .bind(command_id)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed == 0 {
            return Err(Error::Store(format!(
                "remote command not found: {device_id}/{command_id}"
            )));
        }
        Ok(())
    }

    pub async fn remote_command_receipt(
        &self,
        device_id: &str,
        command_id: &str,
    ) -> Result<Option<CommandReceipt>> {
        let row = sqlx::query(
            "SELECT status, reason FROM remote_commands WHERE device_id = ? AND command_id = ?",
        )
        .bind(device_id)
        .bind(command_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|row| CommandReceipt {
            command_id: command_id.into(),
            status: row.get(0),
            reason: row.get(1),
        }))
    }
}
