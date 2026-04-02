use crate::definition::{JobDefinition, ResolvedNodeDefinition, ResolvedNodeOutput};
use anyhow::{Context, Result, bail};
use sqlx::{FromRow, SqlitePool};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::Mutex,
    time::{interval, sleep},
};
use tracing::{error, info};

const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub fn spawn(pool: SqlitePool) {
    tokio::spawn(async move {
        let worker_lock = Arc::new(Mutex::new(()));
        let mut ticker = interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            let _guard = worker_lock.lock().await;
            if let Err(error) = process_next_run(&pool).await {
                error!(?error, "run worker iteration failed");
                sleep(Duration::from_secs(1)).await;
            }
        }
    });
}

async fn process_next_run(pool: &SqlitePool) -> Result<()> {
    let run = sqlx::query_as::<_, QueuedRun>(
        r#"
        SELECT id, job_id, definition_path
        FROM job_runs
        WHERE status = 'queued'
        ORDER BY created_at ASC, id ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;

    let Some(run) = run else {
        return Ok(());
    };

    info!(run_id = run.id, job_id = %run.job_id, "picked queued run");
    if let Err(error) = execute_run(pool, &run).await {
        error!(run_id = run.id, ?error, "run execution failed unexpectedly");
        fail_run_before_start(pool, run.id, format!("{error:#}")).await?;
    }

    Ok(())
}

async fn execute_run(pool: &SqlitePool, run: &QueuedRun) -> Result<()> {
    let definition = JobDefinition::load(&run.definition_path)?;
    if definition.id != run.job_id {
        bail!(
            "job definition id mismatch: run has '{}', YAML has '{}'",
            run.job_id,
            definition.id
        );
    }

    let mut context = RunContext::new(pool.clone(), run.id).await?;
    context.ensure_nodes(&definition.nodes).await?;
    context.transition_job("queued", "running", None).await?;

    let mut results: HashMap<String, NodeCompletion> = HashMap::new();
    let mut stop_scheduling = false;
    let mut canceling = false;

    for (index, node) in definition.nodes.iter().enumerate() {
        let should_cancel = context.is_cancel_requested().await?;
        if should_cancel {
            canceling = true;
        }

        let blocked_by_dependency = node.depends_on.iter().find_map(|dep| {
            results
                .get(dep)
                .filter(|result| result.status != "success")
                .map(|result| (dep, result.status.as_str()))
        });

        if let Some((dep, status)) = blocked_by_dependency {
            context
                .mark_node_skipped(node, format!("dependency '{}' ended with {}", dep, status))
                .await?;
            results.insert(
                node.id.clone(),
                NodeCompletion::new("skipped", None, Some(format!("dependency '{}' ended with {}", dep, status))),
            );
            continue;
        }

        if stop_scheduling {
            context
                .mark_node_skipped(node, "job execution already stopped".to_string())
                .await?;
            results.insert(
                node.id.clone(),
                NodeCompletion::new("skipped", None, Some("job execution already stopped".to_string())),
            );
            continue;
        }

        if canceling {
            context
                .mark_node_skipped(node, "job cancellation requested".to_string())
                .await?;
            results.insert(
                node.id.clone(),
                NodeCompletion::new("skipped", None, Some("job cancellation requested".to_string())),
            );
            continue;
        }

        let outcome = context.execute_node(node).await?;
        let is_terminal_failure = matches!(outcome.status.as_str(), "failed" | "timed_out" | "canceled");
        if is_terminal_failure {
            stop_scheduling = true;
            if outcome.status == "canceled" {
                canceling = true;
            }
        }
        results.insert(
            node.id.clone(),
            NodeCompletion::new(outcome.status, outcome.exit_code, outcome.failure_reason),
        );

        for remaining in definition.nodes.iter().skip(index + 1) {
            if results.contains_key(&remaining.id) {
                continue;
            }
        }
    }

    let final_status = determine_job_status(&results, canceling);
    context.finalize_job(final_status).await?;
    Ok(())
}

fn determine_job_status(results: &HashMap<String, NodeCompletion>, canceling: bool) -> &'static str {
    if results.values().any(|result| result.status == "timed_out") {
        "timed_out"
    } else if results.values().any(|result| result.status == "failed") {
        "failed"
    } else if canceling || results.values().any(|result| result.status == "canceled") {
        "canceled"
    } else {
        "success"
    }
}

