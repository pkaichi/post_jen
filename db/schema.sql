PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS job_definitions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    definition_path TEXT NOT NULL,
    definition_hash TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (job_id)
);

CREATE TABLE IF NOT EXISTS job_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_definition_id INTEGER NOT NULL,
    job_id TEXT NOT NULL,
    job_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN (
            'created',
            'queued',
            'running',
            'cancel_requested',
            'success',
            'failed',
            'timed_out',
            'canceled'
        )
    ),
    trigger_type TEXT NOT NULL,
    triggered_by TEXT,
    definition_path TEXT NOT NULL,
    definition_hash TEXT NOT NULL,
    working_dir TEXT NOT NULL,
    queued_at TEXT,
    started_at TEXT,
    finished_at TEXT,
    cancel_requested_at TEXT,
    rerun_of_job_run_id INTEGER,
    failure_reason TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (job_definition_id) REFERENCES job_definitions(id),
    FOREIGN KEY (rerun_of_job_run_id) REFERENCES job_runs(id)
);

CREATE TABLE IF NOT EXISTS agents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    name TEXT NOT NULL,
    hostname TEXT NOT NULL,
    labels_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'online' CHECK (status IN ('online', 'offline')),
    token_hash TEXT NOT NULL,
    last_heartbeat_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    registered_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (agent_id)
);

CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status);

CREATE TABLE IF NOT EXISTS node_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_run_id INTEGER NOT NULL,
    node_id TEXT NOT NULL,
    node_name TEXT,
    status TEXT NOT NULL CHECK (
        status IN (
            'pending',
            'queued',
            'running',
            'success',
            'failed',
            'timed_out',
            'cancel_requested',
            'canceled',
            'skipped'
        )
    ),
    program TEXT NOT NULL,
    args_json TEXT NOT NULL,
    working_dir TEXT NOT NULL,
    env_json TEXT,
    timeout_sec INTEGER NOT NULL CHECK (timeout_sec > 0),
    retry_count INTEGER NOT NULL DEFAULT 0 CHECK (retry_count >= 0),
    exit_code INTEGER,
    started_at TEXT,
    finished_at TEXT,
    cancel_requested_at TEXT,
    failure_reason TEXT,
    target_json TEXT,
    assigned_agent_id TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (job_run_id) REFERENCES job_runs(id),
    UNIQUE (job_run_id, node_id)
);

CREATE TABLE IF NOT EXISTS run_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_run_id INTEGER NOT NULL,
    node_run_id INTEGER,
    scope TEXT NOT NULL CHECK (scope IN ('job', 'node')),
    event_type TEXT NOT NULL,
    from_status TEXT,
    to_status TEXT,
    message TEXT,
    occurred_at TEXT NOT NULL,
    FOREIGN KEY (job_run_id) REFERENCES job_runs(id),
    FOREIGN KEY (node_run_id) REFERENCES node_runs(id)
);

CREATE TABLE IF NOT EXISTS run_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_run_id INTEGER NOT NULL,
    node_run_id INTEGER,
    stream TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr', 'system')),
    sequence INTEGER NOT NULL,
    content TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    FOREIGN KEY (job_run_id) REFERENCES job_runs(id),
    FOREIGN KEY (node_run_id) REFERENCES node_runs(id)
);

CREATE TABLE IF NOT EXISTS run_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_run_id INTEGER NOT NULL,
    node_run_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    resolved_path TEXT NOT NULL,
    required INTEGER NOT NULL CHECK (required IN (0, 1)),
    exists_flag INTEGER NOT NULL CHECK (exists_flag IN (0, 1)),
    size_bytes INTEGER,
    checked_at TEXT NOT NULL,
    FOREIGN KEY (job_run_id) REFERENCES job_runs(id),
    FOREIGN KEY (node_run_id) REFERENCES node_runs(id)
);

CREATE INDEX IF NOT EXISTS idx_job_runs_job_definition_id_created_at
    ON job_runs(job_definition_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_job_runs_status_created_at
    ON job_runs(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_node_runs_job_run_id
    ON node_runs(job_run_id);

CREATE INDEX IF NOT EXISTS idx_node_runs_job_run_id_node_id
    ON node_runs(job_run_id, node_id);

CREATE INDEX IF NOT EXISTS idx_run_events_job_run_id_occurred_at
    ON run_events(job_run_id, occurred_at);

CREATE INDEX IF NOT EXISTS idx_run_logs_job_run_id_sequence
    ON run_logs(job_run_id, sequence);

CREATE INDEX IF NOT EXISTS idx_run_logs_node_run_id_sequence
    ON run_logs(node_run_id, sequence);

CREATE INDEX IF NOT EXISTS idx_run_artifacts_node_run_id
    ON run_artifacts(node_run_id);
