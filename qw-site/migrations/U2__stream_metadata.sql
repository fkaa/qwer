create table stream_metadata (
    stream_session_id INTEGER NOT NULL UNIQUE,
    encoder text,
    video_bitrate_kbps integer,
    parameter_sets BYTEA,

    FOREIGN KEY(stream_session_id) REFERENCES stream_session(id)
);
