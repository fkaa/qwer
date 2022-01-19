use std::{collections::HashMap, sync::Arc};

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use askama::Template;
use axum::{
    body::{boxed, BoxBody, Empty},
    extract::{Extension, Form, FromRequest, Query, RequestParts, TypedHeader},
    http::{
        header::{REFERER, SET_COOKIE},
        HeaderValue, Response, StatusCode, Uri,
    },
    response::{IntoResponse, Redirect},
};
use cookie::Cookie;
use headers::UserAgent;
use serde::Deserialize;

use crate::{AppData, AskamaTemplate};

#[derive(Template)]
#[template(path = "login.html")]
struct AccountLoginTemplate {
    redirect: String,
}

pub(crate) async fn login_page_get_handler(
    Query(params): Query<HashMap<String, String>>,
) -> Response<BoxBody> {
    let redirect = params
        .get("redirect")
        .cloned()
        .unwrap_or_else(|| String::from("account"));

    let template = AccountLoginTemplate { redirect };

    AskamaTemplate(&template).into_response()
}

#[derive(Deserialize)]
pub(crate) struct LoginAccountForm {
    email: String,
    password: String,
    redirect: String,
}

pub(crate) async fn login_account_post_handler(
    Form(form): Form<LoginAccountForm>,
    Extension(data): Extension<Arc<AppData>>,
    user_agent: Option<TypedHeader<UserAgent>>,
) -> crate::Result<Response<BoxBody>> {
    Ok(login(&data, &form, user_agent).await?)
}

async fn login(
    data: &Arc<AppData>,
    form: &LoginAccountForm,
    user_agent: Option<TypedHeader<UserAgent>>,
) -> anyhow::Result<Response<BoxBody>> {
    if let Some(cookie) = verify_login(data, form, user_agent.map(|h| h.0)).await? {
        let target = format!("/{}", form.redirect)
            .parse::<Uri>()
            .or_else(|_| "/account".parse::<Uri>())?;

        let mut response = Redirect::to(target).into_response();
        response.headers_mut().insert(
            SET_COOKIE,
            HeaderValue::try_from(cookie.encoded().to_string()).unwrap(),
        );

        Ok(response)
    } else {
        Ok(Response::builder()
            .status(StatusCode::FOUND)
            .header("Location", "/account/login#retry")
            .body(boxed(Empty::new()))
            .unwrap())
    }
}

async fn verify_login(
    data: &Arc<AppData>,
    form: &LoginAccountForm,
    user_agent: Option<UserAgent>,
) -> anyhow::Result<Option<Cookie<'static>>> {
    let conn = data.pool.get().await?;

    let row = conn
        .query_opt(
            "
SELECT id, password_hash FROM account
WHERE email = $1
",
            &[&form.email],
        )
        .await?;

    if let Some(row) = row {
        let id = row.get::<_, i32>(0);
        let hash = row.get::<_, String>(1);

        let argon2 = Argon2::default();
        let parsed_hash = PasswordHash::new(&hash)?;

        if argon2
            .verify_password(form.password.as_bytes(), &parsed_hash)
            .is_ok()
        {
            Ok(Some(
                data.session_service
                    .create_auth_cookie(&conn, id, user_agent)
                    .await?,
            ))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

pub struct ExtractReferer(HeaderValue);

#[async_trait::async_trait]
impl<B> FromRequest<B> for ExtractReferer
where
    B: Send,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let referer = req.headers().and_then(|headers| headers.get(REFERER));

        if let Some(referer) = referer {
            Ok(ExtractReferer(referer.clone()))
        } else {
            Err((StatusCode::BAD_REQUEST, "`Referer` header is missing"))
        }
    }
}
