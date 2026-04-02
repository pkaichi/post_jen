use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, sse::{Event, KeepAlive, Sse}},
    routing::{get, post},
};
use crate::definition::JobDefinition;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::{convert::Infallible, fs, path::PathBuf, time::Duration};
use tokio::time::interval;
use tokio_stream::{StreamExt, wrappers::IntervalStream};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub artifacts_dir: PathBuf,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/jobs", get(list_jobs).post(register_job))
        .route("/api/jobs/:job_id", get(get_job))
        .route("/api/jobs/:job_id/runs", post(start_run))
        .route("/api/runs", get(list_runs))
        .route("/api/runs/:run_id", get(get_run))
        .route("/api/runs/:run_id/cancel", post(cancel_run))
        .route("/api/runs/:run_id/rerun", post(rerun_run))
        .route("/api/runs/:run_id/logs", get(get_run_logs))
        .route("/api/runs/:run_id/events", get(get_run_events))
        .route("/api/runs/:run_id/stream", get(stream_run))
        // Agent management API
        .route("/api/agents", get(list_agents).post(register_agent))
        .route("/api/agents/:agent_id", get(get_agent).delete(delete_agent))
        // Agent worker API
        .route("/api/agent/task", get(poll_task))
        .route("/api/agent/result", post(report_result))
        .route("/api/agent/logs", post(report_logs))
        .route("/api/agent/heartbeat", post(heartbeat))
        .route("/api/agent/artifacts", post(upload_artifact))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

#[derive(Debug, Serialize, FromRow)]
struct JobSummary {
    job_id: String,
    name: String,
    description: Option<String>,
    definition_path: String,
    definition_hash: String,
    enabled: i64,
    updated_at: String,
}

async fn list_jobs(State(state): State<AppState>) -> Result<Json<Vec<JobSummary>>, ApiError> {
    let jobs = sqlx::query_as::<_, JobSummary>(
        r#"
        SELECT job_id, name, description, definition_path, definition_hash, enabled, updated_at
        FROM job_definitions
        ORDER BY job_id
        "#,
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(jobs))
}

#[derive(Debug, Deserialize)]
struct RegisterJobRequest {
    definition_path: String,
    enabled: Option<bool>,
}

#[derive(Debug, Serialize, FromRow)]
struct RegisterJobResponse {
    job_id: String,
    name: String,
    description: Option<String>,
    definition_path: String,
    definition_hash: String,
    enabled: i64,
    created_at: String,
    updated_at: String,
}

