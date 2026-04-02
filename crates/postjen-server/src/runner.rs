use crate::definition::JobDefinition;
use postjen_core::definition::ResolvedNodeDefinition;
use postjen_core::executor;
use postjen_core::types::NodeExecutionOutcome;
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::Mutex,
    time::{interval, sleep},
};
use tracing::{error, info, warn};

const LOCAL_AGENT_ID: &str = "local";

/// Register the built-in local agent and spawn all background workers.
pub async fn spawn(pool: SqlitePool, artifacts_dir: PathBuf) -> Result<()> {
    register_local_agent(&pool).await?;

    // Spawn scheduler: picks queued runs, assigns nodes to agents
    let scheduler_pool = pool.clone();
    tokio::spawn(async move {
        let worker_lock = Arc::new(Mutex::new(()));
        let mut ticker = interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            let _guard = worker_lock.lock().await;
            if let Err(error) = process_next_run(&scheduler_pool).await {
                error!(?error, "run worker iteration failed");
                sleep(Duration::from_secs(1)).await;
            }
        }
    });

    // Spawn local worker: executes tasks assigned to built-in agent
    let worker_pool = pool.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(1));
        loop {
            ticker.tick().await;
            if let Err(error) = process_local_task(&worker_pool, &artifacts_dir).await {
                error!(?error, "local worker iteration failed");
                sleep(Duration::from_secs(1)).await;
            }
        }
    });

    // Spawn agent monitor: heartbeat checking
    let monitor_pool = pool.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(15));
        loop {
            ticker.tick().await;
            if let Err(error) = check_agent_heartbeats(&monitor_pool).await {
                error!(?error, "agent heartbeat check failed");
            }
        }
    });

    Ok(())
}

async fn register_local_agent(pool: &SqlitePool) -> Result<()> {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "localhost".to_string());
    let token_hash = format!("{:x}", Sha256::digest(LOCAL_AGENT_ID.as_bytes()));

    sqlx::query(
        r#"
        INSERT INTO agents (agent_id, name, hostname, labels_json, token_hash, status)
        VALUES (?, 'local', ?, '["local"]', ?, 'online')
        ON CONFLICT(agent_id) DO UPDATE SET
            status = 'online',
            last_heartbeat_at = CURRENT_TIMESTAMP
        "#,
    )
    .bind(LOCAL_AGENT_ID)
    .bind(&hostname)
    .bind(&token_hash)
    .execute(pool)
    .await?;

    info!("registered built-in local agent");
    Ok(())
}

