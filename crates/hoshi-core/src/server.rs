use serde::{Deserialize, Serialize};

/// What started a thread's server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServerOrigin {
    Agent,
    Terminal,
}

/// A port on this machine's loopback or wildcard address, held by a process the thread
/// started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalServer {
    pub port: u16,
    pub pid: u32,
    pub process_name: Option<String>,
    pub origin: ServerOrigin,
    /// When the process started, in Unix milliseconds.
    pub started_at: Option<u64>,
}

/// The servers a thread's agent and terminals left listening, by port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadServers {
    pub thread_id: String,
    pub servers: Vec<LocalServer>,
}
