use deadpool_postgres::{Pool, ManagerConfig, RecyclingMethod};
use tokio_postgres::NoTls;

use crate::error::FsError;

pub type DbPool = Pool;

pub fn create_pool(connection_string: &str, statement_timeout_secs: u64) -> Result<DbPool, FsError> {
    let mut pg_config: tokio_postgres::Config = connection_string
        .parse()
        .map_err(|e: tokio_postgres::Error| {
            FsError::DatabaseError(format!("Invalid connection string: {}", e))
        })?;

    // Set statement timeout as a connection-level default
    pg_config.options(format!("-c statement_timeout={}s", statement_timeout_secs));

    let mgr_config = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };
    let mgr = deadpool_postgres::Manager::from_config(pg_config, NoTls, mgr_config);
    let pool = Pool::builder(mgr)
        .max_size(16)
        .build()
        .map_err(|e| FsError::DatabaseError(format!("Failed to create pool: {}", e)))?;

    Ok(pool)
}
