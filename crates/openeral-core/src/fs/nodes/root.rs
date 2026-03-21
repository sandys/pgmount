use crate::db::queries::introspection;
use crate::db::types::SchemaInfo;
use crate::error::FsError;
use crate::fs::attr;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};
use fuser::FileAttr;

pub async fn getattr(ino: u64, _ctx: &NodeContext<'_>) -> Result<FileAttr, FsError> {
    Ok(attr::dir_attr(ino))
}

pub async fn lookup(name: &str, ctx: &NodeContext<'_>) -> Result<NodeIdentity, FsError> {
    let schemas = get_schemas(ctx).await?;
    if schemas.iter().any(|s| s.name == name) {
        Ok(NodeIdentity::Schema {
            name: name.to_string(),
        })
    } else {
        Err(FsError::NotFound)
    }
}

pub async fn readdir(offset: i64, ctx: &NodeContext<'_>) -> Result<Vec<DirEntry>, FsError> {
    let schemas = get_schemas(ctx).await?;
    let entries: Vec<DirEntry> = schemas
        .into_iter()
        .skip(offset as usize)
        .map(|s| DirEntry {
            name: s.name.clone(),
            identity: NodeIdentity::Schema { name: s.name },
            kind: fuser::FileType::Directory,
        })
        .collect();
    Ok(entries)
}

async fn get_schemas(ctx: &NodeContext<'_>) -> Result<Vec<SchemaInfo>, FsError> {
    if let Some(cached) = ctx.cache.get_schemas() {
        return Ok(cached);
    }
    let schemas = introspection::list_schemas(ctx.pool).await?;
    // Filter schemas if configured
    let schemas = if let Some(ref filter) = ctx.config.schemas {
        schemas
            .into_iter()
            .filter(|s| filter.contains(&s.name))
            .collect()
    } else {
        schemas
    };
    ctx.cache.set_schemas(schemas.clone());
    Ok(schemas)
}
