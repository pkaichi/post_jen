use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct AgentClient {
    client: Client,
    base_url: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub agent_id: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskInfo {
    pub node_run_id: i64,
    pub job_run_id: i64,
    pub node_id: String,
    pub node_name: Option<String>,
    pub program: String,
    pub args_json: String,
    pub working_dir: String,
    pub env_json: Option<String>,
    pub timeout_sec: i64,
    #[serde(default)]
    pub outputs: Vec<TaskOutputDef>,
}

#[derive(Debug, Deserialize)]
pub struct TaskOutputDef {
    pub path: String,
    pub required: bool,
}

#[derive(Debug, Serialize)]
pub struct ResultReport {
    pub node_run_id: i64,
    pub status: String,
    pub exit_code: Option<i32>,
    pub failure_reason: Option<String>,
    pub artifacts: Option<Vec<ArtifactReport>>,
}

#[derive(Debug, Serialize)]
pub struct ArtifactReport {
    pub path: String,
    pub resolved_path: String,
    pub required: bool,
    pub exists: bool,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct LogsReport {
    pub node_run_id: i64,
    pub logs: Vec<LogEntry>,
}

#[derive(Debug, Serialize)]
pub struct LogEntry {
    pub stream: String,
    pub content: String,
}

impl AgentClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn register(
        &self,
        name: &str,
        hostname: &str,
        labels: &[String],
    ) -> Result<RegisterResponse> {
        let resp = self
            .client
            .post(format!("{}/api/agents", self.base_url))
            .json(&serde_json::json!({
                "name": name,
                "hostname": hostname,
                "labels": labels,
            }))
            .send()
            .await
            .context("failed to connect to server")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("registration failed: {} {}", status, body);
        }

        resp.json().await.context("failed to parse registration response")
    }

    pub async fn poll_task(&self, token: &str) -> Result<Option<TaskInfo>> {
        let resp = self
            .client
            .get(format!("{}/api/agent/task", self.base_url))
            .bearer_auth(token)
            .send()
            .await
            .context("failed to poll task")?;

        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("poll failed: {} {}", status, body);
        }

        let task = resp.json().await.context("failed to parse task response")?;
        Ok(Some(task))
    }

    pub async fn report_result(&self, token: &str, report: &ResultReport) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/api/agent/result", self.base_url))
            .bearer_auth(token)
            .json(report)
            .send()
            .await
            .context("failed to report result")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("result report failed: {} {}", status, body);
        }

        Ok(())
    }

    pub async fn report_logs(&self, token: &str, report: &LogsReport) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/api/agent/logs", self.base_url))
            .bearer_auth(token)
            .json(report)
            .send()
            .await
            .context("failed to report logs")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("log report failed: {} {}", status, body);
        }

        Ok(())
    }

    pub async fn heartbeat(&self, token: &str) -> Result<()> {
        let resp = self
            .client
            .post(format!("{}/api/agent/heartbeat", self.base_url))
            .bearer_auth(token)
            .send()
            .await
            .context("failed to send heartbeat")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("heartbeat failed: {} {}", status, body);
        }

        Ok(())
    }

    pub async fn upload_artifact(
        &self,
        token: &str,
        node_run_id: i64,
        artifact_path: &str,
        file_data: Vec<u8>,
    ) -> Result<()> {
        let form = reqwest::multipart::Form::new()
            .text("node_run_id", node_run_id.to_string())
            .text("path", artifact_path.to_string())
            .part(
                "file",
                reqwest::multipart::Part::bytes(file_data).file_name(artifact_path.to_string()),
            );

        let resp = self
            .client
            .post(format!("{}/api/agent/artifacts", self.base_url))
            .bearer_auth(token)
            .multipart(form)
            .send()
            .await
            .context("failed to upload artifact")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("artifact upload failed: {} {}", status, body);
        }

        Ok(())
    }
}
