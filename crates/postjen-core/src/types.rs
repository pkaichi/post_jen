use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeExecutionOutcome {
    pub status: String,
    pub exit_code: Option<i32>,
    pub failure_reason: Option<String>,
    pub stdout: String,
    pub stderr: String,
}

impl NodeExecutionOutcome {
    pub fn success(exit_code: Option<i32>) -> Self {
        Self {
            status: "success".to_string(),
            exit_code,
            failure_reason: None,
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    pub fn failed(reason: String) -> Self {
        Self {
            status: "failed".to_string(),
            exit_code: None,
            failure_reason: Some(reason),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    pub fn failed_with_exit(exit_code: Option<i32>, reason: String) -> Self {
        Self {
            status: "failed".to_string(),
            exit_code,
            failure_reason: Some(reason),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    pub fn timed_out() -> Self {
        Self {
            status: "timed_out".to_string(),
            exit_code: None,
            failure_reason: Some("node timed out".to_string()),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    pub fn canceled() -> Self {
        Self {
            status: "canceled".to_string(),
            exit_code: None,
            failure_reason: Some("node canceled".to_string()),
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactResult {
    pub path: String,
    pub resolved_path: String,
    pub required: bool,
    pub exists: bool,
    pub size_bytes: Option<i64>,
}
