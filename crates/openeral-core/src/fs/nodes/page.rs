use super::table;
use crate::db::queries::rows;
use crate::error::FsError;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};

/// Lookup a child inside page_N/ — these are row directories
pub async fn lookup(
    schema: &str,
    table_name: &str,
    _page: u64,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    let pk = table::get_pk(schema, table_name, ctx).await?;
    if pk.column_names.is_empty() {
        return Err(FsError::NotFound);
    }
    // The child name is a row pk_display
    Ok(NodeIdentity::Row {
        schema: schema.to_string(),
        table: table_name.to_string(),
        pk_display: name.to_string(),
    })
}

/// List rows for page_N/
pub async fn readdir(
    schema: &str,
    table_name: &str,
    page: u64,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let pk = table::get_pk(schema, table_name, ctx).await?;
    if pk.column_names.is_empty() {
        return Ok(vec![]);
    }

    let page_size = ctx.config.page_size as i64;
    let row_offset = ((page as i64) - 1) * page_size;

    let row_ids = rows::list_rows(
        ctx.pool,
        schema,
        table_name,
        &pk.column_names,
        page_size,
        row_offset,
    )
    .await?;

    Ok(row_ids
        .iter()
        .map(|row_id| DirEntry {
            name: row_id.display_name.clone(),
            identity: NodeIdentity::Row {
                schema: schema.to_string(),
                table: table_name.to_string(),
                pk_display: row_id.display_name.clone(),
            },
            kind: fuser::FileType::Directory,
        })
        .collect())
}
