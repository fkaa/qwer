CREATE TABLE stream_metadata (
    stream_session_id INTEGER NOT NULL UNIQUE,
    encoder TEXT,
    video_bitrate_kbps INTEGER,
    parameter_sets BYTEA,

    FOREIGN KEY(stream_session_id) REFERENCES stream_session(id)
);
