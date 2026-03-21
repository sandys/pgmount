use crate::error::FsError;
use crate::fs::inode::NodeIdentity;
use crate::fs::nodes::{DirEntry, NodeContext};
use crate::db::queries::rows;

const EXPORT_FORMATS: &[&str] = &["json", "csv", "yaml"];

/// Lookup in .export/ — data.json, data.csv, data.yaml are now directories
pub async fn lookup(
    schema: &str,
    table_name: &str,
    name: &str,
    _ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    for fmt in EXPORT_FORMATS {
        let dir_name = format!("data.{}", fmt);
        if name == dir_name {
            return Ok(NodeIdentity::ExportDir {
                schema: schema.to_string(),
                table: table_name.to_string(),
                format: fmt.to_string(),
            });
        }
    }
    Err(FsError::NotFound)
}

/// List export format directories
pub async fn readdir(
    schema: &str,
    table_name: &str,
    _offset: i64,
    _ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    Ok(EXPORT_FORMATS.iter().map(|fmt| {
        DirEntry {
            name: format!("data.{}", fmt),
            identity: NodeIdentity::ExportDir {
                schema: schema.to_string(),
                table: table_name.to_string(),
                format: fmt.to_string(),
            },
            kind: fuser::FileType::Directory,
        }
    }).collect())
}

/// Lookup in .export/data.json/ — page_1.json, page_2.json, etc
pub async fn lookup_export_dir(
    schema: &str,
    table_name: &str,
    format: &str,
    name: &str,
    _ctx: &NodeContext<'_>,
) -> Result<NodeIdentity, FsError> {
    // Expect page_N.fmt format
    let expected_suffix = format!(".{}", format);
    if let Some(page_part) = name.strip_suffix(&expected_suffix) {
        if let Some(page_str) = page_part.strip_prefix("page_") {
            if let Ok(page) = page_str.parse::<u64>() {
                if page >= 1 {
                    return Ok(NodeIdentity::ExportPageFile {
                        schema: schema.to_string(),
                        table: table_name.to_string(),
                        format: format.to_string(),
                        page,
                    });
                }
            }
        }
    }
    Err(FsError::NotFound)
}

/// List page files in .export/data.json/
pub async fn readdir_export_dir(
    schema: &str,
    table_name: &str,
    format: &str,
    _offset: i64,
    ctx: &NodeContext<'_>,
) -> Result<Vec<DirEntry>, FsError> {
    let count = crate::db::queries::stats::get_exact_row_count(ctx.pool, schema, table_name).await.unwrap_or(0);
    let page_size = ctx.config.page_size as i64;
    let num_pages = if count == 0 { 1 } else { ((count - 1) / page_size) + 1 };
    // Always show at least page_1 even if empty (for discoverability)

    Ok((1..=num_pages).map(|p| {
        DirEntry {
            name: format!("page_{}.{}", p, format),
            identity: NodeIdentity::ExportPageFile {
                schema: schema.to_string(),
                table: table_name.to_string(),
                format: format.to_string(),
                page: p as u64,
            },
            kind: fuser::FileType::RegularFile,
        }
    }).collect())
}

/// Read a page of exported data
pub async fn read_export_page(
    schema: &str,
    table_name: &str,
    format: &str,
    page: u64,
    offset: i64,
    size: u32,
    ctx: &NodeContext<'_>,
) -> Result<Vec<u8>, FsError> {
    let page_size = ctx.config.page_size as i64;
    let row_offset = ((page as i64) - 1) * page_size;

    let (_col_names, all_rows) = rows::get_all_rows_as_text(
        ctx.pool, schema, table_name,
        page_size, row_offset,
    ).await?;

    let content = match format {
        "json" => crate::format::json::format_rows(&all_rows)?,
        "csv" => crate::format::csv::format_rows(&all_rows)?,
        "yaml" => crate::format::yaml::format_rows(&all_rows)?,
        _ => return Err(FsError::InvalidArgument(format!("Unknown format: {}", format))),
    };

    let bytes = content.as_bytes();
    let offset = offset as usize;
    if offset >= bytes.len() {
        return Ok(vec![]);
    }
    let end = (offset + size as usize).min(bytes.len());
    Ok(bytes[offset..end].to_vec())
}
