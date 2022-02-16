ALTER TABLE bandwidth_usage
ADD COLUMN ingest_bytes_since_prev INTEGER NOT NULL DEFAULT 0;
