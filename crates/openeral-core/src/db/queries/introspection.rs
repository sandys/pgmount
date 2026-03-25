use deadpool_postgres::Pool;

use crate::db::types::{ColumnInfo, PrimaryKeyInfo, SchemaInfo, TableInfo};
use crate::error::FsError;

pub async fn list_schemas(pool: &Pool) -> Result<Vec<SchemaInfo>, FsError> {
    let client = super::get_client(pool).await?;

    let rows = client
        .query(
            "SELECT schema_name FROM information_schema.schemata \
             WHERE schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
             ORDER BY schema_name",
            &[],
        )
        .await?;

    let schemas = rows
        .iter()
        .map(|row| SchemaInfo {
            name: row.get("schema_name"),
        })
        .collect();

    Ok(schemas)
}

pub async fn list_tables(pool: &Pool, schema: &str) -> Result<Vec<TableInfo>, FsError> {
    let client = super::get_client(pool).await?;

    let rows = client
        .query(
            "SELECT table_name, table_type FROM information_schema.tables \
             WHERE table_schema = $1 \
             ORDER BY table_name",
            &[&schema],
        )
        .await?;

    let tables = rows
        .iter()
        .map(|row| TableInfo {
            name: row.get("table_name"),
            table_type: row.get("table_type"),
        })
        .collect();

    Ok(tables)
}

pub async fn list_columns(
    pool: &Pool,
    schema: &str,
    table: &str,
) -> Result<Vec<ColumnInfo>, FsError> {
    let client = super::get_client(pool).await?;

    let rows = client
        .query(
            "SELECT column_name, data_type, is_nullable, column_default, ordinal_position \
             FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 \
             ORDER BY ordinal_position",
            &[&schema, &table],
        )
        .await?;

    let columns = rows
        .iter()
        .map(|row| {
            let is_nullable_str: String = row.get("is_nullable");
            let ordinal: i32 = row.get("ordinal_position");
            ColumnInfo {
                name: row.get("column_name"),
                data_type: row.get("data_type"),
                is_nullable: is_nullable_str == "YES",
                column_default: row.get("column_default"),
                ordinal_position: ordinal,
            }
        })
        .collect();

    Ok(columns)
}

pub async fn get_primary_key(
    pool: &Pool,
    schema: &str,
    table: &str,
) -> Result<PrimaryKeyInfo, FsError> {
    let client = super::get_client(pool).await?;

    let rows = client
        .query(
            "SELECT kcu.column_name \
             FROM information_schema.table_constraints tc \
             JOIN information_schema.key_column_usage kcu \
               ON tc.constraint_name = kcu.constraint_name \
              AND tc.table_schema = kcu.table_schema \
             WHERE tc.constraint_type = 'PRIMARY KEY' \
               AND tc.table_schema = $1 \
               AND tc.table_name = $2 \
             ORDER BY kcu.ordinal_position",
            &[&schema, &table],
        )
        .await?;

    let column_names = rows.iter().map(|row| row.get("column_name")).collect();

    Ok(PrimaryKeyInfo { column_names })
}
