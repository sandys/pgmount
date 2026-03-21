CREATE SCHEMA IF NOT EXISTS _openeral;

CREATE TABLE IF NOT EXISTS _openeral.schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TIMESTAMPTZ DEFAULT NOW()
);
