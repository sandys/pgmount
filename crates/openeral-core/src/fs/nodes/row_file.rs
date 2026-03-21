use crate::db::queries::{introspection, rows};
use crate::error::FsError;
use crate::fs::attr;
use crate::fs::nodes::column::parse_pk_display;
use crate::fs::nodes::NodeContext;
use fuser::FileAttr;

pub async fn getattr(
    ino: u64,
    _schema: &str,
    _table: &str,
    _pk_display: &str,
    _format: &str,
    _ctx: &NodeContext<'_>,
) -> Result<FileAttr, FsError> {
    Ok(attr::file_attr(ino, 4096))
}

pub async fn read(
    schema: &str,
    table: &str,
    pk_display: &str,
    format: &str,
    offset: i64,
    size: u32,
    ctx: &NodeContext<'_>,
) -> Result<Vec<u8>, FsError> {
    let pk_info = introspection::get_primary_key(ctx.pool, schema, table).await?;
    let pk_values = parse_pk_display(pk_display, &pk_info.column_names);
    let row_data =
        rows::get_row_data(ctx.pool, schema, table, &pk_info.column_names, &pk_values).await?;

    let content = match format {
        "json" => crate::format::json::format_row(&row_data)?,
        "csv" => crate::format::csv::format_row(&row_data)?,
        "yaml" => crate::format::yaml::format_row(&row_data)?,
        _ => return Err(FsError::InvalidArgument(format!("Unknown format: {}", format))),
    };

    let bytes = content.as_bytes();
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Ok(vec![]);
    }
    let end = (offset + size as usize).min(bytes.len());
    Ok(bytes[offset..end].to_vec())
}
