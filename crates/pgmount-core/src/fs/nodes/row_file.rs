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
        "json" => format_row_json(&row_data)?,
        "csv" => format_row_csv(&row_data)?,
        "yaml" => format_row_yaml(&row_data)?,
        _ => {
            return Err(FsError::InvalidArgument(format!(
                "Unknown format: {}",
                format
            )))
        }
    };

    let bytes = content.as_bytes();
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Ok(vec![]);
    }
    let end = (offset + size as usize).min(bytes.len());
    Ok(bytes[offset..end].to_vec())
}

/// Format a row as JSON
fn format_row_json(row_data: &[(String, Option<String>)]) -> Result<String, FsError> {
    let mut map = serde_json::Map::new();
    for (col, val) in row_data {
        let json_val = match val {
            Some(v) => serde_json::Value::String(v.clone()),
            None => serde_json::Value::Null,
        };
        map.insert(col.clone(), json_val);
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(map))
        .map(|s| format!("{}\n", s))
        .map_err(|e| FsError::SerializationError(e.to_string()))
}

/// Format a row as CSV (header line + value line)
fn format_row_csv(row_data: &[(String, Option<String>)]) -> Result<String, FsError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    let headers: Vec<&str> = row_data.iter().map(|(col, _)| col.as_str()).collect();
    wtr.write_record(&headers)
        .map_err(|e| FsError::SerializationError(e.to_string()))?;
    let values: Vec<&str> = row_data
        .iter()
        .map(|(_, val)| match val {
            Some(v) => v.as_str(),
            None => "NULL",
        })
        .collect();
    wtr.write_record(&values)
        .map_err(|e| FsError::SerializationError(e.to_string()))?;
    let bytes = wtr
        .into_inner()
        .map_err(|e| FsError::SerializationError(e.to_string()))?;
    String::from_utf8(bytes).map_err(|e| FsError::SerializationError(e.to_string()))
}

/// Format a row as YAML
fn format_row_yaml(row_data: &[(String, Option<String>)]) -> Result<String, FsError> {
    let mut map = serde_json::Map::new();
    for (col, val) in row_data {
        let json_val = match val {
            Some(v) => serde_json::Value::String(v.clone()),
            None => serde_json::Value::Null,
        };
        map.insert(col.clone(), json_val);
    }
    serde_yml::to_string(&serde_json::Value::Object(map))
        .map_err(|e| FsError::SerializationError(e.to_string()))
}
