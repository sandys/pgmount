pub mod indexes;
pub mod introspection;
pub mod rows;
pub mod stats;
pub mod workspace;

/// Wraps an identifier in double quotes, escaping any internal double quotes by doubling them.
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub async fn get_client(
    pool: &deadpool_postgres::Pool,
) -> Result<deadpool_postgres::Object, crate::error::FsError> {
    pool.get().await.map_err(|e| {
        crate::error::FsError::DatabaseError(format!("Failed to get connection: {}", e))
    })
}
