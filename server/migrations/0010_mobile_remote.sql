-- Mobile views are disposable projections; execution history remains in the existing tables.
CREATE TABLE remote_devices (
    device_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    token_hash BLOB NOT NULL UNIQUE CHECK(length(token_hash) = 32),
    csrf_token TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    last_seen_at_ms INTEGER,
    revoked_at_ms INTEGER
);

CREATE TABLE remote_sessions (
    conversation_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    workspaces_json TEXT NOT NULL CHECK(json_valid(workspaces_json)),
    model TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    last_seq INTEGER NOT NULL DEFAULT 0 CHECK(last_seq >= 0),
    updated_at_ms INTEGER NOT NULL,
    FOREIGN KEY(conversation_id) REFERENCES conversations(conversation_id) ON DELETE CASCADE
);
CREATE INDEX remote_sessions_updated ON remote_sessions(updated_at_ms DESC, conversation_id);

CREATE TABLE remote_events (
    conversation_id TEXT NOT NULL,
    seq INTEGER NOT NULL CHECK(seq > 0),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY(conversation_id, seq),
    FOREIGN KEY(conversation_id) REFERENCES remote_sessions(conversation_id) ON DELETE CASCADE
);

CREATE TABLE remote_commands (
    device_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('accepted','applied','rejected','outcome_unknown')),
    reason TEXT,
    created_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    PRIMARY KEY(device_id, command_id),
    FOREIGN KEY(device_id) REFERENCES remote_devices(device_id),
    FOREIGN KEY(conversation_id) REFERENCES remote_sessions(conversation_id)
);
