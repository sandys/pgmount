CREATE TABLE IF NOT EXISTS _openeral.mount_log (
    id SERIAL PRIMARY KEY,
    mounted_at TIMESTAMPTZ DEFAULT NOW(),
    mount_point TEXT NOT NULL,
    schemas_filter TEXT[],
    page_size INTEGER,
    openeral_version TEXT
);
