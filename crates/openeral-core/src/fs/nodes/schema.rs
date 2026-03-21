use crate::db::queries::introspection;
use crate::db::types::TableInfo;
use crate::error::FsError;
use crate::fs::attr;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};
use fuser::FileAttr;

pub async fn getattr(ino: u64, _ctx: &NodeContext<'_>) -> Result<FileAttr, FsError> {
    Ok(attr::dir_attr(ino))
}

pub async fn lookup(
    schema: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    let tables = get_tables(schema, ctx).await?;
    if tables.iter().any(|t| t.name == name) {
        Ok(NodeIdentity::Table {
            schema: schema.to_string(),
            table: name.to_string(),
        })
    } else {
        Err(FsError::NotFound)
    }
}

pub async fn readdir(
    schema: &str,
    offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let tables = get_tables(schema, ctx).await?;
    let entries: Vec<DirEntry> = tables
        .into_iter()
        .skip(offset as usize)
        .map(|t| DirEntry {
            name: t.name.clone(),
            identity: NodeIdentity::Table {
                schema: schema.to_string(),
                table: t.name,
            },
            kind: fuser::FileType::Directory,
        })
        .collect();
    Ok(entries)
}

async fn get_tables(schema: &str, ctx: &NodeContext<'_>) -> Result<Vec<TableInfo>, FsError> {
    if let Some(cached) = ctx.cache.get_tables(schema) {
        return Ok(cached);
    }
    let tables = introspection::list_tables(ctx.pool, schema).await?;
    ctx.cache.set_tables(schema, tables.clone());
    Ok(tables)
}
