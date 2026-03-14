use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use crate::definition::JobDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::{convert::Infallible, fs, time::Duration};
use tokio::time::interval;
use tokio_stream::{StreamExt, wrappers::IntervalStream};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
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
