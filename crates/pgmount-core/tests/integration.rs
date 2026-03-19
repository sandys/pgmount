use pgmount_core::config::types::MountConfig;
use pgmount_core::db::pool::create_pool;
use pgmount_core::db::queries::{introspection, rows, stats};
use pgmount_core::fs::cache::MetadataCache;
use pgmount_core::fs::inode::{InodeTable, NodeIdentity};
use std::time::Duration;

fn connection_string() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "host=postgres user=pgmount password=pgmount dbname=testdb".to_string())
}

#[tokio::test]
async fn test_list_schemas() {
    let pool = create_pool(&connection_string()).unwrap();
    let schemas = introspection::list_schemas(&pool).await.unwrap();
    assert!(schemas.iter().any(|s| s.name == "public"));
    // Shouldn't include system schemas
    assert!(!schemas.iter().any(|s| s.name == "pg_catalog"));
    assert!(!schemas.iter().any(|s| s.name == "information_schema"));
}

#[tokio::test]
async fn test_list_tables() {
    let pool = create_pool(&connection_string()).unwrap();
    let tables = introspection::list_tables(&pool, "public").await.unwrap();
    let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert!(table_names.contains(&"users"));
    assert!(table_names.contains(&"posts"));
}

#[tokio::test]
async fn test_list_columns() {
    let pool = create_pool(&connection_string()).unwrap();
    let columns = introspection::list_columns(&pool, "public", "users").await.unwrap();
    let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert!(col_names.contains(&"id"));
    assert!(col_names.contains(&"name"));
    assert!(col_names.contains(&"email"));
    assert!(col_names.contains(&"age"));
    assert!(col_names.contains(&"active"));
}

#[tokio::test]
async fn test_primary_key() {
    let pool = create_pool(&connection_string()).unwrap();
    let pk = introspection::get_primary_key(&pool, "public", "users").await.unwrap();
    assert_eq!(pk.column_names, vec!["id"]);
}

#[tokio::test]
async fn test_list_rows() {
    let pool = create_pool(&connection_string()).unwrap();
    let pk_columns = vec!["id".to_string()];
    let row_ids = rows::list_rows(&pool, "public", "users", &pk_columns, 100, 0).await.unwrap();
    assert_eq!(row_ids.len(), 3);
    assert_eq!(row_ids[0].display_name, "1");
    assert_eq!(row_ids[1].display_name, "2");
    assert_eq!(row_ids[2].display_name, "3");
}

#[tokio::test]
async fn test_get_row_data() {
    let pool = create_pool(&connection_string()).unwrap();
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["1".to_string()];
    let row_data = rows::get_row_data(&pool, "public", "users", &pk_columns, &pk_values).await.unwrap();
    let data_map: std::collections::HashMap<&str, Option<&str>> = row_data
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_deref()))
        .collect();
    assert_eq!(data_map["name"], Some("Alice"));
    assert_eq!(data_map["email"], Some("alice@example.com"));
    assert_eq!(data_map["age"], Some("30"));
}

#[tokio::test]
async fn test_get_column_value() {
    let pool = create_pool(&connection_string()).unwrap();
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["2".to_string()];
    let value = rows::get_column_value(&pool, "public", "users", "name", &pk_columns, &pk_values).await.unwrap();
    assert_eq!(value, Some("Bob".to_string()));
}

#[tokio::test]
async fn test_row_count_estimate() {
    let pool = create_pool(&connection_string()).unwrap();
    // Run ANALYZE first to ensure pg_class.reltuples is populated
    let client = pool.get().await.unwrap();
    client.execute("ANALYZE users", &[]).await.unwrap();
    drop(client);

    let count = stats::get_row_count_estimate(&pool, "public", "users").await.unwrap();
    assert!(count >= 0); // Estimate may not be exact
}

#[tokio::test]
async fn test_exact_row_count() {
    let pool = create_pool(&connection_string()).unwrap();
    let count = stats::get_exact_row_count(&pool, "public", "users").await.unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_inode_table() {
    let table = InodeTable::new();

    // Root is pre-registered at inode 1
    assert_eq!(table.get_ino(&NodeIdentity::Root), Some(1));
    assert!(matches!(table.get_identity(1), Some(NodeIdentity::Root)));

    // New identity gets a new inode
    let schema_id = NodeIdentity::Schema { name: "public".to_string() };
    let ino = table.get_or_insert(schema_id.clone());
    assert!(ino >= 2);

    // Same identity returns the same inode
    let ino2 = table.get_or_insert(schema_id.clone());
    assert_eq!(ino, ino2);

    // Different identity gets different inode
    let other = NodeIdentity::Schema { name: "other".to_string() };
    let ino3 = table.get_or_insert(other);
    assert_ne!(ino, ino3);
}

#[tokio::test]
async fn test_metadata_cache() {
    let cache = MetadataCache::new(Duration::from_secs(60));

    // Empty cache returns None
    assert!(cache.get_schemas().is_none());
    assert!(cache.get_tables("public").is_none());

    // Set and get schemas
    let schemas = vec![pgmount_core::db::types::SchemaInfo { name: "public".to_string() }];
    cache.set_schemas(schemas.clone());
    let cached = cache.get_schemas().unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].name, "public");

    // Invalidate
    cache.invalidate_all();
    assert!(cache.get_schemas().is_none());
}

#[tokio::test]
async fn test_format_json() {
    let row = vec![
        ("name".to_string(), Some("Alice".to_string())),
        ("age".to_string(), Some("30".to_string())),
        ("email".to_string(), None),
    ];
    let json = pgmount_core::format::json::format_row(&row).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["name"], "Alice");
    assert_eq!(parsed["age"], 30); // Should parse as number
    assert!(parsed["email"].is_null());
}

#[tokio::test]
async fn test_format_csv() {
    let row = vec![
        ("name".to_string(), Some("Alice".to_string())),
        ("age".to_string(), Some("30".to_string())),
    ];
    let csv = pgmount_core::format::csv::format_row(&row).unwrap();
    assert!(csv.contains("name,age"));
    assert!(csv.contains("Alice,30"));
}
