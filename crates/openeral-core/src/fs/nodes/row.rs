use crate::db::queries::introspection;
use crate::db::types::ColumnInfo;
use crate::error::FsError;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};

pub async fn lookup(
    schema: &str,
    table: &str,
    pk_display: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    // Check for format files
    match name {
        "row.json" | "row.csv" | "row.yaml" => {
            let format = name.strip_prefix("row.").unwrap().to_string();
            return Ok(NodeIdentity::RowFile {
                schema: schema.to_string(),
                table: table.to_string(),
                pk_display: pk_display.to_string(),
                format,
            });
        }
        _ => {}
    }

    // Check columns
    let columns = get_columns(schema, table, ctx).await?;
    if columns.iter().any(|c| c.name == name) {
        Ok(NodeIdentity::Column {
            schema: schema.to_string(),
            table: table.to_string(),
            pk_display: pk_display.to_string(),
            column: name.to_string(),
        })
    } else {
        Err(FsError::NotFound)
    }
}

pub async fn readdir(
    schema: &str,
    table: &str,
    pk_display: &str,
    offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let columns = get_columns(schema, table, ctx).await?;
    let mut entries: Vec<DirEntry> = columns
        .into_iter()
        .skip(offset as usize)
        .map(|c| DirEntry {
            name: c.name.clone(),
            identity: NodeIdentity::Column {
                schema: schema.to_string(),
                table: table.to_string(),
                pk_display: pk_display.to_string(),
                column: c.name,
            },
            kind: fuser::FileType::RegularFile,
        })
        .collect();

    // Add row format files
    if offset == 0 {
        for fmt in &["json", "csv", "yaml"] {
            entries.push(DirEntry {
                name: format!("row.{}", fmt),
                identity: NodeIdentity::RowFile {
                    schema: schema.to_string(),
                    table: table.to_string(),
                    pk_display: pk_display.to_string(),
                    format: fmt.to_string(),
                },
                kind: fuser::FileType::RegularFile,
            });
        }
    }

    Ok(entries)
}

async fn get_columns(schema: &str, table: &str, ctx: &NodeContext<'_>) -> Result<Vec<ColumnInfo>, FsError> {
    if let Some(cached) = ctx.cache.get_columns(schema, table) {
        return Ok(cached);
    }
    let columns = introspection::list_columns(ctx.pool, schema, table).await?;
    ctx.cache.set_columns(schema, table, columns.clone());
    Ok(columns)
}
