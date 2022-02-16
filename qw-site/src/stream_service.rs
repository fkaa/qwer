use std::sync::Arc;
use std::{collections::HashMap, time::Duration};

use crate::{PostgresConnection, PostgresPool};

use bytesize::ByteSize;
use qw_proto::stream_info::{stream_reply::StreamType, StreamMetadata};
use tokio::sync::RwLock;
use tokio::task;
use tokio::{sync::mpsc, time::sleep};
use tracing::*;

pub struct StreamSession {
    pub id: i32,
    pub account_id: i32,
    pub start: time::OffsetDateTime,
    pub stop: Option<time::OffsetDateTime>,
}

async fn insert_bitrates(
    conn: &PostgresConnection<'_>,
    bandwidths: &[(i32, AggregatedStats)],
) -> anyhow::Result<()> {
    let stmt = conn
        .prepare(
            "
INSERT INTO bandwidth_usage (time, ingest_bytes_since_prev, bytes_since_prev, stream_session_id)
VALUES($1, $2, $3, $4)
        ",
        )
        .await?;

    let mut total_ingest_bytes = 0;
    let mut total_other_bytes = 0;

    for (stream_id, stats) in bandwidths {
        conn.execute(
            &stmt,
            &[
                &stats.time.unix_timestamp(),
                &stats.ingest_bytes,
                &stats.other_bytes,
                &stream_id,
            ],
        )
        .await?;

        total_ingest_bytes += stats.ingest_bytes as u64;
        total_other_bytes += stats.other_bytes as u64;
    }

    debug!(
        "Streaming stats: streams={}, ingest={}, other={}",
        bandwidths.len(),
        ByteSize(total_ingest_bytes),
        ByteSize(total_other_bytes)
    );

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
            &[
                &stream_session_id,
                &meta.video_encoder,
                &meta.video_bitrate_kbps.map(|b| b as i32),
                &meta.parameter_sets,
            ],
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

#[derive(Clone)]
struct AggregatedStats {
    time: time::OffsetDateTime,
    ingest_bytes: i32,
    other_bytes: i32,
}

impl AggregatedStats {
    fn new(time: time::OffsetDateTime) -> Self {
        AggregatedStats {
            time,
            ingest_bytes: 0,
            other_bytes: 0,
        }
    }
}

async fn insert_agg_bitrates(
    pool: Arc<PostgresPool>,
    stats: Vec<(i32, AggregatedStats)>,
) -> anyhow::Result<()> {
    let conn = pool.get().await?;
    insert_bitrates(&conn, &stats).await?;

    Ok(())
}

pub struct StreamSessionService {
    pub recv: mpsc::Receiver<StreamType>,
    pub pool: Arc<PostgresPool>,
    pub streams: HashMap<i32, StreamInfo>,
    aggregated_stats: RwLock<HashMap<i32, AggregatedStats>>,
}

impl StreamSessionService {
    pub fn new(recv: mpsc::Receiver<StreamType>, pool: Arc<PostgresPool>) -> Self {
        StreamSessionService {
            recv,
            pool,
            streams: HashMap::new(),
            aggregated_stats: RwLock::new(HashMap::new()),
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
        tokio::select! {
            res = async {
                loop {
                    if let Some(msg) = self.recv.recv().await {
                        handle_msg(&self.pool, &mut self.streams, &self.aggregated_stats, msg).await?;
                    }
                }
            } => res,
            res = async {
                loop {
                    sleep(Duration::from_secs(60)).await;

                    let stats = {
                        let mut stats = self.aggregated_stats.write().await;
                        stats.drain().collect::<Vec<_>>()
                    };

                    if !stats.is_empty() {
                        let pool = self.pool.clone();
                        task::spawn(async {
                            if let Err(e) = insert_agg_bitrates(pool, stats).await {
                                error!("Failed to insert aggregated stream stats: {e}");
                            }
                        });
                    }
                }
            } => res
        }
    }
}

async fn handle_msg(
    pool: &Arc<PostgresPool>,
    streams: &mut HashMap<i32, StreamInfo>,
    aggregated_stats: &RwLock<HashMap<i32, AggregatedStats>>,
    msg: StreamType,
) -> anyhow::Result<()> {
    match msg {
        StreamType::StreamExisting(stream) => {
            streams.insert(
                stream.stream_session_id,
                StreamInfo {
                    viewers: stream.viewers,
                },
            );

            if let Some(meta) = stream.meta {
                let conn = pool.get().await?;
                attach_stream_metadata(&conn, stream.stream_session_id, meta).await?;
            }
        }
        StreamType::StreamStarted(stream) => {
            streams.insert(stream.stream_session_id, StreamInfo { viewers: 0 });

            if let Some(meta) = stream.meta {
                let conn = pool.get().await?;
                attach_stream_metadata(&conn, stream.stream_session_id, meta).await?;
            }
        }
        StreamType::ViewerJoin(msg) => {
            if let Some(mut stream) = streams.get_mut(&msg.stream_session_id) {
                stream.viewers += 1;
            }
        }
        StreamType::ViewerLeave(msg) => {
            if let Some(mut stream) = streams.get_mut(&msg.stream_session_id) {
                stream.viewers -= 1;
            }
        }
        StreamType::StreamStopped(stream) => {
            let conn = pool.get().await?;
            let end = time::OffsetDateTime::now_utc();

            stop_stream_session(&conn, stream.stream_session_id, end).await?;
            streams.remove(&stream.stream_session_id);
        }
        StreamType::StreamStats(stat) => {
            let mut stats = aggregated_stats.write().await;
            let entry = stats
                .entry(stat.stream_session_id)
                .or_insert(AggregatedStats::new(time::OffsetDateTime::now_utc()));

            if stat.is_ingest {
                entry.ingest_bytes += stat.bytes_since_last_stats as i32;
            } else {
                entry.other_bytes += stat.bytes_since_last_stats as i32;
            }
        }
    }

    Ok(())
}
