use crate::error::FsError;
use std::collections::BTreeMap;

/// Format a single row as YAML.
pub fn format_row(row_data: &[(String, Option<String>)]) -> Result<String, FsError> {
    let mut map = BTreeMap::new();
    for (col, val) in row_data {
        map.insert(
            col.clone(),
            val.clone().unwrap_or_else(|| "null".to_string()),
        );
    }
    serde_yml::to_string(&map).map_err(|e| FsError::SerializationError(e.to_string()))
}

/// Format multiple rows as YAML.
pub fn format_rows(rows: &[Vec<(String, Option<String>)>]) -> Result<String, FsError> {
    let mut arr = Vec::new();
    for row_data in rows {
        let mut map = BTreeMap::new();
        for (col, val) in row_data {
            map.insert(
                col.clone(),
                val.clone().unwrap_or_else(|| "null".to_string()),
            );
        }
        arr.push(map);
    }
    serde_yml::to_string(&arr).map_err(|e| FsError::SerializationError(e.to_string()))
}
