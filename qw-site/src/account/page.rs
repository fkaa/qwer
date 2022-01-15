use std::sync::Arc;

use askama::Template;
use axum::{
    body::{boxed, BoxBody},
    extract::Extension,
    http::{Response, Uri},
    response::{IntoResponse, Redirect},
};

use crate::{AppData, AskamaTemplate};

use super::session::Cookies;

#[derive(Template)]
#[template(path = "account.html")]
struct AccountTemplate {
    name: String,
    email: String,
    stream_key: String,
}

pub(crate) async fn account_page_get_handler(
    Extension(data): Extension<Arc<AppData>>,
    cookies: Cookies,
) -> crate::Result<Response<BoxBody>> {
    if let Some(details) = get_account_details(&data, cookies).await? {
        Ok(AskamaTemplate(&details).into_response())
    } else {
        Ok(Redirect::to(Uri::from_static("/account/login"))
            .into_response()
            .map(boxed))
    }
}

async fn get_account_details(
    data: &Arc<AppData>,
    cookies: Cookies,
) -> anyhow::Result<Option<AccountTemplate>> {
    let conn = data.pool.get().await?;

    if let Some(account_id) = data
        .session_service
        .verify_auth_cookie(&conn, cookies)
        .await?
    {
        let row = conn
            .query_opt(
                "
SELECT name, email, stream_key FROM account
WHERE id = $1
    ",
                &[&account_id],
            )
            .await?
            .ok_or(anyhow::anyhow!("Account not found"))?;

        Ok(Some(AccountTemplate {
            name: row.get::<_, String>(0),
            email: row.get::<_, String>(1),
            stream_key: row.get::<_, String>(2),
        }))
    } else {
        Ok(None)
    }
}
