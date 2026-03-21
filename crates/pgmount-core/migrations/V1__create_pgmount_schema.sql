CREATE SCHEMA IF NOT EXISTS _pgmount;

CREATE TABLE IF NOT EXISTS _pgmount.schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TIMESTAMPTZ DEFAULT NOW()
);
