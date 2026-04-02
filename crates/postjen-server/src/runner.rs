use crate::definition::JobDefinition;
use postjen_core::definition::ResolvedNodeDefinition;
use postjen_core::executor;
use postjen_core::types::NodeExecutionOutcome;
use anyhow::{Context, Result, bail};
use sqlx::{FromRow, SqlitePool};
use std::{
    collections::HashMap,
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::Mutex,
    time::{interval, sleep},
};
use tracing::{error, info, warn};

pub fn spawn(pool: SqlitePool) {
    let monitor_pool = pool.clone();
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

    // Spawn agent monitor for heartbeat checking
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            if let Err(error) = check_agent_heartbeats(&monitor_pool).await {
                error!(?error, "agent heartbeat check failed");
            }
            if let Err(error) = check_remote_node_completions(&monitor_pool).await {
                error!(?error, "remote node completion check failed");
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
    let mut waiting_for_remote: Vec<(String, i64)> = Vec::new();

    for node in &definition.nodes {
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
                NodeCompletion::new("skipped"),
            );
            continue;
        }

        if stop_scheduling {
            context
                .mark_node_skipped(node, "job execution already stopped".to_string())
                .await?;
            results.insert(node.id.clone(), NodeCompletion::new("skipped"));
            continue;
        }

        if canceling {
            context
                .mark_node_skipped(node, "job cancellation requested".to_string())
                .await?;
            results.insert(node.id.clone(), NodeCompletion::new("skipped"));
            continue;
        }

        // Check if this node should be executed remotely
        if node.target.is_some() {
            let node_run_id = context.node_run_id(&node.id)?;
            // Assign to a matching agent
            let assigned = context.assign_to_agent(node_run_id, node).await?;
            if assigned {
                context.update_node_status(node_run_id, "pending", "queued", None, None, None, 0).await?;
                waiting_for_remote.push((node.id.clone(), node_run_id));
                // Wait for the remote node to complete
                let outcome = context.wait_for_remote_node(node_run_id).await?;
                let is_terminal_failure = matches!(outcome.status.as_str(), "failed" | "timed_out" | "canceled");
                if is_terminal_failure {
                    stop_scheduling = true;
                    if outcome.status == "canceled" {
                        canceling = true;
                    }
                }
                results.insert(node.id.clone(), NodeCompletion::from_status(&outcome.status));
            } else {
                // No matching agent available - fail the node
                context.update_node_status(node_run_id, "pending", "failed", Some("no matching agent available"), None, None, 0).await?;
                results.insert(node.id.clone(), NodeCompletion::new("failed"));
                stop_scheduling = true;
            }
            continue;
        }

        // Local execution (unchanged)
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
            NodeCompletion::from_status(&outcome.status),
        );
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

