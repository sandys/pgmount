use pgmount_core::db::pool::create_pool;
use pgmount_core::db::queries::{introspection, rows, stats, indexes};
use pgmount_core::fs::cache::MetadataCache;
use pgmount_core::fs::inode::{InodeTable, NodeIdentity};
use std::time::Duration;
use tokio::sync::OnceCell;

static SETUP_CELL: OnceCell<()> = OnceCell::const_new();

fn connection_string() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "host=postgres user=pgmount password=pgmount dbname=testdb".to_string())
}

/// Ensure test schema exists. Uses tokio::sync::OnceCell for async-safe one-time init.
async fn setup_db(pool: &deadpool_postgres::Pool) {
    SETUP_CELL.get_or_init(|| async {
        let client = pool.get().await.unwrap();
        client.batch_execute("
            DROP SCHEMA IF EXISTS rust_test CASCADE;
            CREATE SCHEMA rust_test;
            CREATE TABLE rust_test.users (
                id SERIAL PRIMARY KEY,
                name TEXT NOT NULL,
                email TEXT,
                age INTEGER,
                active BOOLEAN DEFAULT true
            );
            INSERT INTO rust_test.users (id, name, email, age, active)
            OVERRIDING SYSTEM VALUE
            VALUES
                (1, 'Alice', 'alice@example.com', 30, true),
                (2, 'Bob', 'bob@example.com', 25, false),
                (3, 'Charlie', 'charlie@example.com', 35, true);
            ANALYZE rust_test.users;
        ").await.unwrap();
    }).await;
}

async fn get_pool() -> deadpool_postgres::Pool {
    let pool = create_pool(&connection_string(), 30).unwrap();
    setup_db(&pool).await;
    pool
}

const S: &str = "rust_test";
const T: &str = "users";

#[tokio::test]
async fn test_list_schemas() {
    let pool = get_pool().await;
    let schemas = introspection::list_schemas(&pool).await.unwrap();
    assert!(schemas.iter().any(|s| s.name == S));
    assert!(!schemas.iter().any(|s| s.name == "pg_catalog"));
    assert!(!schemas.iter().any(|s| s.name == "information_schema"));
}

#[tokio::test]
async fn test_list_tables() {
    let pool = get_pool().await;
    let tables = introspection::list_tables(&pool, S).await.unwrap();
    let table_names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert!(table_names.contains(&T));
}

#[tokio::test]
async fn test_list_columns() {
    let pool = get_pool().await;
    let columns = introspection::list_columns(&pool, S, T).await.unwrap();
    let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert!(col_names.contains(&"id"));
    assert!(col_names.contains(&"name"));
    assert!(col_names.contains(&"email"));
    assert!(col_names.contains(&"age"));
    assert!(col_names.contains(&"active"));
    let name_col = columns.iter().find(|c| c.name == "name").unwrap();
    assert_eq!(name_col.data_type, "text");
    assert!(!name_col.is_nullable);
}

#[tokio::test]
async fn test_primary_key() {
    let pool = get_pool().await;
    let pk = introspection::get_primary_key(&pool, S, T).await.unwrap();
    assert_eq!(pk.column_names, vec!["id"]);
}

#[tokio::test]
async fn test_list_rows() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let row_ids = rows::list_rows(&pool, S, T, &pk_columns, 100, 0).await.unwrap();
    assert_eq!(row_ids.len(), 3);
    assert_eq!(row_ids[0].display_name, "1");
    assert_eq!(row_ids[1].display_name, "2");
    assert_eq!(row_ids[2].display_name, "3");
}

#[tokio::test]
async fn test_list_rows_pagination() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let page1 = rows::list_rows(&pool, S, T, &pk_columns, 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].display_name, "1");
    assert_eq!(page1[1].display_name, "2");
    let page2 = rows::list_rows(&pool, S, T, &pk_columns, 2, 2).await.unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].display_name, "3");
}

#[tokio::test]
async fn test_get_row_data() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["1".to_string()];
    let row_data = rows::get_row_data(&pool, S, T, &pk_columns, &pk_values).await.unwrap();
    let data_map: std::collections::HashMap<&str, Option<&str>> = row_data
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_deref()))
        .collect();
    assert_eq!(data_map["name"], Some("Alice"));
    assert_eq!(data_map["email"], Some("alice@example.com"));
    assert_eq!(data_map["age"], Some("30"));
    assert_eq!(data_map["active"], Some("true"));
}

#[tokio::test]
async fn test_get_row_data_not_found() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["99999".to_string()];
    let result = rows::get_row_data(&pool, S, T, &pk_columns, &pk_values).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_column_value() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["2".to_string()];
    let value = rows::get_column_value(&pool, S, T, "name", &pk_columns, &pk_values).await.unwrap();
    assert_eq!(value, Some("Bob".to_string()));
}

