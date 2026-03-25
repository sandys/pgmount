use deadpool_postgres::Pool;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use crate::db::types::RowIdentifier;
use crate::error::FsError;

use super::quote_ident;

/// Characters that must be percent-encoded in PK display names.
/// Encodes: / , = % and all control characters
const PK_ENCODE_SET: &AsciiSet = &CONTROLS.add(b'/').add(b',').add(b'=').add(b'%').add(b'\0');

pub fn encode_pk_value(value: &str) -> String {
    utf8_percent_encode(value, PK_ENCODE_SET).to_string()
}

/// Flexible row query with optional WHERE and ORDER BY clauses.
/// Used by table listing, filter, and order nodes.
/// `extra_where` is appended to the WHERE clause (e.g., "\"category\"::text = $1")
/// `extra_order` overrides the default PK ordering (e.g., "\"name\" ASC")
/// `extra_params` are the parameters for the extra_where clause
pub async fn query_rows(
    pool: &Pool,
    schema: &str,
    table: &str,
    pk_columns: &[String],
    limit: i64,
    offset: i64,
    extra_where: Option<&str>,
    extra_order: Option<&str>,
    extra_params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> Result<Vec<RowIdentifier>, FsError> {
    let client = super::get_client(pool).await?;

    if pk_columns.is_empty() {
        return Err(FsError::DatabaseError(
            "No primary key columns specified".to_string(),
        ));
    }

    let select_cols: Vec<String> = pk_columns.iter().map(|c| quote_ident(c)).collect();

    let order_clause = match extra_order {
        Some(order) => order.to_string(),
        None => select_cols.join(", "),
    };

    // Build WHERE clause
    // extra_params use $1..$N, then LIMIT is $N+1, OFFSET is $N+2
    let param_offset = extra_params.len();
    let where_clause = match extra_where {
        Some(w) => format!(" WHERE {}", w),
        None => String::new(),
    };

    let query = format!(
        "SELECT {} FROM {}.{}{} ORDER BY {} LIMIT ${} OFFSET ${}",
        select_cols.join(", "),
        quote_ident(schema),
        quote_ident(table),
        where_clause,
        order_clause,
        param_offset + 1,
        param_offset + 2,
    );

    let mut all_params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = extra_params.to_vec();
    all_params.push(&limit);
    all_params.push(&offset);

    let rows = client.query(&query, &all_params).await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut pk_values = Vec::with_capacity(pk_columns.len());
        let mut display_parts = Vec::with_capacity(pk_columns.len());

        for (i, col_name) in pk_columns.iter().enumerate() {
            let value_str = column_to_string(row, i);
            if pk_columns.len() == 1 {
                display_parts.push(encode_pk_value(&value_str));
            } else {
                display_parts.push(format!("{}={}", col_name, encode_pk_value(&value_str)));
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

pub async fn list_rows(
    pool: &Pool,
    schema: &str,
    table: &str,
    pk_columns: &[String],
    limit: i64,
    offset: i64,
) -> Result<Vec<RowIdentifier>, FsError> {
    query_rows(
        pool,
        schema,
        table,
        pk_columns,
        limit,
        offset,
        None,
        None,
        &[],
    )
    .await
}

/// Fetch all rows from a table as text columns, in a single query.
/// Returns Vec of (column_names, rows_data) where each row is Vec<Option<String>>.
pub async fn get_all_rows_as_text(
    pool: &Pool,
    schema: &str,
    table: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<String>, Vec<Vec<(String, Option<String>)>>), FsError> {
    let client = super::get_client(pool).await?;

    // Get column names
    let col_info_query = "SELECT column_name FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position";
    let col_rows = client.query(col_info_query, &[&schema, &table]).await?;
    let col_names: Vec<String> = col_rows
        .iter()
        .map(|r| r.get::<_, String>("column_name"))
        .collect();

    // Build SELECT with ::text cast for every column
    let select_exprs: Vec<String> = col_names
        .iter()
        .map(|c| format!("{}::text", quote_ident(c)))
        .collect();

    let query = format!(
        "SELECT {} FROM {}.{} ORDER BY 1 LIMIT $1 OFFSET $2",
        select_exprs.join(", "),
        quote_ident(schema),
        quote_ident(table),
    );

    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&limit, &offset];
    let rows = client.query(&query, &params).await?;

    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut row_data = Vec::with_capacity(col_names.len());
        for (i, col_name) in col_names.iter().enumerate() {
            let value: Option<String> = row.get(i);
            row_data.push((col_name.clone(), value));
        }
        result.push(row_data);
    }

    Ok((col_names, result))
}

pub async fn get_row_data(
    pool: &Pool,
    schema: &str,
    table: &str,
    pk_columns: &[String],
    pk_values: &[String],
) -> Result<Vec<(String, Option<String>)>, FsError> {
    let client = super::get_client(pool).await?;

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

    // First get column names, then build a query that casts all to text
    let col_info_query = "SELECT column_name FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position".to_string();
    let col_rows = client.query(&col_info_query, &[&schema, &table]).await?;
    let col_names: Vec<String> = col_rows
        .iter()
        .map(|r| r.get::<_, String>("column_name"))
        .collect();

    // Build SELECT with ::text cast for every column
    let select_exprs: Vec<String> = col_names
        .iter()
        .map(|c| format!("{}::text", quote_ident(c)))
        .collect();

    let query = format!(
        "SELECT {} FROM {}.{} WHERE {}",
        select_exprs.join(", "),
        quote_ident(schema),
        quote_ident(table),
        where_clauses.join(" AND "),
    );

    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pk_values
        .iter()
        .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let rows = client.query(&query, &params).await?;

    if rows.is_empty() {
        return Err(FsError::NotFound);
    }

    let row = &rows[0];
    let mut result = Vec::with_capacity(col_names.len());

    for (i, col_name) in col_names.iter().enumerate() {
        let value: Option<String> = row.get(i);
        result.push((col_name.clone(), value));
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
    let client = super::get_client(pool).await?;

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
        "SELECT {}::text FROM {}.{} WHERE {}",
        quote_ident(column),
        quote_ident(schema),
        quote_ident(table),
        where_clauses.join(" AND "),
    );

    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = pk_values
        .iter()
        .map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();

    let rows = client.query(&query, &params).await?;

    if rows.is_empty() {
        return Err(FsError::NotFound);
    }

    let value: Option<String> = rows[0].get(0);
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
