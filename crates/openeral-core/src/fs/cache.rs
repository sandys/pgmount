use crate::db::types::*;
use dashmap::DashMap;
use std::time::{Duration, Instant};

struct CacheEntry<T> {
    data: T,
    inserted_at: Instant,
}

impl<T: Clone> CacheEntry<T> {
    fn is_valid(&self, ttl: Duration) -> bool {
        self.inserted_at.elapsed() < ttl
    }
}

pub struct MetadataCache {
    ttl: Duration,
    schemas: DashMap<String, CacheEntry<Vec<SchemaInfo>>>, // key: "" (global)
    tables: DashMap<String, CacheEntry<Vec<TableInfo>>>,   // key: schema
    columns: DashMap<String, CacheEntry<Vec<ColumnInfo>>>, // key: "schema.table"
    primary_keys: DashMap<String, CacheEntry<PrimaryKeyInfo>>, // key: "schema.table"
    row_counts: DashMap<String, CacheEntry<i64>>,          // key: "schema.table"
}

impl MetadataCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            schemas: DashMap::new(),
            tables: DashMap::new(),
            columns: DashMap::new(),
            primary_keys: DashMap::new(),
            row_counts: DashMap::new(),
        }
    }

    pub fn get_schemas(&self) -> Option<Vec<SchemaInfo>> {
        self.schemas.get("").and_then(|e| {
            if e.is_valid(self.ttl) {
                Some(e.data.clone())
            } else {
                None
            }
        })
    }

    pub fn set_schemas(&self, schemas: Vec<SchemaInfo>) {
        self.schemas.insert(
            "".to_string(),
            CacheEntry {
                data: schemas,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn get_tables(&self, schema: &str) -> Option<Vec<TableInfo>> {
        self.tables.get(schema).and_then(|e| {
            if e.is_valid(self.ttl) {
                Some(e.data.clone())
            } else {
                None
            }
        })
    }

    pub fn set_tables(&self, schema: &str, tables: Vec<TableInfo>) {
        self.tables.insert(
            schema.to_string(),
            CacheEntry {
                data: tables,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn get_columns(&self, schema: &str, table: &str) -> Option<Vec<ColumnInfo>> {
        let key = format!("{}.{}", schema, table);
        self.columns.get(&key).and_then(|e| {
            if e.is_valid(self.ttl) {
                Some(e.data.clone())
            } else {
                None
            }
        })
    }

    pub fn set_columns(&self, schema: &str, table: &str, columns: Vec<ColumnInfo>) {
        let key = format!("{}.{}", schema, table);
        self.columns.insert(
            key,
            CacheEntry {
                data: columns,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn get_primary_key(&self, schema: &str, table: &str) -> Option<PrimaryKeyInfo> {
        let key = format!("{}.{}", schema, table);
        self.primary_keys.get(&key).and_then(|e| {
            if e.is_valid(self.ttl) {
                Some(e.data.clone())
            } else {
                None
            }
        })
    }

    pub fn set_primary_key(&self, schema: &str, table: &str, pk: PrimaryKeyInfo) {
        let key = format!("{}.{}", schema, table);
        self.primary_keys.insert(
            key,
            CacheEntry {
                data: pk,
                inserted_at: Instant::now(),
            },
        );
    }

    pub fn get_row_count(&self, schema: &str, table: &str) -> Option<i64> {
        let key = format!("{}.{}", schema, table);
        self.row_counts.get(&key).and_then(|e| {
            if e.is_valid(self.ttl) {
                Some(e.data)
            } else {
                None
            }
        })
    }

    pub fn set_row_count(&self, schema: &str, table: &str, count: i64) {
        let key = format!("{}.{}", schema, table);
        self.row_counts.insert(
            key,
            CacheEntry {
                data: count,
                inserted_at: Instant::now(),
            },
        );
    }

    /// Invalidate all caches
    pub fn invalidate_all(&self) {
        self.schemas.clear();
        self.tables.clear();
        self.columns.clear();
        self.primary_keys.clear();
        self.row_counts.clear();
    }
}