#[tokio::test]
async fn test_get_column_value_null() {
    let pool = get_pool().await;
    let client = pool.get().await.unwrap();
    client.batch_execute("
        CREATE TABLE IF NOT EXISTS rust_test.null_test (id INTEGER PRIMARY KEY, val TEXT);
        TRUNCATE rust_test.null_test;
        INSERT INTO rust_test.null_test (id, val) VALUES (1, NULL);
    ").await.unwrap();

    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["1".to_string()];
    let value = rows::get_column_value(&pool, S, "null_test", "val", &pk_columns, &pk_values).await.unwrap();
    assert_eq!(value, None);
}

#[tokio::test]
async fn test_row_count_estimate() {
    let pool = get_pool().await;
    let count = stats::get_row_count_estimate(&pool, S, T).await.unwrap();
    assert!(count >= 0);
}

#[tokio::test]
async fn test_exact_row_count() {
    let pool = get_pool().await;
    let count = stats::get_exact_row_count(&pool, S, T).await.unwrap();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_list_indexes() {
    let pool = get_pool().await;
    let idxs = indexes::list_indexes(&pool, S, T).await.unwrap();
    assert!(!idxs.is_empty());
    let pk_idx = idxs.iter().find(|i| i.is_primary);
    assert!(pk_idx.is_some(), "Should have a primary key index");
    assert!(pk_idx.unwrap().columns.contains(&"id".to_string()));
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

    // Same identity returns the same inode (idempotent)
    let ino2 = table.get_or_insert(schema_id);
    assert_eq!(ino, ino2);

    // Different identity gets different inode
    let other = NodeIdentity::Schema { name: "other".to_string() };
    let ino3 = table.get_or_insert(other);
    assert_ne!(ino, ino3);

    // Reverse lookup works
    let identity = table.get_identity(ino).unwrap();
    assert!(matches!(identity, NodeIdentity::Schema { name } if name == "public"));
}

#[tokio::test]
async fn test_metadata_cache() {
    let cache = MetadataCache::new(Duration::from_secs(60));

    assert!(cache.get_schemas().is_none());
    assert!(cache.get_tables("public").is_none());

    let schemas = vec![pgmount_core::db::types::SchemaInfo { name: "public".to_string() }];
    cache.set_schemas(schemas);
    let cached = cache.get_schemas().unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].name, "public");

    cache.invalidate_all();
    assert!(cache.get_schemas().is_none());
}

#[tokio::test]
async fn test_metadata_cache_ttl_expiry() {
    let cache = MetadataCache::new(Duration::from_millis(50));
    let schemas = vec![pgmount_core::db::types::SchemaInfo { name: "x".to_string() }];
    cache.set_schemas(schemas);
    assert!(cache.get_schemas().is_some());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(cache.get_schemas().is_none(), "Cache should expire after TTL");
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
    assert_eq!(parsed["age"], 30);
    assert!(parsed["email"].is_null());
}

#[tokio::test]
async fn test_format_json_rows() {
    let rows = vec![
        vec![("a".to_string(), Some("1".to_string()))],
        vec![("a".to_string(), Some("2".to_string()))],
    ];
    let json = pgmount_core::format::json::format_rows(&rows).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_array());
    assert_eq!(parsed.as_array().unwrap().len(), 2);
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

#[tokio::test]
async fn test_format_yaml() {
    let row = vec![
        ("name".to_string(), Some("Alice".to_string())),
        ("age".to_string(), Some("30".to_string())),
    ];
    let yaml = pgmount_core::format::yaml::format_row(&row).unwrap();
    assert!(yaml.contains("name:"));
    assert!(yaml.contains("Alice"));
}

#[tokio::test]
async fn test_quote_ident() {
    assert_eq!(pgmount_core::db::queries::quote_ident("simple"), "\"simple\"");
    assert_eq!(pgmount_core::db::queries::quote_ident("has\"quote"), "\"has\"\"quote\"");
    assert_eq!(pgmount_core::db::queries::quote_ident("with space"), "\"with space\"");
}

#[tokio::test]
async fn test_pk_display_parsing() {
    use pgmount_core::fs::nodes::column::parse_pk_display;

    // Single PK
    let pk_cols = vec!["id".to_string()];
    let values = parse_pk_display("42", &pk_cols);
    assert_eq!(values, vec!["42"]);

    // Composite PK
    let pk_cols = vec!["order_id".to_string(), "item_id".to_string()];
    let values = parse_pk_display("order_id=1,item_id=2", &pk_cols);
    assert_eq!(values, vec!["1", "2"]);
}

