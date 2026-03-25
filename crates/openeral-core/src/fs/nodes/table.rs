use crate::db::queries::introspection;
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
    _ctx: &NodeContext<'_>,
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

    // Check if it's a page directory (page_N)
    if let Some(page_str) = name.strip_prefix("page_") {
        if let Ok(page) = page_str.parse::<u64>() {
            if page >= 1 {
                return Ok(NodeIdentity::PageDir {
                    schema: schema.to_string(),
                    table: table.to_string(),
                    page,
                });
            }
        }
    }

    Err(FsError::NotFound)
}

pub async fn readdir(
    schema: &str,
    table: &str,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let mut entries = Vec::new();

    // Add special dirs
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

    // Calculate number of pages
    let pk = get_pk(schema, table, ctx).await?;
    if !pk.column_names.is_empty() {
        let count = crate::db::queries::stats::get_exact_row_count(ctx.pool, schema, table)
            .await
            .unwrap_or(0);
        let page_size = ctx.config.page_size as i64;
        let num_pages = if count == 0 {
            0
        } else {
            ((count - 1) / page_size) + 1
        };
        for p in 1..=num_pages {
            entries.push(DirEntry {
                name: format!("page_{}", p),
                identity: NodeIdentity::PageDir {
                    schema: schema.to_string(),
                    table: table.to_string(),
                    page: p as u64,
                },
                kind: fuser::FileType::Directory,
            });
        }
    }

    Ok(entries)
}

pub(crate) async fn get_pk(
    schema: &str,
    table: &str,
    ctx: &NodeContext<'_>,
) -> Result<PrimaryKeyInfo, FsError> {
    if let Some(cached) = ctx.cache.get_primary_key(schema, table) {
        return Ok(cached);
    }
    let pk = introspection::get_primary_key(ctx.pool, schema, table).await?;
    ctx.cache.set_primary_key(schema, table, pk.clone());
    Ok(pk)
}