async fn fail_run_before_start(pool: &SqlitePool, run_id: i64, reason: String) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE job_runs
        SET status = 'failed',
            started_at = COALESCE(started_at, CURRENT_TIMESTAMP),
            finished_at = CURRENT_TIMESTAMP,
            failure_reason = ?
        WHERE id = ?
        "#,
    )
    .bind(&reason)
    .bind(run_id)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
        VALUES (?, NULL, 'job', 'status_changed', 'queued', 'failed', ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(run_id)
    .bind(&reason)
    .execute(pool)
    .await?;

    Ok(())
}

struct RunContext {
    pool: SqlitePool,
    run_id: i64,
    next_sequence: i64,
    node_ids: HashMap<String, i64>,
}

impl RunContext {
    async fn new(pool: SqlitePool, run_id: i64) -> Result<Self> {
        let next_sequence = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(sequence), 0) FROM run_logs WHERE job_run_id = ?",
        )
        .bind(run_id)
        .fetch_one(&pool)
        .await?;

        Ok(Self {
            pool,
            run_id,
            next_sequence,
            node_ids: HashMap::new(),
        })
    }

    async fn ensure_nodes(&mut self, nodes: &[ResolvedNodeDefinition]) -> Result<()> {
        for node in nodes {
            let args_json = serde_json::to_string(&node.args)?;
            let env_json = if node.env.is_empty() {
                None
            } else {
                Some(serde_json::to_string(&node.env)?)
            };

            let result = sqlx::query(
                r#"
                INSERT INTO node_runs (
                    job_run_id, node_id, node_name, status, program, args_json, working_dir,
                    env_json, timeout_sec, retry_count
                )
                VALUES (?, ?, ?, 'pending', ?, ?, ?, ?, ?, 0)
                "#,
            )
            .bind(self.run_id)
            .bind(&node.id)
            .bind(&node.name)
            .bind(&node.program)
            .bind(args_json)
            .bind(&node.working_dir)
            .bind(env_json)
            .bind(i64::try_from(node.timeout_sec).context("timeout_sec exceeds i64")?)
            .execute(&self.pool)
            .await?;

            self.node_ids.insert(node.id.clone(), result.last_insert_rowid());
        }

        Ok(())
    }

    async fn transition_job(
        &self,
        from_status: &str,
        to_status: &str,
        message: Option<&str>,
    ) -> Result<()> {
        match to_status {
            "running" => {
                sqlx::query(
                    r#"
                    UPDATE job_runs
                    SET status = 'running',
                        started_at = COALESCE(started_at, CURRENT_TIMESTAMP)
                    WHERE id = ? AND status = ?
                    "#,
                )
                .bind(self.run_id)
                .bind(from_status)
                .execute(&self.pool)
                .await?;
            }
            "success" | "failed" | "timed_out" | "canceled" => {
                sqlx::query(
                    r#"
                    UPDATE job_runs
                    SET status = ?,
                        finished_at = CURRENT_TIMESTAMP,
                        failure_reason = CASE WHEN ? IS NULL THEN failure_reason ELSE ? END
                    WHERE id = ?
                    "#,
                )
                .bind(to_status)
                .bind(message)
                .bind(message)
                .bind(self.run_id)
                .execute(&self.pool)
                .await?;
            }
            _ => {
                sqlx::query("UPDATE job_runs SET status = ? WHERE id = ?")
                    .bind(to_status)
                    .bind(self.run_id)
                    .execute(&self.pool)
                    .await?;
            }
        }

        sqlx::query(
            r#"
            INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
            VALUES (?, NULL, 'job', 'status_changed', ?, ?, ?, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(self.run_id)
        .bind(from_status)
        .bind(to_status)
        .bind(message)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn finalize_job(&self, final_status: &str) -> Result<()> {
        let failure_reason = match final_status {
            "failed" => Some("one or more nodes failed"),
            "timed_out" => Some("one or more nodes timed out"),
            "canceled" => Some("job canceled"),
            _ => None,
        };

        let current_status = sqlx::query_scalar::<_, String>("SELECT status FROM job_runs WHERE id = ?")
            .bind(self.run_id)
            .fetch_one(&self.pool)
            .await?;

        self.transition_job(&current_status, final_status, failure_reason)
            .await
    }

    async fn is_cancel_requested(&self) -> Result<bool> {
        let status = sqlx::query_scalar::<_, String>("SELECT status FROM job_runs WHERE id = ?")
            .bind(self.run_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(status == "cancel_requested")
    }

    async fn mark_node_skipped(&self, node: &ResolvedNodeDefinition, reason: String) -> Result<()> {
        let node_run_id = self.node_run_id(&node.id)?;
        sqlx::query(
            r#"
            UPDATE node_runs
            SET status = 'skipped',
                finished_at = CURRENT_TIMESTAMP,
                failure_reason = ?
            WHERE id = ?
            "#,
        )
        .bind(&reason)
        .bind(node_run_id)
        .execute(&self.pool)
        .await?;

        self.insert_node_event(node_run_id, "pending", "skipped", Some(&reason))
            .await
    }

    async fn execute_node(&mut self, node: &ResolvedNodeDefinition) -> Result<NodeExecutionOutcome> {
        let node_run_id = self.node_run_id(&node.id)?;
        self.update_node_status(node_run_id, "pending", "queued", None, None, None, 0)
            .await?;
        self.update_node_status(node_run_id, "queued", "running", None, None, None, 0)
            .await?;

        let mut attempts = 0;
        let max_attempts = node.retry + 1;
        let mut last_outcome = NodeExecutionOutcome::failed("node did not execute".to_string());

        while attempts < max_attempts {
            let outcome = self.run_process(node).await?;
            let retryable = outcome.status == "failed" && attempts + 1 < max_attempts;
            let exit_code = outcome.exit_code;

            self.insert_process_logs(node_run_id, &outcome.stdout, &outcome.stderr)
                .await?;

            if outcome.status == "success" {
                let artifact_result = self.check_outputs(node_run_id, node, attempts).await?;
                if let Some(reason) = artifact_result {
                    last_outcome = NodeExecutionOutcome {
                        status: "failed".to_string(),
                        exit_code,
                        failure_reason: Some(reason),
                        stdout: outcome.stdout,
                        stderr: outcome.stderr,
                    };
                } else {
                    last_outcome = NodeExecutionOutcome {
                        status: "success".to_string(),
                        exit_code,
                        failure_reason: None,
                        stdout: outcome.stdout,
                        stderr: outcome.stderr,
                    };
                    break;
                }
            } else {
                last_outcome = outcome;
            }

            attempts += 1;
            if retryable {
                self.insert_system_log(
                    node_run_id,
                    format!("retrying node '{}' ({}/{})", node.id, attempts, node.retry),
                )
                .await?;
                continue;
            }
            break;
        }

        let from_status = "running";
        let to_status = last_outcome.status.as_str();
        self.update_node_status(
            node_run_id,
            from_status,
            to_status,
            last_outcome.failure_reason.as_deref(),
            last_outcome.exit_code,
            None,
            attempts,
        )
        .await?;

        Ok(last_outcome)
    }

    async fn run_process(&self, node: &ResolvedNodeDefinition) -> Result<NodeExecutionOutcome> {
        let mut command = Command::new(&node.program);
        command
            .args(&node.args)
            .current_dir(&node.working_dir)
            .envs(&node.env)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Ok(NodeExecutionOutcome::failed(format!(
                    "failed to spawn process: {error}"
                )));
            }
        };

        let stdout_handle = child.stdout.take().map(|mut stdout| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let _ = stdout.read_to_end(&mut buf).await;
                String::from_utf8_lossy(&buf).into_owned()
            })
        });
        let stderr_handle = child.stderr.take().map(|mut stderr| {
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let _ = stderr.read_to_end(&mut buf).await;
                String::from_utf8_lossy(&buf).into_owned()
            })
        });

        let started_at = Instant::now();
        let deadline = Duration::from_secs(node.timeout_sec);

        let status = loop {
            if self.is_cancel_requested().await? {
                let _ = child.kill().await;
                let _ = child.wait().await;
                break NodeExecutionOutcome::canceled();
            }

            if started_at.elapsed() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                break NodeExecutionOutcome::timed_out();
            }

            if let Some(exit) = child.try_wait()? {
                if exit.success() {
                    break NodeExecutionOutcome::success(exit.code());
                }
                break NodeExecutionOutcome::failed_with_exit(
                    exit.code(),
                    format!("process exited with status {:?}", exit.code()),
                );
            }

            sleep(POLL_INTERVAL).await;
        };

        let stdout = if let Some(handle) = stdout_handle {
            handle.await.unwrap_or_default()
        } else {
            String::new()
        };
        let stderr = if let Some(handle) = stderr_handle {
            handle.await.unwrap_or_default()
        } else {
            String::new()
        };

        Ok(NodeExecutionOutcome {
            stdout,
            stderr,
            ..status
        })
    }

    async fn check_outputs(
        &mut self,
        node_run_id: i64,
        node: &ResolvedNodeDefinition,
        retries_used: u32,
    ) -> Result<Option<String>> {
        let mut missing = Vec::new();
        for output in &node.outputs {
            let resolved_path = resolve_output_path(&node.working_dir, output);
            let metadata = tokio::fs::metadata(&resolved_path).await.ok();
            let exists = metadata.is_some();
            let size_bytes = metadata.and_then(|meta| i64::try_from(meta.len()).ok());
            sqlx::query(
                r#"
                INSERT INTO run_artifacts (
                    job_run_id, node_run_id, path, resolved_path, required, exists_flag, size_bytes, checked_at
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
                "#,
            )
            .bind(self.run_id)
            .bind(node_run_id)
            .bind(&output.path)
            .bind(resolved_path.to_string_lossy().to_string())
            .bind(if output.required { 1 } else { 0 })
            .bind(if exists { 1 } else { 0 })
            .bind(size_bytes)
            .execute(&self.pool)
            .await?;

            if output.required && !exists {
                missing.push(output.path.clone());
            }
        }

        if missing.is_empty() {
            sqlx::query("UPDATE node_runs SET retry_count = ? WHERE id = ?")
                .bind(i64::from(retries_used))
                .bind(node_run_id)
                .execute(&self.pool)
                .await?;
            Ok(None)
        } else {
            Ok(Some(format!(
                "required outputs missing: {}",
                missing.join(", ")
            )))
        }
    }

    async fn insert_process_logs(&mut self, node_run_id: i64, stdout: &str, stderr: &str) -> Result<()> {
        if !stdout.is_empty() {
            self.insert_log(node_run_id, "stdout", stdout.to_string()).await?;
        }
        if !stderr.is_empty() {
            self.insert_log(node_run_id, "stderr", stderr.to_string()).await?;
        }
        Ok(())
    }

    async fn insert_system_log(&mut self, node_run_id: i64, content: String) -> Result<()> {
        self.insert_log(node_run_id, "system", content).await
    }

    async fn insert_log(&mut self, node_run_id: i64, stream: &str, content: String) -> Result<()> {
        self.next_sequence += 1;
        sqlx::query(
            r#"
            INSERT INTO run_logs (job_run_id, node_run_id, stream, sequence, content, occurred_at)
            VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(self.run_id)
        .bind(node_run_id)
        .bind(stream)
        .bind(self.next_sequence)
        .bind(content)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_node_status(
        &self,
        node_run_id: i64,
        from_status: &str,
        to_status: &str,
        failure_reason: Option<&str>,
        exit_code: Option<i32>,
        event_message: Option<&str>,
        retry_count: u32,
    ) -> Result<()> {
        let started = matches!(to_status, "running");
        let finished = matches!(to_status, "success" | "failed" | "timed_out" | "canceled");
        let cancel_requested = to_status == "cancel_requested";

        sqlx::query(
            r#"
            UPDATE node_runs
            SET status = ?,
                started_at = CASE WHEN ? THEN COALESCE(started_at, CURRENT_TIMESTAMP) ELSE started_at END,
                finished_at = CASE WHEN ? THEN CURRENT_TIMESTAMP ELSE finished_at END,
                cancel_requested_at = CASE WHEN ? THEN CURRENT_TIMESTAMP ELSE cancel_requested_at END,
                failure_reason = ?,
                exit_code = ?,
                retry_count = ?
            WHERE id = ?
            "#,
        )
        .bind(to_status)
        .bind(started)
        .bind(finished)
        .bind(cancel_requested)
        .bind(failure_reason)
        .bind(exit_code)
        .bind(i64::from(retry_count))
        .bind(node_run_id)
        .execute(&self.pool)
        .await?;

        self.insert_node_event(
            node_run_id,
            from_status,
            to_status,
            event_message.or(failure_reason),
        )
        .await
    }

    async fn insert_node_event(
        &self,
        node_run_id: i64,
        from_status: &str,
        to_status: &str,
        message: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
            VALUES (?, ?, 'node', 'status_changed', ?, ?, ?, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(self.run_id)
        .bind(node_run_id)
        .bind(from_status)
        .bind(to_status)
        .bind(message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn node_run_id(&self, node_id: &str) -> Result<i64> {
        self.node_ids
            .get(node_id)
            .copied()
            .with_context(|| format!("missing node_run id for node '{}'", node_id))
    }
}

#[derive(Debug, FromRow)]
struct QueuedRun {
    id: i64,
    job_id: String,
    definition_path: String,
}

#[derive(Debug)]
struct NodeCompletion {
    status: String,
}

impl NodeCompletion {
    fn new(status: impl Into<String>, _exit_code: Option<i32>, _failure_reason: Option<String>) -> Self {
        Self { status: status.into() }
    }
}

#[derive(Debug)]
struct NodeExecutionOutcome {
    status: String,
    exit_code: Option<i32>,
    failure_reason: Option<String>,
    stdout: String,
    stderr: String,
}

impl NodeExecutionOutcome {
    fn success(exit_code: Option<i32>) -> Self {
        Self {
            status: "success".to_string(),
            exit_code,
            failure_reason: None,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn failed(reason: String) -> Self {
        Self {
            status: "failed".to_string(),
            exit_code: None,
            failure_reason: Some(reason),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn failed_with_exit(exit_code: Option<i32>, reason: String) -> Self {
        Self {
            status: "failed".to_string(),
            exit_code,
            failure_reason: Some(reason),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn timed_out() -> Self {
        Self {
            status: "timed_out".to_string(),
            exit_code: None,
            failure_reason: Some("node timed out".to_string()),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    fn canceled() -> Self {
        Self {
            status: "canceled".to_string(),
            exit_code: None,
            failure_reason: Some("node canceled".to_string()),
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

fn resolve_output_path(working_dir: &str, output: &ResolvedNodeOutput) -> PathBuf {
    let path = Path::new(&output.path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(working_dir).join(path)
    }
}
