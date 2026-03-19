use crate::db::queries::{introspection, rows};
use crate::db::types::PrimaryKeyInfo;
use crate::error::FsError;
use crate::fs::inode::{NodeIdentity, SpecialDirKind};
use crate::fs::nodes::{DirEntry, NodeContext};

const SPECIAL_DIRS: &[(&str, SpecialDirKind)] = &[
    (".info", SpecialDirKind::Info),
    (".export", SpecialDirKind::Export),
    (".filter", SpecialDirKind::Filter),
    (".order", SpecialDirKind::Order),
    (".indexes", SpecialDirKind::Indexes),
];

pub async fn lookup(
    schema: &str,
    table: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    // Check special directories first
    for (dir_name, kind) in SPECIAL_DIRS {
        if name == *dir_name {
            return Ok(NodeIdentity::SpecialDir {
                schema: schema.to_string(),
                table: table.to_string(),
                kind: kind.clone(),
            });
        }
    }

    // Check if it's a row
    let pk = get_pk(schema, table, ctx).await?;
    if pk.column_names.is_empty() {
        return Err(FsError::NotFound);
    }

    // For single-column PKs, name is the value directly
    // For composite PKs, name is "v1,v2" format
    Ok(NodeIdentity::Row {
        schema: schema.to_string(),
        table: table.to_string(),
        pk_display: name.to_string(),
    })
}

pub async fn readdir(
    schema: &str,
    table: &str,
    offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let mut entries = Vec::new();

    // Add special dirs first (at offset 0)
    if offset == 0 {
        for (dir_name, kind) in SPECIAL_DIRS {
            entries.push(DirEntry {
                name: dir_name.to_string(),
                identity: NodeIdentity::SpecialDir {
                    schema: schema.to_string(),
                    table: table.to_string(),
                    kind: kind.clone(),
                },
                kind: fuser::FileType::Directory,
            });
        }
    }

    // Add rows
    let pk = get_pk(schema, table, ctx).await?;
    if !pk.column_names.is_empty() {
        let row_offset = if offset == 0 {
            0
        } else {
            offset - SPECIAL_DIRS.len() as i64
        };
        let row_offset = row_offset.max(0);
        let row_ids = rows::list_rows(
            ctx.pool,
            schema,
            table,
            &pk.column_names,
            ctx.config.page_size as i64,
            row_offset,
        )
        .await?;
        for row_id in row_ids {
            entries.push(DirEntry {
                name: row_id.display_name.clone(),
                identity: NodeIdentity::Row {
                    schema: schema.to_string(),
                    table: table.to_string(),
                    pk_display: row_id.display_name,
                },
                kind: fuser::FileType::Directory,
            });
        }
    }

    Ok(entries)
}

async fn get_pk(schema: &str, table: &str, ctx: &NodeContext<'_>) -> Result<PrimaryKeyInfo, FsError> {
    if let Some(cached) = ctx.cache.get_primary_key(schema, table) {
        return Ok(cached);
    }
    let pk = introspection::get_primary_key(ctx.pool, schema, table).await?;
    ctx.cache.set_primary_key(schema, table, pk.clone());
    Ok(pk)
}
