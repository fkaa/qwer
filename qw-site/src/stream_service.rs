use std::collections::HashMap;
use std::sync::Arc;

use crate::{PostgresConnection, PostgresPool};

use qw_proto::stream_info::{stream_reply::StreamType, StreamMetadata};
use tokio::sync::mpsc;
use tracing::*;

pub struct StreamSession {
    pub id: i32,
    pub account_id: i32,
    pub start: time::OffsetDateTime,
    pub stop: Option<time::OffsetDateTime>,
}

struct BandwidthUsageSample {
    time: time::OffsetDateTime,
    bytes_since_prev: i32,
    stream_session_id: i32,
}

async fn insert_bitrates(
    conn: &PostgresConnection<'_>,
    bandwidths: &[BandwidthUsageSample],
) -> anyhow::Result<()> {
    debug!("Inserting bandwidth usage");

    let stmt = conn
        .prepare(
            "
INSERT INTO bandwidth_usage (time, bytes_since_prev, stream_session_id)
VALUES($1, $2, $3)
        ",
        )
        .await?;

    for sample in bandwidths {
        conn.execute(
            &stmt,
            &[
                &sample.time.unix_timestamp(),
                &sample.bytes_since_prev,
                &sample.stream_session_id,
            ],
        )
        .await?;
    }

    Ok(())
}

pub async fn start_stream_session(
    conn: &PostgresConnection<'_>,
    account: i32,
    is_unlisted: bool,
    start: time::OffsetDateTime,
) -> anyhow::Result<i32> {
    let start = start.unix_timestamp();

    let row = conn
        .query_one(
            "
INSERT INTO stream_session (account_id, start_time, unlisted)
VALUES ($1, $2, $3)
RETURNING id
            ",
            &[&account, &start, &is_unlisted],
        )
        .await?;

    Ok(row.get::<_, i32>(0))
}

pub async fn stop_stream_session(
    conn: &PostgresConnection<'_>,
    stream_session_id: i32,
    end: time::OffsetDateTime,
) -> anyhow::Result<()> {
    let end = end.unix_timestamp();

    let _row = conn
        .execute(
            "
UPDATE stream_session
SET stop_time = $2
WHERE id = $1
            ",
            &[&stream_session_id, &end],
        )
        .await?;

    Ok(())
}

pub async fn attach_stream_metadata(
    conn: &PostgresConnection<'_>,
    stream_session_id: i32,
    meta: StreamMetadata,
) -> anyhow::Result<()> {
    let _ = conn
        .execute(
            "
INSERT INTO stream_metadata (stream_session_id, encoder, video_bitrate_kbps, parameter_sets)
VALUES ($1, $2, $3, $4)
ON CONFLICT DO NOTHING
            ",
            &[&stream_session_id, &meta.video_encoder, &meta.video_bitrate_kbps.map(|b| b as i32), &meta.parameter_sets],
        )
        .await?;

    Ok(())
}

pub async fn get_stream_sessions(
    conn: &PostgresConnection<'_>,
    account: i32,
    start: time::OffsetDateTime,
    end: time::OffsetDateTime,
) -> anyhow::Result<Vec<StreamSession>> {
    let start = start.unix_timestamp();
    let end = end.unix_timestamp();

    let sessions = conn
        .query(
            "
SELECT id, start_time, stop_time FROM stream_session
WHERE
account_id = $1 AND
start_time >= $2 AND
(stop_time <= $3 OR stop_time IS NULL)
        ",
            &[&account, &start, &end],
        )
        .await?;

    let sessions = sessions
        .iter()
        .map(|r| StreamSession {
            id: r.get::<_, i32>(0),
            account_id: account,
            start: time::OffsetDateTime::from_unix_timestamp(r.get::<_, i64>(1)).unwrap(),
            stop: r
                .get::<_, Option<i64>>(2)
                .map(|t| time::OffsetDateTime::from_unix_timestamp(t).unwrap()),
        })
        .collect::<Vec<_>>();

    Ok(sessions)
}

