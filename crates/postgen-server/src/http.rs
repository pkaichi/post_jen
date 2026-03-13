use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/jobs", get(list_jobs))
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

async fn start_run(
    Path(_job_id): Path<String>,
    Json(_payload): Json<StartRunRequest>,
) -> Result<StatusCode, ApiError> {
    Err(ApiError::not_implemented("start_run is not implemented yet"))
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

async fn cancel_run(Path(_run_id): Path<i64>) -> Result<StatusCode, ApiError> {
    Err(ApiError::not_implemented("cancel_run is not implemented yet"))
}

async fn rerun_run(Path(_run_id): Path<i64>) -> Result<StatusCode, ApiError> {
    Err(ApiError::not_implemented("rerun_run is not implemented yet"))
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

async fn stream_run(Path(_run_id): Path<i64>) -> Result<StatusCode, ApiError> {
    Err(ApiError::not_implemented("stream_run is not implemented yet"))
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn not_implemented(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_IMPLEMENTED,
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
