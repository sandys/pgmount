use crate::db::queries::{introspection, rows};
use crate::error::FsError;
use crate::fs::inode::{FilterStage, NodeIdentity};
use crate::fs::nodes::{DirEntry, NodeContext};

pub async fn lookup_root(
    schema: &str,
    table: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    let columns = introspection::list_columns(ctx.pool, schema, table).await?;
    if columns.iter().any(|c| c.name == name) {
        Ok(NodeIdentity::FilterDir {
            schema: schema.to_string(),
            table: table.to_string(),
            stage: FilterStage::Column {
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
            identity: NodeIdentity::FilterDir {
                schema: schema.to_string(),
                table: table.to_string(),
                stage: FilterStage::Column {
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
    value: &str,
    _ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    Ok(NodeIdentity::FilterDir {
        schema: schema.to_string(),
        table: table.to_string(),
        stage: FilterStage::Value {
            column: column.to_string(),
            value: value.to_string(),
        },
    })
}

pub async fn readdir_column(
    _schema: &str,
    _table: &str,
    _column: &str,
    _offset: i64,
    _ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    Ok(vec![])
}

pub async fn lookup_value(
    schema: &str,
    table: &str,
    _column: &str,
    _value: &str,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    let pk = super::table::get_pk(schema, table, ctx).await?;
    if pk.column_names.is_empty() {
        return Err(FsError::NotFound);
    }
    Ok(NodeIdentity::Row {
        schema: schema.to_string(),
        table: table.to_string(),
        pk_display: name.to_string(),
    })
}

pub async fn readdir_value(
    schema: &str,
    table: &str,
    column: &str,
    value: &str,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let pk = super::table::get_pk(schema, table, ctx).await?;
    if pk.column_names.is_empty() {
        return Ok(vec![]);
    }

    let where_clause = format!("{}::text = $1", crate::db::queries::quote_ident(column));
    let filter_value_param: &(dyn tokio_postgres::types::ToSql + Sync) = &value.to_string();

    let filtered = rows::query_rows(
        ctx.pool,
        schema,
        table,
        &pk.column_names,
        ctx.config.page_size as i64,
        0,
        Some(&where_clause),
        None,
        &[filter_value_param],
    )
    .await?;

    Ok(filtered
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
