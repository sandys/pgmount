pub mod root;
pub mod schema;
pub mod table;
pub mod row;
pub mod column;
pub mod row_file;
pub mod info;
pub mod export;
pub mod indexes;
pub mod filter;
pub mod order;
pub mod page;

use crate::config::types::MountConfig;
use crate::error::FsError;
use crate::fs::attr;
use crate::fs::cache::MetadataCache;
use crate::fs::inode::{
    FilterStage, InodeTable, NodeIdentity, OrderStage, SpecialDirKind,
};
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
pub async fn node_getattr(
    identity: &NodeIdentity,
    ino: u64,
    _ctx: &NodeContext<'_>,
) -> Result<FileAttr, FsError> {
    match identity {
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
        | NodeIdentity::View { .. }
        | NodeIdentity::PageDir { .. }
        | NodeIdentity::ExportDir { .. } => Ok(attr::dir_attr(ino)),

        NodeIdentity::Column { .. }
        | NodeIdentity::RowFile { .. }
        | NodeIdentity::InfoFile { .. }
        | NodeIdentity::ExportFile { .. }
        | NodeIdentity::ExportPageFile { .. }
        | NodeIdentity::IndexFile { .. } => Ok(attr::file_attr(ino, 4096)),
    }
}

pub async fn node_lookup(
    identity: &NodeIdentity,
    name: &str,
    ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    match identity {
        NodeIdentity::Root => root::lookup(name, ctx).await,
        NodeIdentity::Schema { name: schema_name } => {
            schema::lookup(schema_name, name, ctx).await
        }
        NodeIdentity::Table { schema, table } => table::lookup(schema, table, name, ctx).await,
        NodeIdentity::Row {
            schema,
            table,
            pk_display,
        } => row::lookup(schema, table, pk_display, name, ctx).await,

        // Special directories dispatch to their respective node modules
        NodeIdentity::SpecialDir {
            schema,
            table,
            kind,
        } => match kind {
            SpecialDirKind::Info => info::lookup(schema, table, name, ctx).await,
            SpecialDirKind::Export => export::lookup(schema, table, name, ctx).await,
            SpecialDirKind::Indexes => indexes::lookup(schema, table, name, ctx).await,
            SpecialDirKind::Filter => filter::lookup_root(schema, table, name, ctx).await,
            SpecialDirKind::Order => order::lookup_root(schema, table, name, ctx).await,
            _ => Err(FsError::NotFound),
        },

        // Filter pipeline stages
        NodeIdentity::FilterDir {
            schema,
            table,
            stage,
        } => match stage {
            FilterStage::Root => filter::lookup_root(schema, table, name, ctx).await,
            FilterStage::Column { column } => {
                filter::lookup_column(schema, table, column, name, ctx).await
            }
            FilterStage::Value {
                column,
                value,
            } => {
                filter::lookup_value(schema, table, column, value, name, ctx).await
            }
        },

        // Order pipeline stages
        NodeIdentity::OrderDir {
            schema,
            table,
            stage,
        } => match stage {
            OrderStage::Root => order::lookup_root(schema, table, name, ctx).await,
            OrderStage::Column { column } => {
                order::lookup_column(schema, table, column, name, ctx).await
            }
            OrderStage::Direction { column, dir } => {
                order::lookup_direction(schema, table, column, dir, name, ctx).await
            }
        },

        // Index directory
        NodeIdentity::IndexDir { schema, table } => {
            indexes::lookup(schema, table, name, ctx).await
        }

        // Page directory (page_N/) contains row dirs
        NodeIdentity::PageDir { schema, table, page } => {
            page::lookup(schema, table, *page, name, ctx).await
        }

        // Export format directory (data.json/) contains page files
        NodeIdentity::ExportDir { schema, table, format } => {
            export::lookup_export_dir(schema, table, format, name, ctx).await
        }

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

        // Special directories
        NodeIdentity::SpecialDir {
            schema,
            table,
            kind,
        } => match kind {
            SpecialDirKind::Info => info::readdir(schema, table, offset, ctx).await,
            SpecialDirKind::Export => export::readdir(schema, table, offset, ctx).await,
            SpecialDirKind::Indexes => indexes::readdir(schema, table, offset, ctx).await,
            SpecialDirKind::Filter => filter::readdir_root(schema, table, offset, ctx).await,
            SpecialDirKind::Order => order::readdir_root(schema, table, offset, ctx).await,
            _ => Ok(vec![]),
        },

        // Filter pipeline stages
        NodeIdentity::FilterDir {
            schema,
            table,
            stage,
        } => match stage {
            FilterStage::Root => filter::readdir_root(schema, table, offset, ctx).await,
            FilterStage::Column { column } => {
                filter::readdir_column(schema, table, column, offset, ctx).await
            }
            FilterStage::Value { column, value } => {
                filter::readdir_value(schema, table, column, value, offset, ctx).await
            }
        },

        // Order pipeline stages
        NodeIdentity::OrderDir {
            schema,
            table,
            stage,
        } => match stage {
            OrderStage::Root => order::readdir_root(schema, table, offset, ctx).await,
            OrderStage::Column { column } => {
                order::readdir_column(schema, table, column, offset, ctx).await
            }
            OrderStage::Direction { column, dir } => {
                order::readdir_direction(schema, table, column, dir, offset, ctx).await
            }
        },

        // Index directory
        NodeIdentity::IndexDir { schema, table } => {
            indexes::readdir(schema, table, offset, ctx).await
        }

        // Page directory (page_N/) lists rows
        NodeIdentity::PageDir { schema, table, page } => {
            page::readdir(schema, table, *page, offset, ctx).await
        }

        // Export format directory (data.json/) lists page files
        NodeIdentity::ExportDir { schema, table, format } => {
            export::readdir_export_dir(schema, table, format, offset, ctx).await
        }

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
        NodeIdentity::InfoFile {
            schema,
            table,
            filename,
        } => info::read(schema, table, filename, offset, size, ctx).await,
        NodeIdentity::ExportFile {
            schema,
            table,
            format,
        } => export::read_export_page(schema, table, format, 1, offset, size, ctx).await,
        NodeIdentity::ExportPageFile {
            schema,
            table,
            format,
            page,
        } => export::read_export_page(schema, table, format, *page, offset, size, ctx).await,
        NodeIdentity::IndexFile {
            schema,
            table,
            index_name,
        } => indexes::read(schema, table, index_name, offset, size, ctx).await,
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
            | NodeIdentity::PageDir { .. }
            | NodeIdentity::ExportDir { .. }
    )
}
