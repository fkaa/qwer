use std::sync::Arc;

use askama::Template;
use axum::{
    body::{boxed, BoxBody},
    extract::Extension,
    http::{Response, Uri},
    response::{IntoResponse, Redirect},
};
use time::{OffsetDateTime, Duration, Time};

use crate::{stream_service::get_stream_sessions, AppData, AskamaTemplate, PostgresConnection};

use super::session::Cookies;

struct DashboardBwSample {
    time: i64,
    ingest_bytes: u64,
    other_bytes: u64,
}

struct DashboardEntry {
    account_name: String,
    data: Vec<DashboardBwSample>,
}

async fn get_bitrates(
    conn: &PostgresConnection<'_>,
    stream_session_id: i32,
) -> anyhow::Result<Vec<(time::OffsetDateTime, u32, u32)>> {
    let samples = conn
        .query(
            "
SELECT time, bytes_since_prev, ingest_bytes_since_prev FROM bandwidth_usage
WHERE stream_session_id = $1
ORDER BY time
        ",
            &[&stream_session_id],
        )
        .await?;

    Ok(samples
        .iter()
        .map(|r| {
            (
                time::OffsetDateTime::from_unix_timestamp(r.get::<_, i64>(0)).unwrap(),
                r.get::<_, i32>(1) as u32,
                r.get::<_, i32>(2) as u32,
            )
        })
        .collect::<Vec<_>>())
}

async fn get_account_name(
    conn: &PostgresConnection<'_>,
    account_id: i32,
) -> anyhow::Result<String> {
    let row = conn
        .query_one(
            "
SELECT name FROM account
WHERE id = $1
            ",
            &[&account_id],
        )
        .await?;

    Ok(row.get::<_, String>(0))
}

async fn get_dashboard_entry(
    conn: &PostgresConnection<'_>,
    account_id: i32,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> anyhow::Result<DashboardEntry> {
    let account_name = get_account_name(conn, account_id).await?;
    let sessions = get_stream_sessions(conn, account_id, start, end).await?;

    let mut data = Vec::new();

    let mut total_ingest_bytes = 0;
    let mut total_other_bytes = 0;
    for session in sessions {
        let samples = get_bitrates(conn, session.id).await?;

        for (time, other_bytes, ingest_bytes) in samples {
            total_ingest_bytes += ingest_bytes as u64;
            total_other_bytes += other_bytes as u64;

            data.push(DashboardBwSample {
                time: time.unix_timestamp(),
                ingest_bytes: total_ingest_bytes,
                other_bytes: total_other_bytes,
            });
        }
    }

    Ok(DashboardEntry { account_name, data })
}

async fn get_all_dashboard_entries(
    conn: &PostgresConnection<'_>,
    start: OffsetDateTime,
    end: OffsetDateTime,
) -> anyhow::Result<Vec<DashboardEntry>> {
    let rows = conn
        .query(
            "
SELECT id FROM account
",
            &[],
        )
        .await?;

    let mut entries = Vec::new();
    for id in rows.iter().map(|r| r.get::<_, i32>(0)) {
        entries.push(get_dashboard_entry(conn, id, start, end).await?);
    }

    Ok(entries)
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    entries: Vec<DashboardEntry>,
    start: i64,
    end: i64,
}

pub(crate) async fn dashboard_page_get_handler(
    Extension(data): Extension<Arc<AppData>>,
    cookies: Cookies,
) -> crate::Result<Response<BoxBody>> {
    Ok(dashboard_page(&data, cookies).await?)
}

async fn dashboard_page(
    data: &Arc<AppData>,
    cookies: Cookies,
) -> anyhow::Result<Response<BoxBody>> {
    let conn = data.pool.get().await?;

    let now = time::OffsetDateTime::now_utc();
    let day = now.day();
    let start = (now - (Duration::DAY * day)).replace_time(Time::MIDNIGHT);
    let end = start + (Duration::DAY * 31);

    if data
        .session_service
        .verify_auth_cookie_has_permissions(&conn, cookies.clone(), 0x1)
        .await?
    {
        let entries = get_all_dashboard_entries(&conn, start, end).await?;

        let template = DashboardTemplate { entries, start: start.unix_timestamp(), end: end.unix_timestamp() };

        Ok(AskamaTemplate(&template).into_response())
    } else if let Some(account_id) = data
        .session_service
        .verify_auth_cookie(&conn, cookies)
        .await?
    {
        let entries = vec![get_dashboard_entry(&conn, account_id, start, end).await?];

        let template = DashboardTemplate { entries, start: start.unix_timestamp(), end: end.unix_timestamp() };

        Ok(AskamaTemplate(&template).into_response())
    } else {
        Ok(Redirect::to(Uri::from_static("/account/login"))
            .into_response()
            .map(boxed))
    }
}
