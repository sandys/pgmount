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
    pub display_name: String,  // For directory name: "pk_value" or "pk1=v1,pk2=v2"
}
