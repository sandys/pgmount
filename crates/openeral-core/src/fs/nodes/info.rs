use crate::db::queries::{introspection, stats};
use crate::error::FsError;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};

const INFO_FILES: &[&str] = &["columns.json", "schema.sql", "count", "primary_key"];

pub async fn lookup(
    schema: &str,
    table: &str,
    name: &str,
    _ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    if INFO_FILES.contains(&name) {
        Ok(NodeIdentity::InfoFile {
            schema: schema.to_string(),
            table: table.to_string(),
            filename: name.to_string(),
        })
    } else {
        Err(FsError::NotFound)
    }
}

pub async fn readdir(
    schema: &str,
    table: &str,
    _offset: i64,
    _ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    Ok(INFO_FILES
        .iter()
        .map(|f| DirEntry {
            name: f.to_string(),
            identity: NodeIdentity::InfoFile {
                schema: schema.to_string(),
                table: table.to_string(),
                filename: f.to_string(),
            },
            kind: fuser::FileType::RegularFile,
        })
        .collect())
}

pub async fn read(
    schema: &str,
    table: &str,
    filename: &str,
    offset: i64,
    size: u32,
    ctx: &NodeContext<'_>,
) -> Result<Vec<u8>, FsError> {
    let content = match filename {
        "columns.json" => {
            let columns = introspection::list_columns(ctx.pool, schema, table).await?;
            let json_cols: Vec<serde_json::Value> = columns
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "name": c.name,
                        "data_type": c.data_type,
                        "is_nullable": c.is_nullable,
                        "column_default": c.column_default,
                        "ordinal_position": c.ordinal_position,
                    })
                })
                .collect();
            serde_json::to_string_pretty(&json_cols)
                .map_err(|e| FsError::SerializationError(e.to_string()))?
                + "\n"
        }
        "schema.sql" => {
            let columns = introspection::list_columns(ctx.pool, schema, table).await?;
            let pk = introspection::get_primary_key(ctx.pool, schema, table).await?;
            let mut ddl = format!(
                "CREATE TABLE {}.{} (\n",
                crate::db::queries::quote_ident(schema),
                crate::db::queries::quote_ident(table)
            );
            for (i, col) in columns.iter().enumerate() {
                ddl.push_str(&format!(
                    "    {} {}",
                    crate::db::queries::quote_ident(&col.name),
                    col.data_type
                ));
                if !col.is_nullable {
                    ddl.push_str(" NOT NULL");
                }
                if let Some(ref def) = col.column_default {
                    ddl.push_str(&format!(" DEFAULT {}", def));
                }
                if i < columns.len() - 1 || !pk.column_names.is_empty() {
                    ddl.push(',');
                }
                ddl.push('\n');
            }
            if !pk.column_names.is_empty() {
                let pk_cols: Vec<String> = pk
                    .column_names
                    .iter()
                    .map(|c| crate::db::queries::quote_ident(c))
                    .collect();
                ddl.push_str(&format!("    PRIMARY KEY ({})\n", pk_cols.join(", ")));
            }
            ddl.push_str(");\n");
            ddl
        }
        "count" => {
            let count = stats::get_exact_row_count(ctx.pool, schema, table).await?;
            format!("{}\n", count)
        }
        "primary_key" => {
            let pk = introspection::get_primary_key(ctx.pool, schema, table).await?;
            pk.column_names.join("\n") + "\n"
        }
        _ => return Err(FsError::NotFound),
    };
    let bytes = content.as_bytes();
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Ok(vec![]);
    }
    let end = (offset + size as usize).min(bytes.len());
    Ok(bytes[offset..end].to_vec())
}
