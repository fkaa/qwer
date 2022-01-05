use std::sync::Arc;

use anyhow::Context;
use ascii::AsciiStr;
use axum::{
    body::{boxed, BoxBody, Empty},
    extract::{Extension, Form},
    response::{IntoResponse, Redirect},
};
use http::{Response, StatusCode, Uri};
use lettre::{
    message::{Mailbox, MultiPart, SinglePart},
    transport::smtp::authentication::{Credentials, Mechanism},
    Message, SmtpTransport, Transport,
};
use serde::Deserialize;
use time::OffsetDateTime;
use tracing::*;

use crate::{unwrap_response, AppData, PostgresConnection};

use super::session::Cookies;

const SECRET_CHARSET: &[u8; 62] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

#[derive(Deserialize)]
pub(crate) struct SendAccountEmail {
    email: String,
}

pub(crate) async fn send_account_email_post_handler(
    Form(form): Form<SendAccountEmail>,
    Extension(data): Extension<Arc<AppData>>,
    cookies: Cookies,
) -> Response<BoxBody> {
    unwrap_response(send_account_creation_email(&data, &form.email, cookies).await)
}

async fn send_account_creation_email(
    data: &AppData,
    address: &str,
    cookie: Cookies,
) -> anyhow::Result<Response<BoxBody>> {
    let mut conn = data.pool.get().await?;

    if data
        .session_service
        .verify_auth_cookie_has_permissions(&conn, cookie, 0x1)
        .await?
    {
        send_account_creation_email_internal(&mut conn, data, address).await?;

        Ok(Redirect::to(Uri::from_static("/account/dashboard"))
            .into_response()
            .map(boxed))
    } else {
        Ok(Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(boxed(Empty::new()))
            .unwrap())
    }
}

fn generate_activation_secret(dest: &mut [u8; 32]) {
    for c in dest.iter_mut() {
        *c = SECRET_CHARSET[fastrand::usize(..SECRET_CHARSET.len())];
    }
}

async fn send_account_creation_email_internal(
    conn: &mut PostgresConnection<'_>,
    data: &AppData,
    address: &str,
) -> anyhow::Result<()> {
    debug!("Sending account creation email to {}", address);

    let tx = conn.transaction().await?;

    let credentials = Credentials::new(data.smtp_user.clone(), data.smtp_pass.clone());

    let mailer = SmtpTransport::starttls_relay(&data.smtp_server)?
        .credentials(credentials)
        .authentication(vec![Mechanism::Plain])
        .build();

    let mut secret = [0u8; 32];
    generate_activation_secret(&mut secret);
    let secret = AsciiStr::from_ascii(&secret[..])?;

    let from: Mailbox = format!("{} <no-reply@{}>", data.site_domain, data.site_domain)
        .parse()
        .context("parsing email source address")?;
    let to: Mailbox = format!("<{}>", address)
        .parse()
        .context("parsing email destination address")?;

    debug!("Sending account creation email to {} from {}", to, from);

    let email = Message::builder()
        .from(from.clone())
        .reply_to(from)
        .to(to)
        .subject("Create an account")
        .multipart(
            MultiPart::alternative()
                .singlepart(SinglePart::plain(format!(
                    "Go to {}account/activate/{} to create a new account",
                    data.web_url, secret
                )))
                .singlepart(SinglePart::html(format!(
                    "<html><body>Click <a href=\"{}account/activate/{}\">here</a> to create a new account</body></html>",
                    data.web_url, secret
                ))),
        )
        .unwrap();

    mailer.send(&email).context("Sending e-mail")?;

    debug!("Sent email!");

    let now = OffsetDateTime::now_utc();

    tx.execute(
        "
INSERT INTO account_approval (email, time_sent, secret)
VALUES ($1, $2, $3)
",
        &[&address, &now.unix_timestamp(), &secret.as_str()],
    )
    .await?;

    tx.commit().await?;

    debug!("Finished sending email!");

    Ok(())
}
