use deadpool_postgres::Pool;

use crate::db::types::IndexInfo;
use crate::error::FsError;

pub async fn list_indexes(
    pool: &Pool,
    schema: &str,
    table: &str,
) -> Result<Vec<IndexInfo>, FsError> {
    let client = pool
        .get()
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to get connection: {}", e)))?;

    let rows = client
        .query(
            "SELECT i.relname as index_name, ix.indisunique, ix.indisprimary, \
                    pg_get_indexdef(i.oid) as definition, \
                    array_agg(a.attname ORDER BY array_position(ix.indkey, a.attnum)) as columns \
             FROM pg_class t \
             JOIN pg_namespace n ON t.relnamespace = n.oid \
             JOIN pg_index ix ON t.oid = ix.indrelid \
             JOIN pg_class i ON ix.indexrelid = i.oid \
             JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey) \
             WHERE n.nspname = $1 AND t.relname = $2 \
             GROUP BY i.relname, ix.indisunique, ix.indisprimary, i.oid \
             ORDER BY i.relname",
            &[&schema, &table],
        )
        .await?;

    let indexes = rows
        .iter()
        .map(|row| {
            let columns: Vec<String> = row.get("columns");
            IndexInfo {
                name: row.get("index_name"),
                is_unique: row.get("indisunique"),
                is_primary: row.get("indisprimary"),
                definition: row.get("definition"),
                columns,
            }
        })
        .collect();

    Ok(indexes)
}