pub struct ActiveStreamSession {
    pub viewer_count: i32,
    pub started: time::OffsetDateTime,
    pub account_name: String,
}

pub async fn get_active_public_stream_sessions(
    conn: &PostgresConnection<'_>,
) -> anyhow::Result<Vec<ActiveStreamSession>> {
    let sessions = conn
        .query(
            "
SELECT
    stream_session.viewer_count,
    stream_session.start_time,
    account.name
FROM stream_session
INNER JOIN account ON
    stream_session.account_id = account.id
WHERE
    stream_session.stop_time IS NULL AND
    NOT stream_session.unlisted
        ",
            &[],
        )
        .await?;

    let sessions = sessions
        .iter()
        .map(|r| ActiveStreamSession {
            viewer_count: r.get::<_, i32>(0),
            started: time::OffsetDateTime::from_unix_timestamp(r.get::<_, i64>(1)).unwrap(),
            account_name: r.get::<_, String>(2),
        })
        .collect::<Vec<_>>();

    Ok(sessions)
}

#[derive(Default)]
pub struct StreamInfo {
    viewers: u32,
}

pub struct StreamSessionService {
    pub recv: mpsc::Receiver<StreamType>,
    pub pool: Arc<PostgresPool>,
    pub streams: HashMap<i32, StreamInfo>,
}

impl StreamSessionService {
    pub fn new(recv: mpsc::Receiver<StreamType>, pool: Arc<PostgresPool>) -> Self {
        StreamSessionService {
            recv,
            pool,
            streams: HashMap::new(),
        }
    }

    pub async fn start(&mut self) -> anyhow::Result<()> {
        let conn = self.pool.get().await?;

        let stop_time = time::OffsetDateTime::now_utc();

        let rows_updated = conn
            .execute(
                "
UPDATE stream_session
SET stop_time = $1
WHERE stop_time IS NULL
            ",
                &[&stop_time.unix_timestamp()],
            )
            .await?;

        debug!("Corrected {} inconsistent stream sessions", rows_updated);

        Ok(())
    }

    pub async fn poll(&mut self) -> anyhow::Result<()> {
        while let Some(msg) = self.recv.recv().await {
            match msg {
                StreamType::StreamExisting(stream) => {
                    self.streams.insert(
                        stream.stream_session_id,
                        StreamInfo {
                            viewers: stream.viewers,
                        },
                    );

                    if let Some(meta) = stream.meta {
                        let conn = self.pool.get().await?;
                        attach_stream_metadata(&conn, stream.stream_session_id, meta).await?;
                    }
                }
                StreamType::StreamStarted(stream) => {
                    self.streams
                        .insert(stream.stream_session_id, StreamInfo { viewers: 0 });

                    if let Some(meta) = stream.meta {
                        let conn = self.pool.get().await?;
                        attach_stream_metadata(&conn, stream.stream_session_id, meta).await?;
                    }
                }
                StreamType::ViewerJoin(msg) => {
                    if let Some(mut stream) = self.streams.get_mut(&msg.stream_session_id) {
                        stream.viewers += 1;
                    }
                }
                StreamType::ViewerLeave(msg) => {
                    if let Some(mut stream) = self.streams.get_mut(&msg.stream_session_id) {
                        stream.viewers -= 1;
                    }
                }
                StreamType::StreamStopped(stream) => {
                    let conn = self.pool.get().await?;
                    let end = time::OffsetDateTime::now_utc();

                    stop_stream_session(&conn, stream.stream_session_id, end).await?;
                    self.streams.remove(&stream.stream_session_id);
                }
                StreamType::StreamStats(stat) => {
                    let conn = self.pool.get().await?;

                    let bw_usage = BandwidthUsageSample {
                        stream_session_id: stat.stream_session_id,
                        time: time::OffsetDateTime::now_utc(),
                        bytes_since_prev: stat.bytes_since_last_stats as i32,
                    };

                    insert_bitrates(&conn, &[bw_usage]).await.unwrap();
                }
            }
        }

        Ok(())
    }
}
