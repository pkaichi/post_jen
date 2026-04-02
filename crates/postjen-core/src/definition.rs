use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::Path,
};

#[derive(Debug, Clone, Deserialize)]
pub struct JobDefinition {
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(rename = "description")]
    pub _description: Option<String>,
    pub defaults: Option<JobDefaults>,
    pub nodes: Vec<NodeDefinition>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct JobDefaults {
    pub working_dir: Option<String>,
    pub timeout_sec: Option<u64>,
    pub retry: Option<u32>,
    pub env: Option<BTreeMap<String, String>>,
    pub target: Option<NodeTarget>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeTarget {
    pub agent: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeDefinition {
    pub id: String,
    pub name: Option<String>,
    pub program: String,
    pub args: Option<Vec<String>>,
    pub working_dir: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub env: Option<BTreeMap<String, String>>,
    pub timeout_sec: Option<u64>,
    pub retry: Option<u32>,
    pub outputs: Option<Vec<NodeOutput>>,
    pub target: Option<NodeTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeOutput {
    pub path: String,
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedJobDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub working_dir: String,
    pub nodes: Vec<ResolvedNodeDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedNodeDefinition {
    pub id: String,
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: String,
    pub depends_on: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub timeout_sec: u64,
    pub retry: u32,
    pub outputs: Vec<ResolvedNodeOutput>,
    pub target: Option<NodeTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedNodeOutput {
    pub path: String,
    pub required: bool,
}

impl JobDefinition {
    pub fn load(path: impl AsRef<Path>) -> Result<ResolvedJobDefinition> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read definition file: {}", path.display()))?;
        let definition: JobDefinition = serde_yaml::from_str(&contents)
            .with_context(|| format!("failed to parse YAML definition: {}", path.display()))?;
        definition.resolve(path)
    }

    fn resolve(self, _source_path: &Path) -> Result<ResolvedJobDefinition> {
        self.validate()?;

        let defaults = self.defaults.unwrap_or_default();
        let default_timeout = defaults.timeout_sec.unwrap_or(1800);
        let default_retry = defaults.retry.unwrap_or(0);
        let default_env = defaults.env.unwrap_or_default();
        let default_working_dir = defaults.working_dir;
        let default_target = defaults.target;

        let mut nodes_by_id: HashMap<&str, &NodeDefinition> = HashMap::new();
        for node in &self.nodes {
            nodes_by_id.insert(node.id.as_str(), node);
        }

        let order = topological_order(&self.nodes)?;
        let mut resolved_nodes = Vec::with_capacity(order.len());
        for node_id in order {
            let node = nodes_by_id
                .get(node_id.as_str())
                .copied()
                .context("internal error: node missing after validation")?;
            let working_dir = node
                .working_dir
                .clone()
                .or_else(|| default_working_dir.clone())
                .with_context(|| format!("node '{}' is missing working_dir", node.id))?;

            let mut env = default_env.clone();
            if let Some(node_env) = &node.env {
                env.extend(node_env.clone());
            }

            let target = node.target.clone().or_else(|| default_target.clone());

            resolved_nodes.push(ResolvedNodeDefinition {
                id: node.id.clone(),
                name: node.name.clone().unwrap_or_else(|| node.id.clone()),
                program: node.program.trim().to_string(),
                args: node.args.clone().unwrap_or_default(),
                working_dir,
                depends_on: node.depends_on.clone().unwrap_or_default(),
                env,
                timeout_sec: node.timeout_sec.unwrap_or(default_timeout),
                retry: node.retry.unwrap_or(default_retry),
                outputs: node
                    .outputs
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|output| ResolvedNodeOutput {
                        path: output.path,
                        required: output.required.unwrap_or(true),
                    })
                    .collect(),
                target,
            });
        }

        let working_dir = default_working_dir.unwrap_or_else(|| {
            resolved_nodes
                .first()
                .map(|node| node.working_dir.clone())
                .unwrap_or_else(|| ".".to_string())
        });

        Ok(ResolvedJobDefinition {
            id: self.id,
            name: self.name,
            description: self._description,
            working_dir,
            nodes: resolved_nodes,
        })
    }

    fn validate(&self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported definition version: {}", self.version);
        }
        if !is_valid_id(&self.id) {
            bail!("invalid job id: {}", self.id);
        }
        if self.name.trim().is_empty() {
            bail!("job name must not be empty");
        }
        if self.nodes.is_empty() {
            bail!("job must have at least one node");
        }

        let mut seen = HashMap::new();
        for node in &self.nodes {
            if !is_valid_id(&node.id) {
                bail!("invalid node id: {}", node.id);
            }
            if node.program.trim().is_empty() {
                bail!("node '{}' has empty program", node.id);
            }
            if node.timeout_sec.unwrap_or(1) == 0 {
                bail!("node '{}' must have timeout_sec >= 1", node.id);
            }
            if let Some(outputs) = &node.outputs {
                for output in outputs {
                    if output.path.trim().is_empty() {
                        bail!("node '{}' has empty output path", node.id);
                    }
                }
            }
            if seen.insert(node.id.as_str(), ()).is_some() {
                bail!("duplicate node id: {}", node.id);
            }
        }

        for node in &self.nodes {
            for dep in node.depends_on.clone().unwrap_or_default() {
                if !seen.contains_key(dep.as_str()) {
                    bail!("node '{}' depends on unknown node '{}'", node.id, dep);
                }
            }
        }

        topological_order(&self.nodes)?;
        Ok(())
    }
}

fn topological_order(nodes: &[NodeDefinition]) -> Result<Vec<String>> {
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut outgoing: HashMap<String, Vec<String>> = HashMap::new();
    for node in nodes {
        indegree.insert(node.id.clone(), node.depends_on.as_ref().map_or(0, Vec::len));
        outgoing.entry(node.id.clone()).or_default();
    }
    for node in nodes {
        for dep in node.depends_on.clone().unwrap_or_default() {
            outgoing.entry(dep).or_default().push(node.id.clone());
        }
    }

    let mut queue: VecDeque<String> = nodes
        .iter()
        .filter(|node| indegree.get(&node.id).copied().unwrap_or(0) == 0)
        .map(|node| node.id.clone())
        .collect();
    let mut order = Vec::with_capacity(nodes.len());

    while let Some(node_id) = queue.pop_front() {
        order.push(node_id.clone());
        if let Some(next_nodes) = outgoing.get(&node_id) {
            for next in next_nodes {
                if let Some(value) = indegree.get_mut(next) {
                    *value -= 1;
                    if *value == 0 {
                        queue.push_back(next.clone());
                    }
                }
            }
        }
    }

    if order.len() != nodes.len() {
        bail!("node dependency graph contains a cycle");
    }

    Ok(order)
}

fn is_valid_id(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }

    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}