#[tokio::test]
async fn test_pk_display_percent_encoding_roundtrip() {
    use pgmount_core::db::queries::rows::encode_pk_value;
    use pgmount_core::fs::nodes::column::parse_pk_display;

    // Value with special chars
    let encoded = encode_pk_value("hello/world");
    assert!(!encoded.contains('/'));
    let pk_cols = vec!["id".to_string()];
    let decoded = parse_pk_display(&encoded, &pk_cols);
    assert_eq!(decoded, vec!["hello/world"]);

    // Value with comma and equals
    let encoded = encode_pk_value("a=b,c");
    assert!(!encoded.contains(','));
    assert!(!encoded.contains('='));
    let decoded = parse_pk_display(&encoded, &pk_cols);
    assert_eq!(decoded, vec!["a=b,c"]);
}

// --- Issue 5: query_rows shared function ---

#[tokio::test]
async fn test_query_rows_with_filter() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let where_clause = "\"active\"::text = $1";
    let filter_val = "true".to_string();
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&filter_val];
    let result = rows::query_rows(&pool, S, T, &pk_columns, 100, 0, Some(where_clause), None, &params).await.unwrap();
    assert_eq!(result.len(), 2); // Alice and Charlie are active
}

#[tokio::test]
async fn test_query_rows_with_order() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let result = rows::query_rows(&pool, S, T, &pk_columns, 100, 0, None, Some("\"name\" DESC"), &[]).await.unwrap();
    assert_eq!(result.len(), 3);
    // Charlie (id=3) > Bob (id=2) > Alice (id=1) alphabetically desc
    assert_eq!(result[0].display_name, "3");
    assert_eq!(result[1].display_name, "2");
    assert_eq!(result[2].display_name, "1");
}

#[tokio::test]
async fn test_query_rows_empty_result() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let where_clause = "\"name\"::text = $1";
    let filter_val = "nonexistent_person".to_string();
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = vec![&filter_val];
    let result = rows::query_rows(&pool, S, T, &pk_columns, 100, 0, Some(where_clause), None, &params).await.unwrap();
    assert!(result.is_empty());
}

// --- Issue 1: Bulk export query ---

#[tokio::test]
async fn test_get_all_rows_as_text() {
    let pool = get_pool().await;
    let (col_names, data) = rows::get_all_rows_as_text(&pool, S, T, 100, 0).await.unwrap();
    assert!(col_names.contains(&"id".to_string()));
    assert!(col_names.contains(&"name".to_string()));
    assert_eq!(data.len(), 3);
    // Verify first row has all columns as text
    let first_row: std::collections::HashMap<&str, Option<&str>> = data[0].iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
    assert!(first_row.contains_key("name"));
}

#[tokio::test]
async fn test_get_all_rows_as_text_pagination() {
    let pool = get_pool().await;
    let (_cols, page1) = rows::get_all_rows_as_text(&pool, S, T, 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2);
    let (_cols, page2) = rows::get_all_rows_as_text(&pool, S, T, 2, 2).await.unwrap();
    assert_eq!(page2.len(), 1);
}

// --- Issue 10: Error path tests ---

#[tokio::test]
async fn test_list_tables_nonexistent_schema() {
    let pool = get_pool().await;
    let tables = introspection::list_tables(&pool, "nonexistent_schema_xyz").await.unwrap();
    assert!(tables.is_empty());
}

#[tokio::test]
async fn test_list_columns_nonexistent_table() {
    let pool = get_pool().await;
    let columns = introspection::list_columns(&pool, S, "nonexistent_table_xyz").await.unwrap();
    assert!(columns.is_empty());
}

#[tokio::test]
async fn test_get_column_value_nonexistent_column() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["1".to_string()];
    let result = rows::get_column_value(&pool, S, T, "nonexistent_col", &pk_columns, &pk_values).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_query_rows_nonexistent_table() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let result = rows::query_rows(&pool, S, "nonexistent_xyz", &pk_columns, 100, 0, None, None, &[]).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_query_rows_empty_pk_columns() {
    let pool = get_pool().await;
    let pk_columns: Vec<String> = vec![];
    let result = rows::query_rows(&pool, S, T, &pk_columns, 100, 0, None, None, &[]).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_row_data_pk_mismatch() {
    let pool = get_pool().await;
    let pk_columns = vec!["id".to_string()];
    let pk_values = vec!["1".to_string(), "extra".to_string()]; // mismatch
    let result = rows::get_row_data(&pool, S, T, &pk_columns, &pk_values).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exact_row_count_nonexistent_table() {
    let pool = get_pool().await;
    let result = stats::get_exact_row_count(&pool, S, "nonexistent_xyz").await;
    assert!(result.is_err());
}