/// Check agent heartbeats and mark stale agents as offline
async fn check_agent_heartbeats(pool: &SqlitePool) -> Result<()> {
    // Mark agents offline if no heartbeat for 60 seconds
    let updated = sqlx::query(
        r#"
        UPDATE agents
        SET status = 'offline'
        WHERE status = 'online'
          AND datetime(last_heartbeat_at) < datetime('now', '-60 seconds')
        "#,
    )
    .execute(pool)
    .await?;

    if updated.rows_affected() > 0 {
        warn!(count = updated.rows_affected(), "marked agents as offline due to heartbeat timeout");

        // Fail running nodes assigned to offline agents
        let failed_nodes = sqlx::query_as::<_, OfflineNodeRun>(
            r#"
            SELECT nr.id, nr.job_run_id
            FROM node_runs nr
            JOIN agents a ON nr.assigned_agent_id = a.agent_id
            WHERE nr.status IN ('queued', 'running')
              AND a.status = 'offline'
            "#,
        )
        .fetch_all(pool)
        .await?;

        for node_run in &failed_nodes {
            sqlx::query(
                r#"
                UPDATE node_runs
                SET status = 'failed',
                    finished_at = CURRENT_TIMESTAMP,
                    failure_reason = 'agent went offline'
                WHERE id = ?
                "#,
            )
            .bind(node_run.id)
            .execute(pool)
            .await?;

            sqlx::query(
                r#"
                INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
                VALUES (?, ?, 'node', 'status_changed', 'running', 'failed', 'agent went offline', CURRENT_TIMESTAMP)
                "#,
            )
            .bind(node_run.job_run_id)
            .bind(node_run.id)
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}

/// Check if any running jobs have all remote nodes completed
async fn check_remote_node_completions(_pool: &SqlitePool) -> Result<()> {
    // This is handled by wait_for_remote_node in the run loop
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

            let target_json = node.target.as_ref().map(|t| serde_json::to_string(t).unwrap_or_default());

            let result = sqlx::query(
                r#"
                INSERT INTO node_runs (
                    job_run_id, node_id, node_name, status, program, args_json, working_dir,
                    env_json, timeout_sec, retry_count, target_json
                )
                VALUES (?, ?, ?, 'pending', ?, ?, ?, ?, ?, 0, ?)
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
            .bind(target_json)
            .execute(&self.pool)
            .await?;

            self.node_ids.insert(node.id.clone(), result.last_insert_rowid());
        }

        Ok(())
    }

    async fn assign_to_agent(&self, node_run_id: i64, node: &ResolvedNodeDefinition) -> Result<bool> {
        let target = match &node.target {
            Some(t) => t,
            None => return Ok(false),
        };

        // Find online agents, optionally filtered by name
        let agents = if let Some(agent_name) = &target.agent {
            sqlx::query_as::<_, AgentRow>(
                "SELECT agent_id, name, labels_json FROM agents WHERE status = 'online' AND name = ?"
            )
            .bind(agent_name)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, AgentRow>(
                "SELECT agent_id, name, labels_json FROM agents WHERE status = 'online'"
            )
            .fetch_all(&self.pool)
            .await?
        };

        for agent in &agents {
            // If labels are specified, check they all match
            if !target.labels.is_empty() {
                let agent_labels: Vec<String> = serde_json::from_str(&agent.labels_json).unwrap_or_default();
                let has_all_labels = target.labels.iter().all(|l| agent_labels.contains(l));
                if !has_all_labels {
                    continue;
                }
            }
            sqlx::query("UPDATE node_runs SET assigned_agent_id = ? WHERE id = ?")
                .bind(&agent.agent_id)
                .bind(node_run_id)
                .execute(&self.pool)
                .await?;
            info!(node_run_id, agent_id = %agent.agent_id, agent_name = %agent.name, "assigned node to agent");
            return Ok(true);
        }

        Ok(false)
    }

    async fn wait_for_remote_node(&self, node_run_id: i64) -> Result<NodeExecutionOutcome> {
        loop {
            let status = sqlx::query_scalar::<_, String>(
                "SELECT status FROM node_runs WHERE id = ?"
            )
            .bind(node_run_id)
            .fetch_one(&self.pool)
            .await?;

            match status.as_str() {
                "success" => return Ok(NodeExecutionOutcome::success(None)),
                "failed" => {
                    let reason = sqlx::query_scalar::<_, Option<String>>(
                        "SELECT failure_reason FROM node_runs WHERE id = ?"
                    )
                    .bind(node_run_id)
                    .fetch_one(&self.pool)
                    .await?;
                    return Ok(NodeExecutionOutcome::failed(reason.unwrap_or_else(|| "remote execution failed".to_string())));
                }
                "timed_out" => return Ok(NodeExecutionOutcome::timed_out()),
                "canceled" => return Ok(NodeExecutionOutcome::canceled()),
                _ => {
                    // Check for job cancellation while waiting
                    if self.is_cancel_requested().await? {
                        return Ok(NodeExecutionOutcome::canceled());
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
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
            let outcome = executor::run_process(node, || false).await;

            let retryable = outcome.status == "failed" && attempts + 1 < max_attempts;
            let exit_code = outcome.exit_code;

            self.insert_process_logs(node_run_id, &outcome.stdout, &outcome.stderr)
                .await?;

            if outcome.status == "success" {
                let artifact_results = executor::check_outputs(&node.outputs, &node.working_dir).await;
                let missing_reason = executor::missing_artifacts_reason(&artifact_results);

                // Record artifacts in DB
                for artifact in &artifact_results {
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
                    .bind(&artifact.path)
                    .bind(&artifact.resolved_path)
                    .bind(if artifact.required { 1 } else { 0 })
                    .bind(if artifact.exists { 1 } else { 0 })
                    .bind(artifact.size_bytes)
                    .execute(&self.pool)
                    .await?;
                }

                if let Some(reason) = missing_reason {
                    last_outcome = NodeExecutionOutcome {
                        status: "failed".to_string(),
                        exit_code,
                        failure_reason: Some(reason),
                        stdout: outcome.stdout,
                        stderr: outcome.stderr,
                    };
                } else {
                    sqlx::query("UPDATE node_runs SET retry_count = ? WHERE id = ?")
                        .bind(i64::from(attempts))
                        .bind(node_run_id)
                        .execute(&self.pool)
                        .await?;
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

#[derive(Debug, FromRow)]
struct AgentRow {
    agent_id: String,
    name: String,
    labels_json: String,
}

#[derive(Debug, FromRow)]
struct OfflineNodeRun {
    id: i64,
    job_run_id: i64,
}

#[derive(Debug)]
struct NodeCompletion {
    status: String,
}

impl NodeCompletion {
    fn new(status: &str) -> Self {
        Self { status: status.to_string() }
    }

    fn from_status(status: &str) -> Self {
        Self { status: status.to_string() }
    }
}
