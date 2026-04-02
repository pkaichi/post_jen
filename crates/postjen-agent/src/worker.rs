use crate::client::{AgentClient, LogEntry, LogsReport, ResultReport, TaskInfo};
use postjen_core::definition::ResolvedNodeDefinition;
use postjen_core::executor;
use std::collections::BTreeMap;
use tokio::time::{Duration, interval};
use tracing::{error, info, warn};

pub async fn poll_loop(client: &AgentClient, token: &str, interval_secs: u64) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    loop {
        ticker.tick().await;
        match client.poll_task(token).await {
            Ok(Some(task)) => {
                info!(node_run_id = task.node_run_id, node_id = %task.node_id, "received task");
                if let Err(error) = execute_task(client, token, &task).await {
                    error!(node_run_id = task.node_run_id, ?error, "task execution failed");
                    // Report failure to server
                    let _ = client
                        .report_result(
                            token,
                            &ResultReport {
                                node_run_id: task.node_run_id,
                                status: "failed".to_string(),
                                exit_code: None,
                                failure_reason: Some(format!("{error:#}")),
                                artifacts: None,
                            },
                        )
                        .await;
                }
            }
            Ok(None) => {
                // No task available
            }
            Err(error) => {
                warn!(?error, "failed to poll task");
            }
        }
    }
}

async fn execute_task(client: &AgentClient, token: &str, task: &TaskInfo) -> anyhow::Result<()> {
    let args: Vec<String> = serde_json::from_str(&task.args_json)?;
    let env: BTreeMap<String, String> = task
        .env_json
        .as_deref()
        .map(|s| serde_json::from_str(s).unwrap_or_default())
        .unwrap_or_default();

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
        outputs: Vec::new(), // Outputs are checked after execution
        target: None,
    };

    // Execute the process
    let outcome = executor::run_process(&node, || false).await;

    // Send logs
    let mut logs = Vec::new();
    if !outcome.stdout.is_empty() {
        logs.push(LogEntry {
            stream: "stdout".to_string(),
            content: outcome.stdout.clone(),
        });
    }
    if !outcome.stderr.is_empty() {
        logs.push(LogEntry {
            stream: "stderr".to_string(),
            content: outcome.stderr.clone(),
        });
    }
    if !logs.is_empty() {
        client
            .report_logs(
                token,
                &LogsReport {
                    node_run_id: task.node_run_id,
                    logs,
                },
            )
            .await?;
    }

    // Check artifacts (if we had output definitions - for now the server handles this)
    // TODO: Pass output definitions from server to agent in task info

    // Report result
    client
        .report_result(
            token,
            &ResultReport {
                node_run_id: task.node_run_id,
                status: outcome.status.clone(),
                exit_code: outcome.exit_code,
                failure_reason: outcome.failure_reason.clone(),
                artifacts: None,
            },
        )
        .await?;

    info!(
        node_run_id = task.node_run_id,
        status = %outcome.status,
        "task completed"
    );

    Ok(())
}

pub async fn heartbeat_loop(client: &AgentClient, token: &str, interval_secs: u64) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    loop {
        ticker.tick().await;
        if let Err(error) = client.heartbeat(token).await {
            warn!(?error, "heartbeat failed");
        }
    }
}
