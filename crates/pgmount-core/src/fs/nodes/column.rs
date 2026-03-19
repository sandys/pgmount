use crate::db::queries::{introspection, rows};
use crate::error::FsError;
use crate::fs::attr;
use crate::fs::nodes::NodeContext;
use fuser::FileAttr;

pub async fn getattr(
    ino: u64,
    _schema: &str,
    _table: &str,
    _pk_display: &str,
    _column: &str,
    _ctx: &NodeContext<'_>,
) -> Result<FileAttr, FsError> {
    // Use estimated size; actual size determined on read
    Ok(attr::file_attr(ino, 4096))
}

pub async fn read(
    schema: &str,
    table: &str,
    pk_display: &str,
    column: &str,
    offset: i64,
    size: u32,
    ctx: &NodeContext<'_>,
) -> Result<Vec<u8>, FsError> {
    let pk_info = introspection::get_primary_key(ctx.pool, schema, table).await?;
    let pk_values = parse_pk_display(pk_display, &pk_info.column_names);
    let value =
        rows::get_column_value(ctx.pool, schema, table, column, &pk_info.column_names, &pk_values)
            .await?;
    let content = match value {
        Some(v) => format!("{}\n", v),
        None => "NULL\n".to_string(),
    };
    let bytes = content.as_bytes();
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Ok(vec![]);
    }
    let end = (offset + size as usize).min(bytes.len());
    Ok(bytes[offset..end].to_vec())
}

/// Parse a pk_display string back into individual PK values.
/// For single-column PKs: "value"
/// For composite PKs: "val1,val2"
pub fn parse_pk_display(display: &str, pk_columns: &[String]) -> Vec<String> {
    if pk_columns.len() == 1 {
        vec![display.to_string()]
    } else {
        display.split(',').map(|s| s.to_string()).collect()
    }
}
