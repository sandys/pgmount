use crate::db::queries::{introspection, rows};
use crate::error::FsError;
use crate::fs::inode::{NodeIdentity, OrderStage};
use crate::fs::nodes::{DirEntry, NodeContext};

pub async fn lookup_root(
    schema: &str,
    table: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    let columns = introspection::list_columns(ctx.pool, schema, table).await?;
    if columns.iter().any(|c| c.name == name) {
        Ok(NodeIdentity::OrderDir {
            schema: schema.to_string(),
            table: table.to_string(),
            stage: OrderStage::Column {
                column: name.to_string(),
            },
        })
    } else {
        Err(FsError::NotFound)
    }
}

pub async fn readdir_root(
    schema: &str,
    table: &str,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let columns = introspection::list_columns(ctx.pool, schema, table).await?;
    Ok(columns
        .iter()
        .map(|c| DirEntry {
            name: c.name.clone(),
            identity: NodeIdentity::OrderDir {
                schema: schema.to_string(),
                table: table.to_string(),
                stage: OrderStage::Column {
                    column: c.name.clone(),
                },
            },
            kind: fuser::FileType::Directory,
        })
        .collect())
}

pub async fn lookup_column(
    schema: &str,
    table: &str,
    column: &str,
    name: &str,
    _ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    match name {
        "asc" | "desc" => Ok(NodeIdentity::OrderDir {
            schema: schema.to_string(),
            table: table.to_string(),
            stage: OrderStage::Direction {
                column: column.to_string(),
                dir: name.to_string(),
            },
        }),
        _ => Err(FsError::NotFound),
    }
}

pub async fn readdir_column(
    schema: &str,
    table: &str,
    column: &str,
    _offset: i64,
    _ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    Ok(["asc", "desc"]
        .iter()
        .map(|dir| DirEntry {
            name: dir.to_string(),
            identity: NodeIdentity::OrderDir {
                schema: schema.to_string(),
                table: table.to_string(),
                stage: OrderStage::Direction {
                    column: column.to_string(),
                    dir: dir.to_string(),
                },
            },
            kind: fuser::FileType::Directory,
        })
        .collect())
}

pub async fn lookup_direction(
    schema: &str,
    table: &str,
    _column: &str,
    _dir: &str,
    name: &str,
    _ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    Ok(NodeIdentity::Row {
        schema: schema.to_string(),
        table: table.to_string(),
        pk_display: name.to_string(),
    })
}

pub async fn readdir_direction(
    schema: &str,
    table: &str,
    column: &str,
    dir: &str,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let pk = super::table::get_pk(schema, table, ctx).await?;
    if pk.column_names.is_empty() {
        return Ok(vec![]);
    }

    let dir_sql = if dir == "desc" { "DESC" } else { "ASC" };
    let order_clause = format!("{} {}", crate::db::queries::quote_ident(column), dir_sql);

    let ordered = rows::query_rows(
        ctx.pool,
        schema,
        table,
        &pk.column_names,
        ctx.config.page_size as i64,
        0,
        None,
        Some(&order_clause),
        &[],
    )
    .await?;

    Ok(ordered
        .iter()
        .map(|row_id| DirEntry {
            name: row_id.display_name.clone(),
            identity: NodeIdentity::Row {
                schema: schema.to_string(),
                table: table.to_string(),
                pk_display: row_id.display_name.clone(),
            },
            kind: fuser::FileType::Directory,
        })
        .collect())
}