async fn register_job(
    State(state): State<AppState>,
    Json(payload): Json<RegisterJobRequest>,
) -> Result<(StatusCode, Json<RegisterJobResponse>), ApiError> {
    let definition_path = payload.definition_path.trim();
    if definition_path.is_empty() {
        return Err(ApiError::bad_request("definition_path must not be empty"));
    }

    let definition = JobDefinition::load(definition_path)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let definition_contents = fs::read(definition_path)
        .map_err(|error| ApiError::bad_request(format!("failed to read definition file: {error}")))?;
    let definition_hash = format!("{:x}", Sha256::digest(definition_contents));
    let enabled = if payload.enabled.unwrap_or(true) { 1_i64 } else { 0_i64 };

    let existing = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM job_definitions WHERE job_id = ?")
        .bind(&definition.id)
        .fetch_one(&state.pool)
        .await?;
    let status = if existing == 0 {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO job_definitions (
            job_id, name, description, definition_path, definition_hash, enabled
        )
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(job_id) DO UPDATE SET
            name = excluded.name,
            description = excluded.description,
            definition_path = excluded.definition_path,
            definition_hash = excluded.definition_hash,
            enabled = excluded.enabled,
            updated_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(&definition.id)
    .bind(&definition.name)
    .bind(definition.description.as_deref())
    .bind(definition_path)
    .bind(&definition_hash)
    .bind(enabled)
    .execute(&mut *tx)
    .await?;

    let response = sqlx::query_as::<_, RegisterJobResponse>(
        r#"
        SELECT job_id, name, description, definition_path, definition_hash, enabled, created_at, updated_at
        FROM job_definitions
        WHERE job_id = ?
        "#,
    )
    .bind(&definition.id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((status, Json(response)))
}

#[derive(Debug, Serialize, FromRow)]
struct JobDefinitionRow {
    job_id: String,
    name: String,
    description: Option<String>,
    definition_path: String,
    definition_hash: String,
    enabled: i64,
    created_at: String,
    updated_at: String,
}

async fn get_job(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> Result<Json<JobDefinitionRow>, ApiError> {
    let row = sqlx::query_as::<_, JobDefinitionRow>(
        r#"
        SELECT job_id, name, description, definition_path, definition_hash, enabled, created_at, updated_at
        FROM job_definitions
        WHERE job_id = ?
        "#,
    )
    .bind(job_id)
    .fetch_optional(&state.pool)
    .await?;

    match row {
        Some(job) => Ok(Json(job)),
        None => Err(ApiError::not_found("job not found")),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StartRunRequest {
    trigger_type: Option<String>,
    triggered_by: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
struct StartRunResponse {
    run_id: i64,
    status: String,
    queued_at: String,
}

async fn start_run(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
    Json(payload): Json<StartRunRequest>,
) -> Result<(StatusCode, Json<StartRunResponse>), ApiError> {
    let job = sqlx::query_as::<_, JobDefinitionForRun>(
        r#"
        SELECT id, job_id, name, definition_path, definition_hash, enabled
        FROM job_definitions
        WHERE job_id = ?
        "#,
    )
    .bind(&job_id)
    .fetch_optional(&state.pool)
    .await?;

    let job = match job {
        Some(job) => job,
        None => return Err(ApiError::not_found("job not found")),
    };

    if job.enabled == 0 {
        return Err(ApiError::conflict("job is disabled"));
    }

    let trigger_type = payload
        .trigger_type
        .unwrap_or_else(|| "manual".to_string())
        .trim()
        .to_string();
    if trigger_type.is_empty() {
        return Err(ApiError::bad_request("trigger_type must not be empty"));
    }

    let definition = JobDefinition::load(&job.definition_path)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let working_dir = definition.working_dir;
    let mut tx = state.pool.begin().await?;

    let result = sqlx::query(
        r#"
        INSERT INTO job_runs (
            job_definition_id, job_id, job_name, status, trigger_type, triggered_by,
            definition_path, definition_hash, working_dir, queued_at
        )
        VALUES (?, ?, ?, 'queued', ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(job.id)
    .bind(&job.job_id)
    .bind(&job.name)
    .bind(&trigger_type)
    .bind(payload.triggered_by.as_deref())
    .bind(&job.definition_path)
    .bind(&job.definition_hash)
    .bind(&working_dir)
    .execute(&mut *tx)
    .await?;

    let run_id = result.last_insert_rowid();

    sqlx::query(
        r#"
        INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
        VALUES (?, NULL, 'job', 'status_changed', NULL, 'queued', 'run created', CURRENT_TIMESTAMP)
        "#,
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;

    let response = sqlx::query_as::<_, StartRunResponse>(
        r#"
        SELECT id AS run_id, status, queued_at
        FROM job_runs
        WHERE id = ?
        "#,
    )
    .bind(run_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(response)))
}

#[derive(Debug, Deserialize)]
struct RunListQuery {
    job_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct RunSummary {
    id: i64,
    job_id: String,
    job_name: String,
    status: String,
    trigger_type: String,
    triggered_by: Option<String>,
    queued_at: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
}

async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<RunListQuery>,
) -> Result<Json<Vec<RunSummary>>, ApiError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = query.offset.unwrap_or(0).max(0);

    let runs = sqlx::query_as::<_, RunSummary>(
        r#"
        SELECT id, job_id, job_name, status, trigger_type, triggered_by, queued_at, started_at, finished_at
        FROM job_runs
        WHERE (?1 IS NULL OR job_id = ?1)
          AND (?2 IS NULL OR status = ?2)
        ORDER BY created_at DESC
        LIMIT ?3 OFFSET ?4
        "#,
    )
    .bind(query.job_id)
    .bind(query.status)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(runs))
}

#[derive(Debug, Serialize, FromRow)]
struct RunDetail {
    id: i64,
    job_id: String,
    job_name: String,
    status: String,
    trigger_type: String,
    triggered_by: Option<String>,
    definition_path: String,
    definition_hash: String,
    working_dir: String,
    queued_at: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    cancel_requested_at: Option<String>,
    rerun_of_job_run_id: Option<i64>,
    failure_reason: Option<String>,
    created_at: String,
}

async fn get_run(
    State(state): State<AppState>,
    Path(run_id): Path<i64>,
) -> Result<Json<RunDetail>, ApiError> {
    let run = sqlx::query_as::<_, RunDetail>(
        r#"
        SELECT id, job_id, job_name, status, trigger_type, triggered_by, definition_path, definition_hash,
               working_dir, queued_at, started_at, finished_at, cancel_requested_at,
               rerun_of_job_run_id, failure_reason, created_at
        FROM job_runs
        WHERE id = ?
        "#,
    )
    .bind(run_id)
    .fetch_optional(&state.pool)
    .await?;

    match run {
        Some(run) => Ok(Json(run)),
        None => Err(ApiError::not_found("run not found")),
    }
}

#[derive(Debug, Serialize, FromRow)]
struct CancelRunResponse {
    run_id: i64,
    status: String,
    cancel_requested_at: Option<String>,
}

async fn cancel_run(
    State(state): State<AppState>,
    Path(run_id): Path<i64>,
) -> Result<Json<CancelRunResponse>, ApiError> {
    let run = sqlx::query_as::<_, RunCancellationRecord>(
        r#"
        SELECT id, status, cancel_requested_at
        FROM job_runs
        WHERE id = ?
        "#,
    )
    .bind(run_id)
    .fetch_optional(&state.pool)
    .await?;

    let run = match run {
        Some(run) => run,
        None => return Err(ApiError::not_found("run not found")),
    };

    match run.status.as_str() {
        "queued" | "running" => {
            let mut tx = state.pool.begin().await?;

            sqlx::query(
                r#"
                UPDATE job_runs
                SET status = 'cancel_requested',
                    cancel_requested_at = CURRENT_TIMESTAMP
                WHERE id = ?
                "#,
            )
            .bind(run_id)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                r#"
                INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
                VALUES (?, NULL, 'job', 'status_changed', ?, 'cancel_requested', 'cancel requested', CURRENT_TIMESTAMP)
                "#,
            )
            .bind(run_id)
            .bind(&run.status)
            .execute(&mut *tx)
            .await?;

            let response = sqlx::query_as::<_, CancelRunResponse>(
                r#"
                SELECT id AS run_id, status, cancel_requested_at
                FROM job_runs
                WHERE id = ?
                "#,
            )
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;

            tx.commit().await?;

            Ok(Json(response))
        }
        "cancel_requested" | "canceled" => Ok(Json(CancelRunResponse {
            run_id: run.id,
            status: run.status,
            cancel_requested_at: run.cancel_requested_at,
        })),
        _ => Err(ApiError::conflict("run cannot be canceled in its current status")),
    }
}

#[derive(Debug, Serialize, FromRow)]
struct RerunRunResponse {
    run_id: i64,
    rerun_of_run_id: i64,
    status: String,
    queued_at: String,
}

async fn rerun_run(
    State(state): State<AppState>,
    Path(run_id): Path<i64>,
) -> Result<(StatusCode, Json<RerunRunResponse>), ApiError> {
    let source_run = sqlx::query_as::<_, RunForRerun>(
        r#"
        SELECT id, job_definition_id, job_id, job_name, definition_path, definition_hash, working_dir
        FROM job_runs
        WHERE id = ?
        "#,
    )
    .bind(run_id)
    .fetch_optional(&state.pool)
    .await?;

    let source_run = match source_run {
        Some(run) => run,
        None => return Err(ApiError::not_found("run not found")),
    };

    let mut tx = state.pool.begin().await?;

    let result = sqlx::query(
        r#"
        INSERT INTO job_runs (
            job_definition_id, job_id, job_name, status, trigger_type, triggered_by,
            definition_path, definition_hash, working_dir, queued_at, rerun_of_job_run_id
        )
        VALUES (?, ?, ?, 'queued', 'rerun', NULL, ?, ?, ?, CURRENT_TIMESTAMP, ?)
        "#,
    )
    .bind(source_run.job_definition_id)
    .bind(&source_run.job_id)
    .bind(&source_run.job_name)
    .bind(&source_run.definition_path)
    .bind(&source_run.definition_hash)
    .bind(&source_run.working_dir)
    .bind(source_run.id)
    .execute(&mut *tx)
    .await?;

    let new_run_id = result.last_insert_rowid();

    sqlx::query(
        r#"
        INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
        VALUES (?, NULL, 'job', 'status_changed', NULL, 'queued', 'rerun created', CURRENT_TIMESTAMP)
        "#,
    )
    .bind(new_run_id)
    .execute(&mut *tx)
    .await?;

    let response = sqlx::query_as::<_, RerunRunResponse>(
        r#"
        SELECT id AS run_id, rerun_of_job_run_id AS rerun_of_run_id, status, queued_at
        FROM job_runs
        WHERE id = ?
        "#,
    )
    .bind(new_run_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(response)))
}

#[derive(Debug, Deserialize)]
struct RunLogsQuery {
    node_id: Option<String>,
    stream: Option<String>,
    after_sequence: Option<i64>,
    limit: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
struct RunLogRow {
    sequence: i64,
    node_run_id: Option<i64>,
    stream: String,
    content: String,
    occurred_at: String,
}

async fn get_run_logs(
    State(state): State<AppState>,
    Path(run_id): Path<i64>,
    Query(query): Query<RunLogsQuery>,
) -> Result<Json<Vec<RunLogRow>>, ApiError> {
    let limit = query.limit.unwrap_or(200).clamp(1, 1000);

    let logs = sqlx::query_as::<_, RunLogRow>(
        r#"
        SELECT rl.sequence, rl.node_run_id, rl.stream, rl.content, rl.occurred_at
        FROM run_logs rl
        LEFT JOIN node_runs nr ON rl.node_run_id = nr.id
        WHERE rl.job_run_id = ?1
          AND (?2 IS NULL OR nr.node_id = ?2)
          AND (?3 IS NULL OR rl.stream = ?3)
          AND (?4 IS NULL OR rl.sequence > ?4)
        ORDER BY rl.sequence ASC
        LIMIT ?5
        "#,
    )
    .bind(run_id)
    .bind(query.node_id)
    .bind(query.stream)
    .bind(query.after_sequence)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(logs))
}

#[derive(Debug, Serialize, FromRow)]
struct RunEventRow {
    scope: String,
    event_type: String,
    from_status: Option<String>,
    to_status: Option<String>,
    message: Option<String>,
    occurred_at: String,
}

async fn get_run_events(
    State(state): State<AppState>,
    Path(run_id): Path<i64>,
) -> Result<Json<Vec<RunEventRow>>, ApiError> {
    let events = sqlx::query_as::<_, RunEventRow>(
        r#"
        SELECT scope, event_type, from_status, to_status, message, occurred_at
        FROM run_events
        WHERE job_run_id = ?
        ORDER BY occurred_at ASC, id ASC
        "#,
    )
    .bind(run_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(events))
}

#[derive(Debug, Serialize, FromRow)]
struct RunStreamSnapshot {
    id: i64,
    status: String,
    queued_at: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    cancel_requested_at: Option<String>,
}

async fn stream_run(
    State(state): State<AppState>,
    Path(run_id): Path<i64>,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM job_runs WHERE id = ?")
        .bind(run_id)
        .fetch_one(&state.pool)
        .await?;

    if exists == 0 {
        return Err(ApiError::not_found("run not found"));
    }

    let pool = state.pool.clone();
    let stream = IntervalStream::new(interval(Duration::from_secs(1))).then(move |_| {
        let pool = pool.clone();
        async move {
            let snapshot = sqlx::query_as::<_, RunStreamSnapshot>(
                r#"
                SELECT id, status, queued_at, started_at, finished_at, cancel_requested_at
                FROM job_runs
                WHERE id = ?
                "#,
            )
            .bind(run_id)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten();

            let (event, data) = match snapshot {
                Some(snapshot) => (
                    "run_state",
                    serde_json::to_string(&snapshot)
                        .unwrap_or_else(|_| "{\"error\":\"failed to serialize snapshot\"}".to_string()),
                ),
                None => (
                    "run_deleted",
                    json!({ "run_id": run_id, "status": "deleted" }).to_string(),
                ),
            };

            Ok(Event::default().event(event).data(data))
        }
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Debug, FromRow)]
struct JobDefinitionForRun {
    id: i64,
    job_id: String,
    name: String,
    definition_path: String,
    definition_hash: String,
    enabled: i64,
}

#[derive(Debug, FromRow)]
struct RunCancellationRecord {
    id: i64,
    status: String,
    cancel_requested_at: Option<String>,
}

#[derive(Debug, FromRow)]
struct RunForRerun {
    id: i64,
    job_definition_id: i64,
    job_id: String,
    job_name: String,
    definition_path: String,
    definition_hash: String,
    working_dir: String,
}

// ──────────────────────────────────────────────
// Agent Management API
// ──────────────────────────────────────────────

#[derive(Debug, Serialize, FromRow)]
struct AgentSummary {
    agent_id: String,
    name: String,
    hostname: String,
    labels_json: String,
    status: String,
    last_heartbeat_at: String,
    registered_at: String,
}

async fn list_agents(State(state): State<AppState>) -> Result<Json<Vec<AgentSummary>>, ApiError> {
    let agents = sqlx::query_as::<_, AgentSummary>(
        "SELECT agent_id, name, hostname, labels_json, status, last_heartbeat_at, registered_at FROM agents ORDER BY registered_at",
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(agents))
}

#[derive(Debug, Deserialize)]
struct RegisterAgentRequest {
    name: String,
    hostname: String,
    labels: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct RegisterAgentResponse {
    agent_id: String,
    token: String,
}

async fn register_agent(
    State(state): State<AppState>,
    Json(payload): Json<RegisterAgentRequest>,
) -> Result<(StatusCode, Json<RegisterAgentResponse>), ApiError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("name must not be empty"));
    }

    let agent_id = format!("agent-{}", generate_random_id());
    let token = generate_token();
    let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
    let labels = payload.labels.unwrap_or_default();
    let labels_json = serde_json::to_string(&labels)
        .map_err(|e| ApiError::bad_request(format!("invalid labels: {e}")))?;

    sqlx::query(
        r#"
        INSERT INTO agents (agent_id, name, hostname, labels_json, token_hash)
        VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(&agent_id)
    .bind(name)
    .bind(&payload.hostname)
    .bind(&labels_json)
    .bind(&token_hash)
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(RegisterAgentResponse { agent_id, token }),
    ))
}

async fn get_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<Json<AgentSummary>, ApiError> {
    let agent = sqlx::query_as::<_, AgentSummary>(
        "SELECT agent_id, name, hostname, labels_json, status, last_heartbeat_at, registered_at FROM agents WHERE agent_id = ?",
    )
    .bind(&agent_id)
    .fetch_optional(&state.pool)
    .await?;

    match agent {
        Some(a) => Ok(Json(a)),
        None => Err(ApiError::not_found("agent not found")),
    }
}

async fn delete_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM agents WHERE agent_id = ?")
        .bind(&agent_id)
        .execute(&state.pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("agent not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ──────────────────────────────────────────────
// Agent Worker API
// ──────────────────────────────────────────────

async fn authenticate_agent(pool: &SqlitePool, headers: &HeaderMap) -> Result<String, ApiError> {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::unauthorized("missing or invalid Authorization header"))?;

    let token_hash = format!("{:x}", Sha256::digest(auth.as_bytes()));
    let agent_id = sqlx::query_scalar::<_, String>(
        "SELECT agent_id FROM agents WHERE token_hash = ? AND status = 'online'"
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: e.to_string() })?;

    agent_id.ok_or_else(|| ApiError::unauthorized("invalid token or agent offline"))
}

#[derive(Debug, Serialize, FromRow)]
struct TaskRow {
    node_run_id: i64,
    job_run_id: i64,
    node_id: String,
    node_name: Option<String>,
    program: String,
    args_json: String,
    working_dir: String,
    env_json: Option<String>,
    timeout_sec: i64,
    definition_path: String,
    job_id: String,
}

#[derive(Debug, Serialize)]
struct TaskResponse {
    node_run_id: i64,
    job_run_id: i64,
    node_id: String,
    node_name: Option<String>,
    program: String,
    args_json: String,
    working_dir: String,
    env_json: Option<String>,
    timeout_sec: i64,
    outputs: Vec<TaskOutputDef>,
}

#[derive(Debug, Serialize)]
struct TaskOutputDef {
    path: String,
    required: bool,
}

async fn poll_task(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    let agent_id = authenticate_agent(&state.pool, &headers).await?;

    // Get agent labels
    let labels_json = sqlx::query_scalar::<_, String>(
        "SELECT labels_json FROM agents WHERE agent_id = ?"
    )
    .bind(&agent_id)
    .fetch_one(&state.pool)
    .await?;
    let _agent_labels: Vec<String> = serde_json::from_str(&labels_json).unwrap_or_default();

    // Find queued node_runs assigned to this agent
    let task = sqlx::query_as::<_, TaskRow>(
        r#"
        SELECT nr.id AS node_run_id, nr.job_run_id, nr.node_id, nr.node_name,
               nr.program, nr.args_json, nr.working_dir, nr.env_json, nr.timeout_sec,
               jr.definition_path, jr.job_id
        FROM node_runs nr
        JOIN job_runs jr ON nr.job_run_id = jr.id
        WHERE nr.status = 'queued'
          AND nr.assigned_agent_id = ?
        ORDER BY nr.created_at ASC
        LIMIT 1
        "#,
    )
    .bind(&agent_id)
    .fetch_optional(&state.pool)
    .await?;

    match task {
        Some(task) => {
            // Load outputs from job definition for this node
            let outputs = match JobDefinition::load(&task.definition_path) {
                Ok(def) => def
                    .nodes
                    .iter()
                    .find(|n| n.id == task.node_id)
                    .map(|n| {
                        n.outputs
                            .iter()
                            .map(|o| TaskOutputDef {
                                path: o.path.clone(),
                                required: o.required,
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            };

            // Mark as running
            sqlx::query("UPDATE node_runs SET status = 'running', started_at = CURRENT_TIMESTAMP WHERE id = ?")
                .bind(task.node_run_id)
                .execute(&state.pool)
                .await?;

            sqlx::query(
                r#"
                INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
                VALUES (?, ?, 'node', 'status_changed', 'queued', 'running', 'picked by agent', CURRENT_TIMESTAMP)
                "#,
            )
            .bind(task.job_run_id)
            .bind(task.node_run_id)
            .execute(&state.pool)
            .await?;

            let response = TaskResponse {
                node_run_id: task.node_run_id,
                job_run_id: task.job_run_id,
                node_id: task.node_id,
                node_name: task.node_name,
                program: task.program,
                args_json: task.args_json,
                working_dir: task.working_dir,
                env_json: task.env_json,
                timeout_sec: task.timeout_sec,
                outputs,
            };

            Ok(Json(response).into_response())
        }
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

#[derive(Debug, Deserialize)]
struct ReportResultRequest {
    node_run_id: i64,
    status: String,
    exit_code: Option<i32>,
    failure_reason: Option<String>,
    artifacts: Option<Vec<ArtifactReport>>,
}

#[derive(Debug, Deserialize)]
struct ArtifactReport {
    path: String,
    resolved_path: String,
    required: bool,
    exists: bool,
    size_bytes: Option<i64>,
}

async fn report_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ReportResultRequest>,
) -> Result<StatusCode, ApiError> {
    let agent_id = authenticate_agent(&state.pool, &headers).await?;

    // Verify the node is assigned to this agent and is running
    let assigned = sqlx::query_scalar::<_, Option<String>>(
        "SELECT assigned_agent_id FROM node_runs WHERE id = ?"
    )
    .bind(payload.node_run_id)
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    if assigned.as_deref() != Some(&agent_id) {
        return Err(ApiError::bad_request("node not assigned to this agent"));
    }

    let valid_statuses = ["success", "failed", "timed_out", "canceled"];
    if !valid_statuses.contains(&payload.status.as_str()) {
        return Err(ApiError::bad_request("invalid status"));
    }

    let finished = matches!(payload.status.as_str(), "success" | "failed" | "timed_out" | "canceled");

    // Get job_run_id for events
    let job_run_id = sqlx::query_scalar::<_, i64>(
        "SELECT job_run_id FROM node_runs WHERE id = ?"
    )
    .bind(payload.node_run_id)
    .fetch_one(&state.pool)
    .await?;

    sqlx::query(
        r#"
        UPDATE node_runs
        SET status = ?,
            exit_code = ?,
            failure_reason = ?,
            finished_at = CASE WHEN ? THEN CURRENT_TIMESTAMP ELSE finished_at END
        WHERE id = ?
        "#,
    )
    .bind(&payload.status)
    .bind(payload.exit_code)
    .bind(payload.failure_reason.as_deref())
    .bind(finished)
    .bind(payload.node_run_id)
    .execute(&state.pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
        VALUES (?, ?, 'node', 'status_changed', 'running', ?, ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(job_run_id)
    .bind(payload.node_run_id)
    .bind(&payload.status)
    .bind(payload.failure_reason.as_deref())
    .execute(&state.pool)
    .await?;

    // Record artifacts if provided
    if let Some(artifacts) = &payload.artifacts {
        for artifact in artifacts {
            sqlx::query(
                r#"
                INSERT INTO run_artifacts (
                    job_run_id, node_run_id, path, resolved_path, required, exists_flag, size_bytes, checked_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
                "#,
            )
            .bind(job_run_id)
            .bind(payload.node_run_id)
            .bind(&artifact.path)
            .bind(&artifact.resolved_path)
            .bind(if artifact.required { 1 } else { 0 })
            .bind(if artifact.exists { 1 } else { 0 })
            .bind(artifact.size_bytes)
            .execute(&state.pool)
            .await?;
        }
    }

    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
struct ReportLogsRequest {
    node_run_id: i64,
    logs: Vec<LogEntry>,
}

#[derive(Debug, Deserialize)]
struct LogEntry {
    stream: String,
    content: String,
}

async fn report_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ReportLogsRequest>,
) -> Result<StatusCode, ApiError> {
    let _agent_id = authenticate_agent(&state.pool, &headers).await?;

    let job_run_id = sqlx::query_scalar::<_, i64>(
        "SELECT job_run_id FROM node_runs WHERE id = ?"
    )
    .bind(payload.node_run_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("node run not found"))?;

    let mut next_sequence = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sequence), 0) FROM run_logs WHERE job_run_id = ?"
    )
    .bind(job_run_id)
    .fetch_one(&state.pool)
    .await?;

    for entry in &payload.logs {
        next_sequence += 1;
        sqlx::query(
            r#"
            INSERT INTO run_logs (job_run_id, node_run_id, stream, sequence, content, occurred_at)
            VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(job_run_id)
        .bind(payload.node_run_id)
        .bind(&entry.stream)
        .bind(next_sequence)
        .bind(&entry.content)
        .execute(&state.pool)
        .await?;
    }

    Ok(StatusCode::OK)
}

async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let agent_id = authenticate_agent(&state.pool, &headers).await?;

    sqlx::query("UPDATE agents SET last_heartbeat_at = CURRENT_TIMESTAMP, status = 'online' WHERE agent_id = ?")
        .bind(&agent_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::OK)
}

async fn upload_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    let _agent_id = authenticate_agent(&state.pool, &headers).await?;

    let mut node_run_id: Option<i64> = None;
    let mut artifact_path: Option<String> = None;
    let mut file_data: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart error: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "node_run_id" => {
                let text = field.text().await.map_err(|e| ApiError::bad_request(e.to_string()))?;
                node_run_id = Some(text.parse::<i64>().map_err(|e| ApiError::bad_request(e.to_string()))?);
            }
            "path" => {
                artifact_path = Some(field.text().await.map_err(|e| ApiError::bad_request(e.to_string()))?);
            }
            "file" => {
                file_data = Some(field.bytes().await.map_err(|e| ApiError::bad_request(e.to_string()))?.to_vec());
            }
            _ => {}
        }
    }

    let node_run_id = node_run_id.ok_or_else(|| ApiError::bad_request("node_run_id is required"))?;
    let artifact_path = artifact_path.ok_or_else(|| ApiError::bad_request("path is required"))?;
    let file_data = file_data.ok_or_else(|| ApiError::bad_request("file is required"))?;

    let job_run_id = sqlx::query_scalar::<_, i64>(
        "SELECT job_run_id FROM node_runs WHERE id = ?"
    )
    .bind(node_run_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| ApiError::not_found("node run not found"))?;

    // Store artifact: artifacts/{job_run_id}/{node_run_id}/{path}
    let dest_dir = state.artifacts_dir
        .join(job_run_id.to_string())
        .join(node_run_id.to_string());
    tokio::fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("failed to create artifact dir: {e}") })?;

    let dest_path = dest_dir.join(&artifact_path);
    // Ensure parent directory exists for nested paths
    if let Some(parent) = dest_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("failed to create dir: {e}") })?;
    }

    let size_bytes = file_data.len() as i64;
    tokio::fs::write(&dest_path, &file_data)
        .await
        .map_err(|e| ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: format!("failed to write artifact: {e}") })?;

    // Record in DB
    sqlx::query(
        r#"
        INSERT INTO run_artifacts (
            job_run_id, node_run_id, path, resolved_path, required, exists_flag, size_bytes, checked_at
        )
        VALUES (?, ?, ?, ?, 1, 1, ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(job_run_id)
    .bind(node_run_id)
    .bind(&artifact_path)
    .bind(dest_path.to_string_lossy().to_string())
    .bind(size_bytes)
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

fn generate_random_id() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 8] = rng.r#gen();
    hex::encode(bytes)
}

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.r#gen();
    hex::encode(bytes)
}

// ──────────────────────────────────────────────
// Error handling
// ──────────────────────────────────────────────

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(json!({ "error": self.message }));
        (self.status, body).into_response()
    }
}
