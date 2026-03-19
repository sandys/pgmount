use crate::error::FsError;
use serde_json::{Map, Value};

/// Format a single row as a JSON object.
/// row_data is Vec<(column_name, Option<value_string>)>
pub fn format_row(row_data: &[(String, Option<String>)]) -> Result<String, FsError> {
    let mut map = Map::new();
    for (col, val) in row_data {
        let json_val = match val {
            Some(v) => {
                // Try to parse as number or boolean first
                if let Ok(n) = v.parse::<i64>() {
                    Value::Number(n.into())
                } else if let Ok(n) = v.parse::<f64>() {
                    Value::Number(serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into()))
                } else if v == "true" || v == "false" {
                    Value::Bool(v == "true")
                } else {
                    Value::String(v.clone())
                }
            }
            None => Value::Null,
        };
        map.insert(col.clone(), json_val);
    }
    serde_json::to_string_pretty(&Value::Object(map))
        .map(|s| s + "\n")
        .map_err(|e| FsError::SerializationError(e.to_string()))
}

/// Format multiple rows as a JSON array.
pub fn format_rows(rows: &[Vec<(String, Option<String>)>]) -> Result<String, FsError> {
    let mut arr = Vec::new();
    for row_data in rows {
        let mut map = Map::new();
        for (col, val) in row_data {
            let json_val = match val {
                Some(v) => Value::String(v.clone()),
                None => Value::Null,
            };
            map.insert(col.clone(), json_val);
        }
        arr.push(Value::Object(map));
    }
    serde_json::to_string_pretty(&Value::Array(arr))
        .map(|s| s + "\n")
        .map_err(|e| FsError::SerializationError(e.to_string()))
}
