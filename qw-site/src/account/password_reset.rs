use std::sync::Arc;

use anyhow::Context;
use argon2::{password_hash::SaltString, Argon2, PasswordHasher};
use ascii::AsciiStr;
use askama::Template;
use axum::{
    body::{boxed, BoxBody, Empty},
    extract::{Extension, Form, Path},
    http::{Response, StatusCode},
    response::IntoResponse,
};
use lettre::{
    message::{Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::{Credentials, Mechanism},
    Message, SmtpTransport, Transport,
};
use rand::rngs::OsRng;
use serde::Deserialize;
use time::OffsetDateTime;
use tracing::debug;

use crate::{account::registration::generate_secret, AppData, AskamaTemplate};

#[derive(Template)]
#[template(path = "forgot_password.html")]
struct ForgotPasswordTemplate;

pub(crate) async fn forgot_password_page_get_handler() -> Response<BoxBody> {
    AskamaTemplate(&ForgotPasswordTemplate).into_response()
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct ForgotPasswordForm {
    email: String,
}

pub(crate) async fn forgot_password_post_handler(
    Form(form): Form<ForgotPasswordForm>,
    Extension(data): Extension<Arc<AppData>>,
) -> crate::Result<&'static str> {
    send_reset_email(&data, form).await?;

    Ok("Success!")
}

async fn send_reset_email(data: &Arc<AppData>, form: ForgotPasswordForm) -> anyhow::Result<()> {
    let mut conn = data.pool.get().await?;
    let tx = conn.transaction().await?;

    let account_id = tx
        .query_opt(
            "
SELECT id FROM account
WHERE email = $1
",
            &[&form.email],
        )
        .await?
        .map(|r| r.get::<_, i32>(0))
        .ok_or(anyhow::anyhow!("Failed to get account id"))?;

    let credentials = Credentials::new(data.smtp_user.clone(), data.smtp_pass.clone());

    let mailer = SmtpTransport::starttls_relay(&data.smtp_server)?
        .credentials(credentials)
        .authentication(vec![Mechanism::Plain])
        .build();

    let mut secret = [0u8; 32];
    generate_secret(&mut secret);
    let secret = AsciiStr::from_ascii(&secret[..])?;

    let from: Mailbox = format!("{} <no-reply@{}>", data.site_domain, data.site_domain)
        .parse()
        .context("parsing email source address")?;
    let to: Mailbox = format!("<{}>", form.email)
        .parse()
        .context("parsing email destination address")?;

    debug!("Sending account creation email to {} from {}", to, from);

    let email = Message::builder()
        .from(from.clone())
        .reply_to(from)
        .to(to)
        .subject("Reset your password")
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(format!(
                    "Go to {}account/reset/{} to change your password",
                    data.web_url, secret
                )))
                .singlepart(SinglePart::html(format!(
                    "<html><body>Click <a href=\"{}account/reset/{}\">here</a> to change your password</body></html>",
                    data.web_url, secret
                ))),
        )
        .unwrap();

    mailer.send(&email).context("Sending e-mail")?;

    debug!("Sent email!");

    let now = OffsetDateTime::now_utc();

    tx.execute(
        "
INSERT INTO password_reset (secret, account_id, time_sent)
VALUES ($1, $2, $3)
",
        &[&secret.as_str(), &account_id, &now.unix_timestamp()],
    )
    .await?;

    tx.commit().await?;

    Ok(())
}

#[derive(Template)]
#[template(path = "reset_password.html")]
struct ChangePasswordTemplate<'a> {
    secret: &'a str,
}

pub(crate) async fn change_password_page_get_handler(
    Path(secret): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> Response<BoxBody> {
    match account_by_secret(&data, &secret).await {
        Ok(_) => {
            let template = ChangePasswordTemplate { secret: &secret };

            AskamaTemplate(&template).into_response()
        }
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(boxed(Empty::new()))
            .unwrap(),
    }
}

async fn account_by_secret(data: &Arc<AppData>, secret: &str) -> anyhow::Result<i32> {
    let conn = data.pool.get().await?;
    let row = conn
        .query_opt(
            "
SELECT account_id FROM password_reset
WHERE secret = $1
",
            &[&secret],
        )
        .await?;

    row.map(|r| r.get::<_, i32>(0))
        .ok_or(anyhow::anyhow!("Failed to get account id"))
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) struct ResetAccountForm {
    password: String,
    repeat_password: String,
    secret: String,
}

pub(crate) async fn reset_password_post_handler(
    Form(form): Form<ResetAccountForm>,
    Extension(data): Extension<Arc<AppData>>,
) -> crate::Result<&'static str> {
    reset_password(&data, form).await?;

    Ok("Success!")
}

async fn reset_password(data: &Arc<AppData>, form: ResetAccountForm) -> anyhow::Result<()> {
    if form.password != form.repeat_password {
        anyhow::bail!("Passwords are not the same!");
    }

    let mut conn = data.pool.get().await?;
    let tx = conn.transaction().await?;

    let account_id = account_by_secret(data, &form.secret).await?;

    tx.execute(
        "
DELETE FROM password_reset
WHERE secret = $1
",
        &[&form.secret],
    )
    .await?;

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let phc_string = argon2
        .hash_password(form.password.as_bytes(), &salt)?
        .to_string();

    tx.execute(
        "
UPDATE account
SET password_hash = $1
WHERE id = $2
",
        &[&phc_string, &account_id],
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
