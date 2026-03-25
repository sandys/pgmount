#[derive(Debug, Clone)]
pub struct SchemaInfo {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub table_type: String, // BASE TABLE or VIEW
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub is_nullable: bool,
    pub column_default: Option<String>,
    pub ordinal_position: i32,
}

#[derive(Debug, Clone)]
pub struct PrimaryKeyInfo {
    pub column_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct IndexInfo {
    pub name: String,
    pub is_unique: bool,
    pub is_primary: bool,
    pub definition: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RowIdentifier {
    pub pk_values: Vec<(String, String)>, // (column_name, value_as_string)
    pub display_name: String,             // For directory name: "pk_value" or "pk1=v1,pk2=v2"
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceConfig {
    pub id: String,
    pub display_name: Option<String>,
    pub config: WorkspaceLayout,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceLayout {
    #[serde(default)]
    pub auto_dirs: Vec<String>,
    #[serde(default)]
    pub seed_files: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceFile {
    pub workspace_id: String,
    pub path: String,
    pub parent_path: String,
    pub name: String,
    pub is_dir: bool,
    pub content: Option<Vec<u8>>,
    pub mode: i32,
    pub size: i64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub atime_ns: i64,
    pub nlink: i32,
    pub uid: i32,
    pub gid: i32,
}
