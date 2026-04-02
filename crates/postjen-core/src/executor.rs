use crate::definition::{ResolvedNodeDefinition, ResolvedNodeOutput};
use crate::types::{ArtifactResult, NodeExecutionOutcome};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command, time::sleep};

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Run a process for the given node definition.
/// `is_canceled` is called periodically to check if execution should be aborted.
pub async fn run_process<F>(node: &ResolvedNodeDefinition, is_canceled: F) -> NodeExecutionOutcome
where
    F: Fn() -> bool,
{
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
            return NodeExecutionOutcome::failed(format!("failed to spawn process: {error}"));
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
        if is_canceled() {
            let _ = child.kill().await;
            let _ = child.wait().await;
            break NodeExecutionOutcome::canceled();
        }

        if started_at.elapsed() >= deadline {
            let _ = child.kill().await;
            let _ = child.wait().await;
            break NodeExecutionOutcome::timed_out();
        }

        match child.try_wait() {
            Ok(Some(exit)) => {
                if exit.success() {
                    break NodeExecutionOutcome::success(exit.code());
                }
                break NodeExecutionOutcome::failed_with_exit(
                    exit.code(),
                    format!("process exited with status {:?}", exit.code()),
                );
            }
            Ok(None) => {}
            Err(error) => {
                break NodeExecutionOutcome::failed(format!("failed to check process status: {error}"));
            }
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

    NodeExecutionOutcome {
        stdout,
        stderr,
        ..status
    }
}

/// Check output artifacts on the local filesystem.
pub async fn check_outputs(
    outputs: &[ResolvedNodeOutput],
    working_dir: &str,
) -> Vec<ArtifactResult> {
    let mut results = Vec::with_capacity(outputs.len());
    for output in outputs {
        let resolved_path = resolve_output_path(working_dir, output);
        let metadata = tokio::fs::metadata(&resolved_path).await.ok();
        let exists = metadata.is_some();
        let size_bytes = metadata.and_then(|meta| i64::try_from(meta.len()).ok());
        results.push(ArtifactResult {
            path: output.path.clone(),
            resolved_path: resolved_path.to_string_lossy().to_string(),
            required: output.required,
            exists,
            size_bytes,
        });
    }
    results
}

/// Check if any required artifacts are missing. Returns failure reason if so.
pub fn missing_artifacts_reason(results: &[ArtifactResult]) -> Option<String> {
    let missing: Vec<&str> = results
        .iter()
        .filter(|a| a.required && !a.exists)
        .map(|a| a.path.as_str())
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(format!("required outputs missing: {}", missing.join(", ")))
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
