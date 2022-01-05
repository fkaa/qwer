use std::sync::Arc;

use askama::Template;
use axum::{
    body::{boxed, BoxBody},
    extract::Extension,
    response::{IntoResponse, Redirect},
};
use http::{Response, Uri};

use crate::{unwrap_response, AppData, AskamaTemplate};

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
) -> Response<BoxBody> {
    unwrap_response(get_account_details(&data, cookies).await.map(|d| {
        if let Some(details) = d {
            AskamaTemplate(&details).into_response()
        } else {
            Redirect::to(Uri::from_static("/account/login"))
                .into_response()
                .map(boxed)
        }
    }))
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
