use std::sync::Arc;

use askama::Template;
use axum::{
    body::{boxed, BoxBody, Empty},
    extract::Extension,
    response::IntoResponse,
};
use http::{Response, StatusCode};

use crate::{
    stream_service::get_stream_sessions, unwrap_response, AppData, AskamaTemplate,
    PostgresConnection,
};

use super::session::Cookies;

struct DashboardBwSample {
    time: i64,
    bytes: Option<u64>,
}

struct DashboardEntry {
    account_name: String,
    data: Vec<DashboardBwSample>,
}

async fn get_bitrates(
    conn: &PostgresConnection<'_>,
    stream_session_id: i32,
) -> anyhow::Result<Vec<(time::OffsetDateTime, u32)>> {
    let samples = conn
        .query(
            "
SELECT time, bytes_since_prev FROM bandwidth_usage
WHERE stream_session_id = $1
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
) -> anyhow::Result<DashboardEntry> {
    let start = time::OffsetDateTime::UNIX_EPOCH;
    let end = time::OffsetDateTime::now_utc();

    let account_name = get_account_name(conn, account_id).await?;
    let sessions = get_stream_sessions(conn, account_id, start, end).await?;

    let mut data = Vec::new();

    let mut total_bytes = 0;
    for session in sessions {
        data.push(DashboardBwSample {
            time: session.start.unix_timestamp(),
            bytes: None,
        });

        let samples = get_bitrates(conn, session.id).await?;

        for (time, bytes) in samples {
            total_bytes += bytes as u64;

            data.push(DashboardBwSample {
                time: time.unix_timestamp(),
                bytes: Some(total_bytes),
            });
        }

        if let Some(stop) = session.stop {
            data.push(DashboardBwSample {
                time: stop.unix_timestamp(),
                bytes: None,
            });
        }
    }

    Ok(DashboardEntry { account_name, data })
}

async fn get_all_dashboard_entries(
    conn: &PostgresConnection<'_>,
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
        entries.push(get_dashboard_entry(conn, id).await?);
    }

    Ok(entries)
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    entries: Vec<DashboardEntry>,
}

pub(crate) async fn dashboard_page_get_handler(
    Extension(data): Extension<Arc<AppData>>,
    cookies: Cookies,
) -> Response<BoxBody> {
    unwrap_response(dashboard_page(&data, cookies).await)
}

async fn dashboard_page(
    data: &Arc<AppData>,
    cookies: Cookies,
) -> anyhow::Result<Response<BoxBody>> {
    let conn = data.pool.get().await?;

    if data
        .session_service
        .verify_auth_cookie_has_permissions(&conn, cookies, 0x1)
        .await?
    {
        let entries = get_all_dashboard_entries(&conn).await?;

        let template = DashboardTemplate { entries };

        Ok(AskamaTemplate(&template).into_response())
    } else {
        Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(boxed(Empty::new()))
            .unwrap())
    }
}
