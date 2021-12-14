use std::{iter, sync::Arc};

use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use askama::Template;
use axum::{
    body::{boxed, BoxBody, Empty},
    extract::{Extension, Form, Path},
    response::{IntoResponse, Redirect},
};
use http::{Response, StatusCode, Uri};
use rand::{distributions::Alphanumeric, rngs::OsRng, Rng};
use serde::Deserialize;

use crate::{unwrap_response, AppData, AskamaTemplate};

use super::session::Cookies;

#[derive(Template)]
#[template(path = "create_account.html")]
struct CreateAccountTemplate<'a> {
    email: &'a str,
    secret: &'a str,
}

pub(crate) async fn create_account_page_get_handler(
    Path(secret): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> Response<BoxBody> {
    match secret_exists(&data, &secret).await {
        Ok(email) => {
            let template = CreateAccountTemplate {
                email: &email,
                secret: &secret,
            };

            AskamaTemplate(&template).into_response()
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(boxed(Empty::new()))
            .unwrap(),
    }
}

async fn secret_exists(data: &Arc<AppData>, secret: &str) -> anyhow::Result<String> {
    let conn = data.pool.get().await?;
    let row = conn
        .query_opt(
            "
SELECT email FROM account_approval
WHERE secret = $1
",
            &[&secret],
        )
        .await?;

    row.map(|r| r.get::<_, String>(0))
        .ok_or(anyhow::anyhow!(""))
}

pub(crate) async fn generate_new_stream_key_post_handler(
    Extension(data): Extension<Arc<AppData>>,
    cookies: Cookies,
) -> Response<BoxBody> {
    unwrap_response(regenerate_stream_key(&data, cookies).await.map(|authed| {
        if authed {
            Redirect::to(Uri::from_static("/account")).into_response()
        } else {
            Redirect::to(Uri::from_static("/account/login")).into_response()
        }
    }))
}

fn generate_secret(len: usize) -> String {
    iter::repeat_with(|| OsRng.sample(Alphanumeric))
        .map(char::from)
        .take(len)
        .collect()
}

async fn regenerate_stream_key(data: &Arc<AppData>, cookies: Cookies) -> anyhow::Result<bool> {
    let conn = data.pool.get().await?;

    if let Some(account_id) = data
        .session_service
        .verify_auth_cookie(&conn, cookies)
        .await?
    {
        let secret = generate_secret(32);

        let _ = conn
            .execute(
                "
UPDATE account
SET stream_key = $1
WHERE id = $2
",
                &[&secret, &account_id],
            )
            .await?;

        Ok(true)
    } else {
        Ok(false)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct CreateAccountForm {
    account_name: String,
    email: String,
    password: String,
    repeat_password: String,
    secret: String,
}

pub(crate) async fn create_account_post_handler(
    Form(form): Form<CreateAccountForm>,
    Extension(data): Extension<Arc<AppData>>,
) -> Response<BoxBody> {
    unwrap_response(create_account(&data, form).await.map(|_| {
        Response::builder()
            .status(StatusCode::OK)
            .body(Empty::new())
            .unwrap()
    }))
}

async fn create_account(data: &Arc<AppData>, form: CreateAccountForm) -> anyhow::Result<()> {
    if form.password != form.repeat_password {
        anyhow::bail!("Passwords are not the same!");
    }

    let mut conn = data.pool.get().await?;
    let tx = conn.transaction().await?;

    let rows = tx
        .execute(
            "
DELETE FROM account_approval
WHERE secret = $1
",
            &[&form.secret],
        )
        .await?;

    if rows == 0 {
        anyhow::bail!("Invalid secret {}", form.secret);
    }

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let phc_string = argon2
        .hash_password(form.password.as_bytes(), &salt)?
        .to_string();

    tx.execute(
        "
INSERT INTO account (name, password_hash, email, stream_key)
VALUES ($1, $2, $3, $4)
",
        &[&form.account_name, &phc_string, &form.email, &generate_secret(32)],
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