// ──────────────────────────────────────────────
// Scheduler: assigns nodes to agents
// ──────────────────────────────────────────────

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
            results.insert(node.id.clone(), NodeCompletion::new("skipped"));
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

        // Assign node to an agent (local or remote)
        let node_run_id = context.node_run_id(&node.id)?;
        let assigned = context.assign_to_agent(node_run_id, node).await?;
        if assigned {
            context.update_node_status(node_run_id, "pending", "queued", None, None, None, 0).await?;
            // Wait for agent (local or remote) to complete the node
            let outcome = context.wait_for_agent_node(node_run_id).await?;
            let is_terminal_failure = matches!(outcome.status.as_str(), "failed" | "timed_out" | "canceled");
            if is_terminal_failure {
                stop_scheduling = true;
                if outcome.status == "canceled" {
                    canceling = true;
                }
            }
            results.insert(node.id.clone(), NodeCompletion::from_status(&outcome.status));
        } else {
            context.update_node_status(node_run_id, "pending", "failed", Some("no matching agent available"), None, None, 0).await?;
            results.insert(node.id.clone(), NodeCompletion::new("failed"));
            stop_scheduling = true;
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

// ──────────────────────────────────────────────
// Local worker: executes tasks for built-in agent
// ──────────────────────────────────────────────

async fn process_local_task(pool: &SqlitePool, artifacts_dir: &PathBuf) -> Result<()> {
    let task = sqlx::query_as::<_, LocalTask>(
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
    .bind(LOCAL_AGENT_ID)
    .fetch_optional(pool)
    .await?;

    let Some(task) = task else {
        return Ok(());
    };

    info!(node_run_id = task.node_run_id, node_id = %task.node_id, "local worker picked task");

    // Mark as running
    sqlx::query("UPDATE node_runs SET status = 'running', started_at = CURRENT_TIMESTAMP WHERE id = ?")
        .bind(task.node_run_id)
        .execute(pool)
        .await?;
    insert_node_event(pool, task.job_run_id, task.node_run_id, "queued", "running", Some("picked by local agent")).await?;

    // Build resolved node from task
    let args: Vec<String> = serde_json::from_str(&task.args_json).unwrap_or_default();
    let env: std::collections::BTreeMap<String, String> = task.env_json
        .as_deref()
        .map(|s| serde_json::from_str(s).unwrap_or_default())
        .unwrap_or_default();

    // Load outputs from definition
    let outputs = match JobDefinition::load(&task.definition_path) {
        Ok(def) => def.nodes.iter()
            .find(|n| n.id == task.node_id)
            .map(|n| n.outputs.clone())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    let node = ResolvedNodeDefinition {
        id: task.node_id.clone(),
        name: task.node_name.clone().unwrap_or_else(|| task.node_id.clone()),
        program: task.program.clone(),
        args,
        working_dir: task.working_dir.clone(),
        depends_on: Vec::new(),
        env,
        timeout_sec: task.timeout_sec as u64,
        retry: 0,
        outputs: outputs.clone(),
        target: None,
    };

    // Execute
    let outcome = executor::run_process(&node, || false).await;

    // Record logs
    let mut next_seq = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sequence), 0) FROM run_logs WHERE job_run_id = ?"
    )
    .bind(task.job_run_id)
    .fetch_one(pool)
    .await?;

    if !outcome.stdout.is_empty() {
        next_seq += 1;
        sqlx::query(
            "INSERT INTO run_logs (job_run_id, node_run_id, stream, sequence, content, occurred_at) VALUES (?, ?, 'stdout', ?, ?, CURRENT_TIMESTAMP)"
        )
        .bind(task.job_run_id).bind(task.node_run_id).bind(next_seq).bind(&outcome.stdout)
        .execute(pool).await?;
    }
    if !outcome.stderr.is_empty() {
        next_seq += 1;
        sqlx::query(
            "INSERT INTO run_logs (job_run_id, node_run_id, stream, sequence, content, occurred_at) VALUES (?, ?, 'stderr', ?, ?, CURRENT_TIMESTAMP)"
        )
        .bind(task.job_run_id).bind(task.node_run_id).bind(next_seq).bind(&outcome.stderr)
        .execute(pool).await?;
    }

    // Check artifacts and store locally
    let mut final_status = outcome.status.clone();
    let mut final_failure_reason = outcome.failure_reason.clone();

    if outcome.status == "success" && !outputs.is_empty() {
        let artifact_results = executor::check_outputs(&outputs, &task.working_dir).await;

        for artifact in &artifact_results {
            // Copy artifact to server artifacts dir
            if artifact.exists {
                let dest_dir = artifacts_dir
                    .join(task.job_run_id.to_string())
                    .join(task.node_run_id.to_string());
                let dest_path = dest_dir.join(&artifact.path);
                if let Some(parent) = dest_path.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::copy(&artifact.resolved_path, &dest_path).await;
            }

            sqlx::query(
                r#"
                INSERT INTO run_artifacts (
                    job_run_id, node_run_id, path, resolved_path, required, exists_flag, size_bytes, checked_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
                "#,
            )
            .bind(task.job_run_id).bind(task.node_run_id)
            .bind(&artifact.path).bind(&artifact.resolved_path)
            .bind(if artifact.required { 1 } else { 0 })
            .bind(if artifact.exists { 1 } else { 0 })
            .bind(artifact.size_bytes)
            .execute(pool).await?;
        }

        if let Some(reason) = executor::missing_artifacts_reason(&artifact_results) {
            final_status = "failed".to_string();
            final_failure_reason = Some(reason);
        }
    }

    // Update node status
    let finished = matches!(final_status.as_str(), "success" | "failed" | "timed_out" | "canceled");
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
    .bind(&final_status)
    .bind(outcome.exit_code)
    .bind(final_failure_reason.as_deref())
    .bind(finished)
    .bind(task.node_run_id)
    .execute(pool)
    .await?;

    insert_node_event(pool, task.job_run_id, task.node_run_id, "running", &final_status, final_failure_reason.as_deref()).await?;

    info!(node_run_id = task.node_run_id, status = %final_status, "local worker completed task");
    Ok(())
}

async fn insert_node_event(pool: &SqlitePool, job_run_id: i64, node_run_id: i64, from: &str, to: &str, message: Option<&str>) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
        VALUES (?, ?, 'node', 'status_changed', ?, ?, ?, CURRENT_TIMESTAMP)
        "#,
    )
    .bind(job_run_id).bind(node_run_id).bind(from).bind(to).bind(message)
    .execute(pool).await?;
    Ok(())
}

// ──────────────────────────────────────────────
// Heartbeat monitor
// ──────────────────────────────────────────────

async fn check_agent_heartbeats(pool: &SqlitePool) -> Result<()> {
    // Keep local agent alive
    sqlx::query("UPDATE agents SET last_heartbeat_at = CURRENT_TIMESTAMP WHERE agent_id = ?")
        .bind(LOCAL_AGENT_ID)
        .execute(pool)
        .await?;

    let updated = sqlx::query(
        r#"
        UPDATE agents
        SET status = 'offline'
        WHERE status = 'online'
          AND agent_id != ?
          AND datetime(last_heartbeat_at) < datetime('now', '-60 seconds')
        "#,
    )
    .bind(LOCAL_AGENT_ID)
    .execute(pool)
    .await?;

    if updated.rows_affected() > 0 {
        warn!(count = updated.rows_affected(), "marked agents as offline due to heartbeat timeout");

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
                "UPDATE node_runs SET status = 'failed', finished_at = CURRENT_TIMESTAMP, failure_reason = 'agent went offline' WHERE id = ?",
            )
            .bind(node_run.id)
            .execute(pool)
            .await?;

            insert_node_event(pool, node_run.job_run_id, node_run.id, "running", "failed", Some("agent went offline")).await?;
        }
    }

    Ok(())
}

