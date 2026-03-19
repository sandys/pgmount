pub mod root;
pub mod schema;
pub mod table;
pub mod row;
pub mod column;
pub mod row_file;

use crate::config::types::MountConfig;
use crate::error::FsError;
use crate::fs::attr;
use crate::fs::cache::MetadataCache;
use crate::fs::inode::{InodeTable, NodeIdentity};
use deadpool_postgres::Pool;
use fuser::FileAttr;

/// Context provided to node operations
pub struct NodeContext<'a> {
    pub pool: &'a Pool,
    pub cache: &'a MetadataCache,
    pub inodes: &'a InodeTable,
    pub config: &'a MountConfig,
}

/// A directory entry returned by readdir
#[derive(Debug)]
pub struct DirEntry {
    pub name: String,
    pub identity: NodeIdentity,
    pub kind: fuser::FileType,
}

/// Dispatch node operations based on NodeIdentity.
/// This avoids async trait complexity by using direct async functions.
pub async fn node_getattr(
    identity: &NodeIdentity,
    ino: u64,
    ctx: &NodeContext<'_>,
) -> Result<FileAttr, FsError> {
    match identity {
        NodeIdentity::Root => root::getattr(ino, ctx).await,
        NodeIdentity::Schema { .. } => schema::getattr(ino, ctx).await,
        NodeIdentity::Table { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::Row { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::Column {
            schema,
            table,
            pk_display,
            column,
        } => column::getattr(ino, schema, table, pk_display, column, ctx).await,
        NodeIdentity::RowFile {
            schema,
            table,
            pk_display,
            format,
        } => row_file::getattr(ino, schema, table, pk_display, format, ctx).await,
        NodeIdentity::SpecialDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::InfoFile { .. } => {
            // Info files have estimated size
            Ok(attr::file_attr(ino, 4096))
        }
        NodeIdentity::ExportFile { .. } => Ok(attr::file_attr(ino, 4096)),
        NodeIdentity::IndexDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::IndexFile { .. } => Ok(attr::file_attr(ino, 4096)),
        NodeIdentity::FilterDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::OrderDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::LimitDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::ByIndexDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::ViewsDir { .. } => Ok(attr::dir_attr(ino)),
        NodeIdentity::View { .. } => Ok(attr::dir_attr(ino)),
    }
}

pub async fn node_lookup(
    identity: &NodeIdentity,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    match identity {
        NodeIdentity::Root => root::lookup(name, ctx).await,
        NodeIdentity::Schema { name: schema } => schema::lookup(schema, name, ctx).await,
        NodeIdentity::Table { schema, table } => table::lookup(schema, table, name, ctx).await,
        NodeIdentity::Row {
            schema,
            table,
            pk_display,
        } => row::lookup(schema, table, pk_display, name, ctx).await,
        _ => Err(FsError::NotFound),
    }
}

pub async fn node_readdir(
    identity: &NodeIdentity,
    offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    match identity {
        NodeIdentity::Root => root::readdir(offset, ctx).await,
        NodeIdentity::Schema { name } => schema::readdir(name, offset, ctx).await,
        NodeIdentity::Table { schema, table } => table::readdir(schema, table, offset, ctx).await,
        NodeIdentity::Row {
            schema,
            table,
            pk_display,
        } => row::readdir(schema, table, pk_display, offset, ctx).await,
        _ => Ok(vec![]),
    }
}

pub async fn node_read(
    identity: &NodeIdentity,
    offset: i64,
    size: u32,
    ctx: &NodeContext<'_>,
) -> Result<Vec<u8>, FsError> {
    match identity {
        NodeIdentity::Column {
            schema,
            table,
            pk_display,
            column,
        } => column::read(schema, table, pk_display, column, offset, size, ctx).await,
        NodeIdentity::RowFile {
            schema,
            table,
            pk_display,
            format,
        } => row_file::read(schema, table, pk_display, format, offset, size, ctx).await,
        _ => Err(FsError::IsADirectory),
    }
}

/// Check if a node identity represents a directory
pub fn is_directory(identity: &NodeIdentity) -> bool {
    matches!(
        identity,
        NodeIdentity::Root
            | NodeIdentity::Schema { .. }
            | NodeIdentity::Table { .. }
            | NodeIdentity::Row { .. }
            | NodeIdentity::SpecialDir { .. }
            | NodeIdentity::FilterDir { .. }
            | NodeIdentity::OrderDir { .. }
            | NodeIdentity::LimitDir { .. }
            | NodeIdentity::ByIndexDir { .. }
            | NodeIdentity::IndexDir { .. }
            | NodeIdentity::ViewsDir { .. }
    )
}
