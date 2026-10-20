//! Verifies durable mobile remote projections and command idempotency.
#[path = "support/fixtures.rs"]
mod fixtures;

use std::borrow::Cow;

use cursor_server::{
    error::Error,
    model::ConversationId,
    remote::{CommandReceipt, RemoteDevice, RemoteSettings, SessionSnapshot, SessionSummary},
};

fn snapshot(id: &str, status: &str) -> SessionSnapshot {
    SessionSnapshot {
        summary: SessionSummary {
            id: id.into(),
            title: "Remote session".into(),
            workspaces: vec![r"D:\work\alpha".into(), r"D:\work\beta".into()],
            model: "test-model".into(),
            updated_at_ms: 100,
            status: status.into(),
            controllable: true,
        },
        seq: 0,
        epoch: "epoch-1".into(),
        target_ref: Some("opaque-target".into()),
        messages: Vec::new(),
        questions: Vec::new(),
        synchronized: true,
    }
}

async fn insert_device(store: &cursor_server::store::Store, id: &str, hash: &[u8]) {
    store
        .remote_insert_device(
            &RemoteDevice {
                id: id.into(),
                name: "Phone".into(),
                created_at_ms: 10,
                last_seen_at_ms: None,
                revoked_at_ms: None,
            },
            hash,
            "csrf-token",
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn settings_default_and_round_trip() {
    let (_directory, store) = fixtures::temp_store().await;
    assert_eq!(
        store.remote_settings().await.unwrap(),
        RemoteSettings::default()
    );
    let settings = RemoteSettings {
        enabled: true,
        port: 43128,
        public_origin: "https://host.example".into(),
        allowed_workspaces: vec![r"D:\work\alpha".into()],
    };
    store.set_remote_settings(&settings).await.unwrap();
    assert_eq!(store.remote_settings().await.unwrap(), settings);
}

#[tokio::test]
async fn device_auth_uses_hash_is_rate_limited_and_revocation_is_immediate() {
    let (_directory, store) = fixtures::temp_store().await;
    let hash = [7_u8; 32];
    insert_device(&store, "phone-1", &hash).await;
    assert!(store
        .remote_auth_device(&[8_u8; 32])
        .await
        .unwrap()
        .is_none());
    let (first, csrf) = store.remote_auth_device(&hash).await.unwrap().unwrap();
    assert_eq!(csrf, "csrf-token");
    let first_seen = first.last_seen_at_ms.unwrap();
    let (second, _) = store.remote_auth_device(&hash).await.unwrap().unwrap();
    assert_eq!(second.last_seen_at_ms, Some(first_seen));
    store.remote_revoke_device("phone-1").await.unwrap();
    assert!(store.remote_auth_device(&hash).await.unwrap().is_none());
}

#[tokio::test]
async fn snapshot_sequence_is_cas_transactional_and_requires_conversation() {
    let (_directory, store) = fixtures::temp_store().await;
    let mut missing = snapshot("missing", "running");
    assert!(store.remote_save_snapshot(&mut missing).await.is_err());
    assert_eq!(missing.seq, 0);
    store
        .ensure_conversation(&ConversationId("conversation-1".into()))
        .await
        .unwrap();
    let mut first = snapshot("conversation-1", "running");
    store.remote_save_snapshot(&mut first).await.unwrap();
    assert_eq!(first.seq, 1);
    let mut stale = first.clone();
    stale.seq = 0;
    stale.summary.workspaces = vec![r"D:\stale-root".into()];
    assert!(matches!(
        store.remote_save_snapshot(&mut stale).await,
        Err(Error::Store(_))
    ));
    assert_eq!(
        store.remote_projects().await.unwrap(),
        vec![r"D:\work\alpha", r"D:\work\beta"]
    );
    first.summary.title = "Updated".into();
    store.remote_save_snapshot(&mut first).await.unwrap();
    assert_eq!(first.seq, 2);
    assert_eq!(
        store.remote_load_session("conversation-1").await.unwrap(),
        Some(first.clone())
    );
}

#[tokio::test]
async fn command_acceptance_is_idempotent_and_restart_marks_uncertain_state() {
    let (_directory, store) = fixtures::temp_store().await;
    store
        .ensure_conversation(&ConversationId("conversation-1".into()))
        .await
        .unwrap();
    let mut running = snapshot("conversation-1", "running");
    store.remote_save_snapshot(&mut running).await.unwrap();
    insert_device(&store, "phone-1", &[9_u8; 32]).await;
    let (accepted, is_new) = store
        .remote_accept_command("phone-1", "conversation-1", "command-1", "hash-a")
        .await
        .unwrap();
    assert!(is_new);
    let (same, is_new) = store
        .remote_accept_command("phone-1", "conversation-1", "command-1", "hash-a")
        .await
        .unwrap();
    assert!(!is_new);
    assert_eq!(same, accepted);
    assert!(matches!(
        store
            .remote_accept_command("phone-1", "conversation-1", "command-1", "hash-b")
            .await,
        Err(Error::Protocol(_))
    ));
    store.remote_restart().await.unwrap();
    assert_eq!(
        store
            .remote_command_receipt("phone-1", "command-1")
            .await
            .unwrap(),
        Some(CommandReceipt {
            command_id: "command-1".into(),
            status: "outcome_unknown".into(),
            reason: Some("server_restarted".into())
        })
    );
    let restarted = store
        .remote_load_session("conversation-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restarted.summary.status, "unavailable");
    assert_eq!(restarted.seq, 2);
    assert!(!restarted.summary.controllable);
    assert_eq!(restarted.target_ref, None);
    assert!(restarted.questions.is_empty());
    assert!(!restarted.synchronized);
}

#[tokio::test]
async fn command_completion_round_trips_receipt() {
    let (_directory, store) = fixtures::temp_store().await;
    store
        .ensure_conversation(&ConversationId("conversation-1".into()))
        .await
        .unwrap();
    let mut session = snapshot("conversation-1", "completed");
    store.remote_save_snapshot(&mut session).await.unwrap();
    insert_device(&store, "phone-1", &[1_u8; 32]).await;
    store
        .remote_accept_command("phone-1", "conversation-1", "command-1", "hash")
        .await
        .unwrap();
    store
        .remote_finish_command("phone-1", "command-1", "rejected", Some("run_ended"))
        .await
        .unwrap();
    assert_eq!(
        store
            .remote_command_receipt("phone-1", "command-1")
            .await
            .unwrap()
            .unwrap()
            .status,
        "rejected"
    );
}

#[tokio::test]
async fn restart_clears_controls_for_completed_snapshots_and_emits_event() {
    let (_directory, store) = fixtures::temp_store().await;
    store
        .ensure_conversation(&ConversationId("completed-session".into()))
        .await
        .unwrap();
    let mut completed = snapshot("completed-session", "completed");
    store.remote_save_snapshot(&mut completed).await.unwrap();
    store.remote_restart().await.unwrap();
    let restarted = store
        .remote_load_session("completed-session")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restarted.summary.status, "completed");
    assert_eq!(restarted.seq, 2);
    assert!(!restarted.summary.controllable);
    assert_eq!(restarted.target_ref, None);
    assert!(!restarted.synchronized);
}

#[tokio::test]
async fn list_sessions_clamps_zero_and_large_limits() {
    let (_directory, store) = fixtures::temp_store().await;
    for index in 0..55 {
        let id = format!("session-{index:02}");
        store
            .ensure_conversation(&ConversationId(id.clone()))
            .await
            .unwrap();
        let mut value = snapshot(&id, "completed");
        value.summary.updated_at_ms = index;
        store.remote_save_snapshot(&mut value).await.unwrap();
    }
    assert_eq!(store.remote_list_sessions(None, 0).await.unwrap().len(), 1);
    assert_eq!(
        store.remote_list_sessions(None, 500).await.unwrap().len(),
        50
    );
}

#[tokio::test]
async fn migration_constraints_retention_and_statistics_clear_are_isolated() {
    let (_directory, store) = fixtures::temp_store().await;
    store
        .ensure_conversation(&ConversationId("conversation-1".into()))
        .await
        .unwrap();
    let mut session = snapshot("conversation-1", "completed");
    store.remote_save_snapshot(&mut session).await.unwrap();
    insert_device(&store, "phone-1", &[3_u8; 32]).await;
    let bad = RemoteDevice {
        id: "bad".into(),
        name: "Bad".into(),
        created_at_ms: 1,
        last_seen_at_ms: None,
        revoked_at_ms: None,
    };
    assert!(store
        .remote_insert_device(&bad, &[1_u8; 31], "csrf")
        .await
        .is_err());
    let duplicate = RemoteDevice {
        id: "duplicate".into(),
        ..bad.clone()
    };
    assert!(store
        .remote_insert_device(&duplicate, &[3_u8; 32], "csrf")
        .await
        .is_err());
    assert!(store
        .remote_accept_command("missing", "conversation-1", "bad-device", "hash")
        .await
        .is_err());
    assert!(store
        .remote_accept_command("phone-1", "missing", "bad-session", "hash")
        .await
        .is_err());

    let payload = serde_json::to_string(&session).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let mut tx = store.pool().begin().await.unwrap();
    for seq in 2_i64..=5_001 {
        sqlx::query("INSERT INTO remote_events(conversation_id, seq, payload_json, created_at_ms) VALUES (?, ?, ?, ?)")
            .bind("conversation-1").bind(seq).bind(&payload)
            .bind(if seq == 2 { now - 86_400_001 } else { now }).execute(&mut *tx).await.unwrap();
    }
    sqlx::query("UPDATE remote_sessions SET last_seq = 5001 WHERE conversation_id = ?")
        .bind("conversation-1")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    session.seq = 5_001;
    store.remote_save_snapshot(&mut session).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM remote_events WHERE conversation_id = ?")
            .bind("conversation-1")
            .fetch_one(store.pool())
            .await
            .unwrap();
    assert_eq!(count, 5_000);
    assert_eq!(
        store.remote_oldest_event("conversation-1").await.unwrap(),
        Some(3)
    );
    store.clear_all_statistics_storage().await.unwrap();
    assert!(store
        .remote_devices()
        .await
        .unwrap()
        .iter()
        .any(|d| d.id == "phone-1"));
    assert!(store
        .remote_load_session("conversation-1")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn database_at_0009_upgrades_without_losing_existing_rows() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("upgrade.db");
    let url = format!("sqlite://{}", database.display());
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&database)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    let all = sqlx::migrate!("./migrations");
    let through_0009 = sqlx::migrate::Migrator {
        migrations: Cow::Owned(all.iter().take(9).cloned().collect()),
        ..sqlx::migrate::Migrator::DEFAULT
    };
    through_0009.run(&pool).await.unwrap();
    sqlx::query("INSERT INTO service_settings(setting_key, value_json, updated_at_ms) VALUES ('preserved', '{\"value\":1}', 7)").execute(&pool).await.unwrap();
    pool.close().await;
    let store = cursor_server::store::Store::connect(&url).await.unwrap();
    let preserved: String = sqlx::query_scalar(
        "SELECT value_json FROM service_settings WHERE setting_key = 'preserved'",
    )
    .fetch_one(store.pool())
    .await
    .unwrap();
    assert_eq!(preserved, r#"{"value":1}"#);
    let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('remote_devices','remote_sessions','remote_events','remote_commands')").fetch_one(store.pool()).await.unwrap();
    assert_eq!(tables, 4);
}