// ──────────────────────────────────────────────
// RunContext: scheduler helper
// ──────────────────────────────────────────────

struct RunContext {
    pool: SqlitePool,
    run_id: i64,
    node_ids: HashMap<String, i64>,
}

impl RunContext {
    async fn new(pool: SqlitePool, run_id: i64) -> Result<Self> {
        Ok(Self {
            pool,
            run_id,
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

    /// Assign a node to an agent. For target-less nodes, assign to built-in local agent.
    async fn assign_to_agent(&self, node_run_id: i64, node: &ResolvedNodeDefinition) -> Result<bool> {
        match &node.target {
            Some(target) => {
                // Find matching remote agent
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
                    if !target.labels.is_empty() {
                        let agent_labels: Vec<String> = serde_json::from_str(&agent.labels_json).unwrap_or_default();
                        if !target.labels.iter().all(|l| agent_labels.contains(l)) {
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
            None => {
                // No target: assign to built-in local agent
                sqlx::query("UPDATE node_runs SET assigned_agent_id = ? WHERE id = ?")
                    .bind(LOCAL_AGENT_ID)
                    .bind(node_run_id)
                    .execute(&self.pool)
                    .await?;
                info!(node_run_id, "assigned node to local agent");
                Ok(true)
            }
        }
    }

    async fn wait_for_agent_node(&self, node_run_id: i64) -> Result<NodeExecutionOutcome> {
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
                    return Ok(NodeExecutionOutcome::failed(reason.unwrap_or_else(|| "execution failed".to_string())));
                }
                "timed_out" => return Ok(NodeExecutionOutcome::timed_out()),
                "canceled" => return Ok(NodeExecutionOutcome::canceled()),
                _ => {
                    if self.is_cancel_requested().await? {
                        return Ok(NodeExecutionOutcome::canceled());
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn transition_job(&self, from_status: &str, to_status: &str, message: Option<&str>) -> Result<()> {
        match to_status {
            "running" => {
                sqlx::query(
                    "UPDATE job_runs SET status = 'running', started_at = COALESCE(started_at, CURRENT_TIMESTAMP) WHERE id = ? AND status = ?"
                )
                .bind(self.run_id).bind(from_status)
                .execute(&self.pool).await?;
            }
            "success" | "failed" | "timed_out" | "canceled" => {
                sqlx::query(
                    "UPDATE job_runs SET status = ?, finished_at = CURRENT_TIMESTAMP, failure_reason = CASE WHEN ? IS NULL THEN failure_reason ELSE ? END WHERE id = ?"
                )
                .bind(to_status).bind(message).bind(message).bind(self.run_id)
                .execute(&self.pool).await?;
            }
            _ => {
                sqlx::query("UPDATE job_runs SET status = ? WHERE id = ?")
                    .bind(to_status).bind(self.run_id)
                    .execute(&self.pool).await?;
            }
        }

        sqlx::query(
            r#"
            INSERT INTO run_events (job_run_id, node_run_id, scope, event_type, from_status, to_status, message, occurred_at)
            VALUES (?, NULL, 'job', 'status_changed', ?, ?, ?, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(self.run_id).bind(from_status).bind(to_status).bind(message)
        .execute(&self.pool).await?;

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
        self.transition_job(&current_status, final_status, failure_reason).await
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
        sqlx::query("UPDATE node_runs SET status = 'skipped', finished_at = CURRENT_TIMESTAMP, failure_reason = ? WHERE id = ?")
            .bind(&reason).bind(node_run_id)
            .execute(&self.pool).await?;
        insert_node_event(&self.pool, self.run_id, node_run_id, "pending", "skipped", Some(&reason)).await
    }

    async fn update_node_status(&self, node_run_id: i64, from_status: &str, to_status: &str, failure_reason: Option<&str>, exit_code: Option<i32>, event_message: Option<&str>, retry_count: u32) -> Result<()> {
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
                failure_reason = ?, exit_code = ?, retry_count = ?
            WHERE id = ?
            "#,
        )
        .bind(to_status).bind(started).bind(finished).bind(cancel_requested)
        .bind(failure_reason).bind(exit_code).bind(i64::from(retry_count)).bind(node_run_id)
        .execute(&self.pool).await?;

        insert_node_event(&self.pool, self.run_id, node_run_id, from_status, to_status, event_message.or(failure_reason)).await
    }

    fn node_run_id(&self, node_id: &str) -> Result<i64> {
        self.node_ids.get(node_id).copied()
            .with_context(|| format!("missing node_run id for node '{}'", node_id))
    }
}

// ──────────────────────────────────────────────
// Data types
// ──────────────────────────────────────────────

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
struct LocalTask {
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
