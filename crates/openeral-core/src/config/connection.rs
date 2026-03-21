use std::fs;
use std::path::PathBuf;

use crate::error::FsError;
use crate::config::PgmountConfig;

/// Resolve a PostgreSQL connection string from multiple sources, in priority order:
///
/// 1. An explicit CLI argument (`cli_arg`), if provided.
/// 2. An environment variable named by `env_var` (e.g. `"OPENERAL_DATABASE_URL"`).
/// 3. The first connection entry in `~/.openeral/config.yml`.
///
/// Returns `FsError::InvalidArgument` if no source yields a connection string.
pub fn resolve_connection_string(
    cli_arg: Option<&str>,
    env_var: &str,
) -> Result<String, FsError> {
    // 1. Explicit CLI argument takes highest priority.
    if let Some(arg) = cli_arg {
        return Ok(arg.to_string());
    }

    // 2. Environment variable.
    if let Ok(val) = std::env::var(env_var) {
        if !val.is_empty() {
            return Ok(val);
        }
    }

    // 3. Config file at ~/.openeral/config.yml
    if let Some(home) = home_dir() {
        let config_path = home.join(".openeral").join("config.yml");
        if config_path.exists() {
            let contents = fs::read_to_string(&config_path).map_err(|e| {
                FsError::InvalidArgument(format!(
                    "Failed to read config file {}: {}",
                    config_path.display(),
                    e,
                ))
            })?;

            let config: PgmountConfig = serde_yml::from_str(&contents).map_err(|e| {
                FsError::InvalidArgument(format!(
                    "Failed to parse config file {}: {}",
                    config_path.display(),
                    e,
                ))
            })?;

            if let Some(first) = config.connections.first() {
                return Ok(first.connection_string.clone());
            }
        }
    }

    Err(FsError::InvalidArgument(
        "No connection string provided. Pass one via CLI, set the environment variable, \
         or configure ~/.openeral/config.yml"
            .to_string(),
    ))
}

/// Returns the user's home directory, preferring `$HOME` on Unix.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}
