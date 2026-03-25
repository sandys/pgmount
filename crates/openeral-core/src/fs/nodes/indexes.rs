use crate::db::queries::indexes as idx_queries;
use crate::error::FsError;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};

pub async fn lookup(
    schema: &str,
    table: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    let indexes = idx_queries::list_indexes(ctx.pool, schema, table).await?;
    if indexes.iter().any(|i| i.name == name) {
        Ok(NodeIdentity::IndexFile {
            schema: schema.to_string(),
            table: table.to_string(),
            index_name: name.to_string(),
        })
    } else {
        Err(FsError::NotFound)
    }
}

pub async fn readdir(
    schema: &str,
    table: &str,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let indexes = idx_queries::list_indexes(ctx.pool, schema, table).await?;
    Ok(indexes
        .iter()
        .map(|idx| DirEntry {
            name: idx.name.clone(),
            identity: NodeIdentity::IndexFile {
                schema: schema.to_string(),
                table: table.to_string(),
                index_name: idx.name.clone(),
            },
            kind: fuser::FileType::RegularFile,
        })
        .collect())
}

pub async fn read(
    schema: &str,
    table: &str,
    index_name: &str,
    offset: i64,
    size: u32,
    ctx: &NodeContext<'_>,
) -> Result<Vec<u8>, FsError> {
    let indexes = idx_queries::list_indexes(ctx.pool, schema, table).await?;
    let idx = indexes
        .iter()
        .find(|i| i.name == index_name)
        .ok_or(FsError::NotFound)?;

    let content = format!(
        "Name: {}\nUnique: {}\nPrimary: {}\nColumns: {}\nDefinition: {}\n",
        idx.name,
        idx.is_unique,
        idx.is_primary,
        idx.columns.join(", "),
        idx.definition,
    );

    let bytes = content.as_bytes();
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Ok(vec![]);
    }
    let end = (offset + size as usize).min(bytes.len());
    Ok(bytes[offset..end].to_vec())
}
