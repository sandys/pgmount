use deadpool_postgres::Pool;

use crate::db::types::RowIdentifier;
use crate::error::FsError;

/// Wraps an identifier in double quotes, escaping any internal double quotes by doubling them.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub async fn list_rows(
    pool: &Pool,
    schema: &str,
    table: &str,
    pk_columns: &[String],
    limit: i64,
    offset: i64,
) -> Result<Vec<RowIdentifier>, FsError> {
    let client = pool
        .get()
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to get connection: {}", e)))?;

    if pk_columns.is_empty() {
        return Err(FsError::DatabaseError(
            "No primary key columns specified".to_string(),
        ));
    }

    let select_cols: Vec<String> = pk_columns.iter().map(|c| quote_ident(c)).collect();
    let order_cols = select_cols.clone();

    let query = format!(
        "SELECT {} FROM {}.{} ORDER BY {} LIMIT $1 OFFSET $2",
        select_cols.join(", "),
        quote_ident(schema),
        quote_ident(table),
        order_cols.join(", "),
    );

    let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        vec![&limit, &offset];

    let rows = client.query(&query, &param_refs).await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut pk_values = Vec::with_capacity(pk_columns.len());
        let mut display_parts = Vec::with_capacity(pk_columns.len());

        for (i, col_name) in pk_columns.iter().enumerate() {
            // Get the value as a string representation
            let value_str = column_to_string(row, i);
            if pk_columns.len() == 1 {
                display_parts.push(value_str.clone());
            } else {
                display_parts.push(format!("{}={}", col_name, &value_str));
            }
            pk_values.push((col_name.clone(), value_str));
        }

        let display_name = display_parts.join(",");

        result.push(RowIdentifier {
            pk_values,
            display_name,
        });
    }

    Ok(result)
}

pub async fn get_row_data(
    pool: &Pool,
    schema: &str,
    table: &str,
    pk_columns: &[String],
    pk_values: &[String],
) -> Result<Vec<(String, Option<String>)>, FsError> {
    let client = pool
        .get()
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to get connection: {}", e)))?;

    if pk_columns.len() != pk_values.len() {
        return Err(FsError::InvalidArgument(
            "PK columns and values length mismatch".to_string(),
        ));
    }

    let where_clauses: Vec<String> = pk_columns
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{}::text = ${}", quote_ident(col), i + 1))
        .collect();

    let query = format!(
        "SELECT * FROM {}.{} WHERE {}",
        quote_ident(schema),
        quote_ident(table),
        where_clauses.join(" AND "),
    );

    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        pk_values.iter().map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync)).collect();

    let rows = client.query(&query, &params).await?;

    if rows.is_empty() {
        return Err(FsError::NotFound);
    }

    let row = &rows[0];
    let columns = row.columns();
    let mut result = Vec::with_capacity(columns.len());

    for (i, col) in columns.iter().enumerate() {
        let value = column_to_option_string(row, i);
        result.push((col.name().to_string(), value));
    }

    Ok(result)
}

pub async fn get_column_value(
    pool: &Pool,
    schema: &str,
    table: &str,
    column: &str,
    pk_columns: &[String],
    pk_values: &[String],
) -> Result<Option<String>, FsError> {
    let client = pool
        .get()
        .await
        .map_err(|e| FsError::DatabaseError(format!("Failed to get connection: {}", e)))?;

    if pk_columns.len() != pk_values.len() {
        return Err(FsError::InvalidArgument(
            "PK columns and values length mismatch".to_string(),
        ));
    }

    let where_clauses: Vec<String> = pk_columns
        .iter()
        .enumerate()
        .map(|(i, col)| format!("{}::text = ${}", quote_ident(col), i + 1))
        .collect();

    let query = format!(
        "SELECT {} FROM {}.{} WHERE {}",
        quote_ident(column),
        quote_ident(schema),
        quote_ident(table),
        where_clauses.join(" AND "),
    );

    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
        pk_values.iter().map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync)).collect();

    let rows = client.query(&query, &params).await?;

    if rows.is_empty() {
        return Err(FsError::NotFound);
    }

    let value = column_to_option_string(&rows[0], 0);
    Ok(value)
}

/// Convert a column value to a String representation.
/// Handles common PostgreSQL types by casting to text via the database.
fn column_to_string(row: &tokio_postgres::Row, idx: usize) -> String {
    // Try to get the value as a string directly; if not possible, fall back to text representation.
    if let Ok(v) = row.try_get::<_, String>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<_, i32>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, i64>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, i16>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, f32>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, f64>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, bool>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, chrono::NaiveDateTime>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, chrono::NaiveDate>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, chrono::DateTime<chrono::Utc>>(idx) {
        return v.to_string();
    }
    if let Ok(v) = row.try_get::<_, serde_json::Value>(idx) {
        return v.to_string();
    }
    // Fallback
    "NULL".to_string()
}

/// Convert a column value to an Option<String>, returning None for SQL NULL values.
fn column_to_option_string(row: &tokio_postgres::Row, idx: usize) -> Option<String> {
    // First check for NULL by trying Option<String>
    if let Ok(v) = row.try_get::<_, Option<String>>(idx) {
        return v;
    }
    if let Ok(v) = row.try_get::<_, Option<i32>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<i64>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<i16>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<f32>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<f64>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<bool>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<chrono::NaiveDateTime>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<chrono::NaiveDate>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(idx) {
        return v.map(|x| x.to_string());
    }
    if let Ok(v) = row.try_get::<_, Option<serde_json::Value>>(idx) {
        return v.map(|x| x.to_string());
    }
    // If we can't determine the type, return None
    None
}
