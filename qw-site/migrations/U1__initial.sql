CREATE TABLE account (
       id SERIAL PRIMARY KEY,

       name VARCHAR(32) NOT NULL UNIQUE,
       password_hash VARCHAR(128) NOT NULL,

       email VARCHAR(64) NOT NULL UNIQUE,
       stream_key VARCHAR(32) NOT NULL
);

CREATE TABLE account_permission (
       account_id INTEGER PRIMARY KEY,
       permissions INTEGER NOT NULL,

       FOREIGN KEY(account_id) REFERENCES account(id)
);

CREATE TABLE account_approval (
       email VARCHAR(64) NOT NULL UNIQUE,
       time_sent BIGINT NOT NULL,
       secret VARCHAR(32) NOT NULL UNIQUE
);

CREATE TABLE account_session (
       id VARCHAR(64) PRIMARY KEY,
       account_id INTEGER NOT NULL,
       user_agent TEXT,

       FOREIGN KEY(account_id) REFERENCES account(id)
);

CREATE TABLE stream_session (
       id SERIAL PRIMARY KEY,
       account_id INTEGER NOT NULL,
       viewer_count INTEGER NOT NULL DEFAULT 0,
       start_time BIGINT NOT NULL,
       stop_time BIGINT,

       FOREIGN KEY(account_id) REFERENCES account(id)
);

CREATE TABLE bandwidth_usage (
       id SERIAL PRIMARY KEY,
       stream_session_id INTEGER NOT NULL,
       time BIGINT NOT NULL,
       bytes_since_prev INTEGER NOT NULL,

       FOREIGN KEY(stream_session_id) REFERENCES stream_session(id)
);

CREATE INDEX time_idx ON bandwidth_usage (time);
