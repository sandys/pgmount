CREATE TABLE IF NOT EXISTS _openeral.cache_hints (
    id SERIAL PRIMARY KEY,
    schema_name TEXT NOT NULL,
    table_name TEXT NOT NULL,
    hint_type TEXT NOT NULL,
    hint_value TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (schema_name, table_name, hint_type)
);
