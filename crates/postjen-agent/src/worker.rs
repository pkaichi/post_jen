use crate::client::{AgentClient, ArtifactReport, LogEntry, LogsReport, ResultReport, TaskInfo};
use postjen_core::definition::{ResolvedNodeDefinition, ResolvedNodeOutput};
use postjen_core::executor;
use std::collections::BTreeMap;
use std::path::Path;
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
            Ok(None) => {}
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

    let outputs: Vec<ResolvedNodeOutput> = task
        .outputs
        .iter()
        .map(|o| ResolvedNodeOutput {
            path: o.path.clone(),
            required: o.required,
        })
        .collect();

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

    // Check and upload artifacts if execution succeeded
    let mut final_status = outcome.status.clone();
    let mut final_failure_reason = outcome.failure_reason.clone();
    let mut artifact_reports = Vec::new();

    if outcome.status == "success" && !outputs.is_empty() {
        let artifact_results = executor::check_outputs(&outputs, &task.working_dir).await;

        for artifact in &artifact_results {
            artifact_reports.push(ArtifactReport {
                path: artifact.path.clone(),
                resolved_path: artifact.resolved_path.clone(),
                required: artifact.required,
                exists: artifact.exists,
                size_bytes: artifact.size_bytes,
            });

            // Upload the file if it exists
            if artifact.exists {
                let resolved = Path::new(&artifact.resolved_path);
                match tokio::fs::read(resolved).await {
                    Ok(data) => {
                        if let Err(e) = client
                            .upload_artifact(token, task.node_run_id, &artifact.path, data)
                            .await
                        {
                            warn!(path = %artifact.path, ?e, "failed to upload artifact");
                        } else {
                            info!(path = %artifact.path, "artifact uploaded");
                        }
                    }
                    Err(e) => {
                        warn!(path = %artifact.resolved_path, ?e, "failed to read artifact file");
                    }
                }
            }
        }

        // Check if required artifacts are missing
        if let Some(reason) = executor::missing_artifacts_reason(&artifact_results) {
            final_status = "failed".to_string();
            final_failure_reason = Some(reason);
        }
    }

    // Report result
    client
        .report_result(
            token,
            &ResultReport {
                node_run_id: task.node_run_id,
                status: final_status.clone(),
                exit_code: outcome.exit_code,
                failure_reason: final_failure_reason,
                artifacts: if artifact_reports.is_empty() {
                    None
                } else {
                    Some(artifact_reports)
                },
            },
        )
        .await?;

    info!(
        node_run_id = task.node_run_id,
        status = %final_status,
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
