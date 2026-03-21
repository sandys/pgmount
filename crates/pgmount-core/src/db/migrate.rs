use std::ops::DerefMut;

use crate::error::FsError;
use tracing::info;

mod embedded {
    use refinery::embed_migrations;
    embed_migrations!("migrations");
}

/// Run all pending database migrations.
///
/// Creates the `_pgmount` schema and internal metadata tables.
/// Safe to call multiple times — already-applied migrations are skipped.
pub async fn run_migrations(pool: &deadpool_postgres::Pool) -> Result<(), FsError> {
    let mut client = pool.get().await.map_err(|e| {
        FsError::DatabaseError(format!("Failed to get connection for migrations: {}", e))
    })?;
    let client_ref: &mut tokio_postgres::Client = client.deref_mut();

    info!("Running database migrations");
    embedded::migrations::runner()
        .run_async(client_ref)
        .await
        .map_err(|e| FsError::DatabaseError(format!("Migration failed: {}", e)))?;

    info!("Migrations complete");
    Ok(())
}

/// Record a mount session in the `_pgmount.mount_log` table.
pub async fn log_mount_session(
    pool: &deadpool_postgres::Pool,
    mount_point: &str,
    schemas_filter: Option<&[String]>,
    page_size: usize,
) -> Result<(), FsError> {
    let client = pool.get().await.map_err(|e| {
        FsError::DatabaseError(format!("Failed to get connection for mount log: {}", e))
    })?;

    let version = env!("CARGO_PKG_VERSION");
    let schemas: Option<Vec<&str>> = schemas_filter.map(|s| s.iter().map(|s| s.as_str()).collect());

    client
        .execute(
            "INSERT INTO _pgmount.mount_log (mount_point, schemas_filter, page_size, pgmount_version) \
             VALUES ($1, $2, $3, $4)",
            &[&mount_point, &schemas, &(page_size as i32), &version],
        )
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to log mount session: {}", e)))?;

    Ok(())
}
