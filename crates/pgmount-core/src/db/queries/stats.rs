use deadpool_postgres::Pool;

use crate::error::FsError;

/// Returns an estimated row count from pg_class.reltuples.
/// This is fast but may be stale if ANALYZE hasn't been run recently.
pub async fn get_row_count_estimate(
    pool: &Pool,
    schema: &str,
    table: &str,
) -> Result<i64, FsError> {
    let client = pool
        .get()
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to get connection: {}", e)))?;

    let rows = client
        .query(
            "SELECT COALESCE(reltuples::bigint, 0) as count \
             FROM pg_class c \
             JOIN pg_namespace n ON c.relnamespace = n.oid \
             WHERE n.nspname = $1 AND c.relname = $2",
            &[&schema, &table],
        )
        .await?;

    if rows.is_empty() {
        return Err(FsError::NotFound);
    }

    let count: i64 = rows[0].get("count");
    Ok(count)
}

/// Returns the exact row count via COUNT(*).
/// This performs a full table scan and may be slow on large tables.
pub async fn get_exact_row_count(
    pool: &Pool,
    schema: &str,
    table: &str,
) -> Result<i64, FsError> {
    let client = pool
        .get()
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to get connection: {}", e)))?;

    let query = format!(
        "SELECT COUNT(*) as count FROM {}.{}",
        quote_ident(schema),
        quote_ident(table),
    );

    let rows = client.query(&query, &[]).await?;

    if rows.is_empty() {
        return Err(FsError::DatabaseError(
            "COUNT(*) returned no rows".to_string(),
        ));
    }

    let count: i64 = rows[0].get("count");
    Ok(count)
}

/// Wraps an identifier in double quotes, escaping any internal double quotes by doubling them.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
