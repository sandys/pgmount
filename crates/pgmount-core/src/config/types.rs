use std::time::Duration;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct PgmountConfig {
    pub connections: Vec<ConnectionConfig>,
    pub default_schema_filter: Option<Vec<String>>,
    pub cache_ttl_secs: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConnectionConfig {
    pub name: String,
    pub connection_string: String,
    pub schemas: Option<Vec<String>>,
    pub read_only: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct MountConfig {
    pub connection_string: String,
    pub mount_point: String,
    pub schemas: Option<Vec<String>>,
    pub read_only: bool,
    pub cache_ttl: Duration,
    pub page_size: usize,
    pub statement_timeout_secs: u64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceMountConfig {
    pub connection_string: String,
    pub workspace_id: String,
    pub mount_point: String,
    pub display_name: Option<String>,
    pub statement_timeout_secs: u64,
}

impl Default for MountConfig {
    fn default() -> Self {
        Self {
            connection_string: String::new(),
            mount_point: String::new(),
            schemas: None,
            read_only: true,
            cache_ttl: Duration::from_secs(30),
            page_size: 1000,
            statement_timeout_secs: 30,
        }
    }
}
