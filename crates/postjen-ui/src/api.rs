use std::collections::HashMap;
use gloo_net::http::Request;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParamDefinition {
    pub name: String,
    pub default: Option<String>,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeTarget {
    pub agent: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub target: Option<NodeTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Triggers {
    pub cron: Option<String>,
    #[serde(default)]
    pub webhook: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JobDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub params: Vec<ParamDefinition>,
    pub triggers: Option<Triggers>,
    pub nodes: Vec<NodeDefinition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunSummary {
    pub id: i64,
    pub job_id: String,
    pub job_name: String,
    pub status: String,
    pub trigger_type: String,
    pub triggered_by: Option<String>,
    pub queued_at: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunDetail {
    pub id: i64,
    pub job_id: String,
    pub job_name: String,
    pub status: String,
    pub trigger_type: String,
    pub triggered_by: Option<String>,
    pub queued_at: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub failure_reason: Option<String>,
    pub params_json: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeRun {
    pub node_id: String,
    pub node_name: Option<String>,
    pub status: String,
    pub assigned_agent_id: Option<String>,
    pub exit_code: Option<i32>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LogEntry {
    pub sequence: i64,
    pub node_run_id: Option<i64>,
    pub stream: String,
    pub content: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JobSummary {
    pub job_id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentSummary {
    pub agent_id: String,
    pub name: String,
    pub hostname: String,
    pub labels_json: String,
    pub status: String,
    pub last_heartbeat_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SecretSummary {
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StartRunResponse {
    pub run_id: i64,
    pub status: String,
}

pub async fn fetch_runs(limit: u32) -> Result<Vec<RunSummary>, String> {
    let resp = Request::get(&format!("/api/runs?limit={limit}"))
        .send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_run(run_id: i64) -> Result<RunDetail, String> {
    let resp = Request::get(&format!("/api/runs/{run_id}"))
        .send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_run_nodes(run_id: i64) -> Result<Vec<NodeRun>, String> {
    let resp = Request::get(&format!("/api/runs/{run_id}/nodes"))
        .send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_run_logs(run_id: i64, after_seq: Option<i64>) -> Result<Vec<LogEntry>, String> {
    let mut url = format!("/api/runs/{run_id}/logs?limit=500");
    if let Some(seq) = after_seq {
        url.push_str(&format!("&after_sequence={seq}"));
    }
    let resp = Request::get(&url).send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_jobs() -> Result<Vec<JobSummary>, String> {
    let resp = Request::get("/api/jobs").send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_job_runs(job_id: &str, limit: u32) -> Result<Vec<RunSummary>, String> {
    let resp = Request::get(&format!("/api/runs?job_id={job_id}&limit={limit}"))
        .send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_agents() -> Result<Vec<AgentSummary>, String> {
    let resp = Request::get("/api/agents").send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_secrets() -> Result<Vec<SecretSummary>, String> {
    let resp = Request::get("/api/secrets").send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn fetch_job_definition(job_id: &str) -> Result<JobDefinition, String> {
    let resp = Request::get(&format!("/api/jobs/{job_id}/definition"))
        .send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn start_run(job_id: &str, params: Option<HashMap<String, String>>) -> Result<StartRunResponse, String> {
    let body = serde_json::json!({
        "trigger_type": "manual",
        "triggered_by": "web-ui",
        "params": params,
    });
    let resp = Request::post(&format!("/api/jobs/{job_id}/runs"))
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&body).unwrap())
        .map_err(|e| e.to_string())?
        .send().await.map_err(|e| e.to_string())?;
    if !resp.ok() {
        let text = resp.text().await.unwrap_or_default();
        return Err(text);
    }
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn cancel_run(run_id: i64) -> Result<(), String> {
    Request::post(&format!("/api/runs/{run_id}/cancel"))
        .send().await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn rerun_run(run_id: i64) -> Result<StartRunResponse, String> {
    let resp = Request::post(&format!("/api/runs/{run_id}/rerun"))
        .send().await.map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}
