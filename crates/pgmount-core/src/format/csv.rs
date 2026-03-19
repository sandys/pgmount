use crate::error::FsError;

/// Format a single row as CSV (header + one data line).
pub fn format_row(row_data: &[(String, Option<String>)]) -> Result<String, FsError> {
    let mut wtr = csv::Writer::from_writer(vec![]);
    // Header
    let headers: Vec<&str> = row_data.iter().map(|(col, _)| col.as_str()).collect();
    wtr.write_record(&headers).map_err(|e| FsError::SerializationError(e.to_string()))?;
    // Data
    let values: Vec<String> = row_data.iter().map(|(_, val)| {
        match val {
            Some(v) => v.clone(),
            None => "NULL".to_string(),
        }
    }).collect();
    wtr.write_record(&values).map_err(|e| FsError::SerializationError(e.to_string()))?;
    wtr.flush().map_err(|e| FsError::SerializationError(e.to_string()))?;
    String::from_utf8(wtr.into_inner().map_err(|e| FsError::SerializationError(e.to_string()))?)
        .map_err(|e| FsError::SerializationError(e.to_string()))
}

/// Format multiple rows as CSV.
pub fn format_rows(rows: &[Vec<(String, Option<String>)>]) -> Result<String, FsError> {
    if rows.is_empty() {
        return Ok(String::new());
    }
    let mut wtr = csv::Writer::from_writer(vec![]);
    // Header from first row
    let headers: Vec<&str> = rows[0].iter().map(|(col, _)| col.as_str()).collect();
    wtr.write_record(&headers).map_err(|e| FsError::SerializationError(e.to_string()))?;
    for row_data in rows {
        let values: Vec<String> = row_data.iter().map(|(_, val)| {
            match val {
                Some(v) => v.clone(),
                None => "NULL".to_string(),
            }
        }).collect();
        wtr.write_record(&values).map_err(|e| FsError::SerializationError(e.to_string()))?;
    }
    wtr.flush().map_err(|e| FsError::SerializationError(e.to_string()))?;
    String::from_utf8(wtr.into_inner().map_err(|e| FsError::SerializationError(e.to_string()))?)
        .map_err(|e| FsError::SerializationError(e.to_string()))
}
