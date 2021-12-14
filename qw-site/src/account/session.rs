use std::iter;

use axum::extract::{FromRequest, RequestParts};
use cookie::{Cookie, CookieJar, Key, SameSite};
use headers::UserAgent;
use http::{header::COOKIE, StatusCode};
use rand::{distributions::Alphanumeric, rngs::OsRng, Rng};
use tracing::*;

use crate::PostgresConnection;

pub struct AccountSessionService {
    domain: String,
    key: Key,
}

impl AccountSessionService {
    pub fn new(domain: String, key: &str) -> Self {
        let key = Key::from(key.as_bytes());

        AccountSessionService { domain, key }
    }

    pub async fn create_auth_cookie(
        &self,
        conn: &PostgresConnection<'_>,
        account_id: i32,
        user_agent: Option<UserAgent>,
    ) -> anyhow::Result<Cookie<'static>> {
        let id: String = iter::repeat(())
            .map(|()| OsRng.sample(Alphanumeric))
            .map(char::from)
            .take(64)
            .collect();

        let mut jar = CookieJar::new();
        jar.private_mut(&self.key)
            .add(Cookie::new("__auth", id.clone()));
        let mut cookie = jar.get("__auth").unwrap().to_owned();
        cookie.set_http_only(true);
        cookie.set_secure(true);
        cookie.set_same_site(SameSite::Strict);
        cookie.set_domain(self.domain.clone());
        cookie.make_permanent();

        debug!("session id: {}, encrypted: {}", id, cookie.value());

        let ua = user_agent.map(|ua| ua.as_str().to_string());

        conn.execute(
            "
INSERT INTO account_session (id, account_id, user_agent)
VALUES ($1, $2, $3)
",
            &[&id, &account_id, &ua],
        )
        .await?;

        Ok(cookie)
    }

    pub async fn verify_auth_cookie_has_permissions(
        &self,
        conn: &PostgresConnection<'_>,
        cookies: Cookies,
        permissions: i32,
    ) -> anyhow::Result<bool> {
        let private_jar = cookies.0.private(&self.key);

        if let Some(session_id) = private_jar.get("__auth") {
            let row = conn
                .query_opt(
                    "
SELECT account_permission.permissions
FROM account_permission
INNER JOIN account_session
ON account_permission.account_id = account_session.account_id
WHERE account_session.id = $1
                ",
                    &[&session_id.value()],
                )
                .await?;

            Ok(row
                .map(|r| r.get::<_, i32>(0) & permissions == permissions)
                .unwrap_or(false))
        } else {
            Ok(false)
        }
    }

    pub async fn verify_auth_cookie(
        &self,
        conn: &PostgresConnection<'_>,
        cookies: Cookies,
    ) -> anyhow::Result<Option<i32>> {
        let private_jar = cookies.0.private(&self.key);

        if let Some(session_id) = private_jar.get("__auth") {
            let row = conn
                .query_opt(
                    "
SELECT account_id FROM account_session
WHERE id = $1
                ",
                    &[&session_id.value()],
                )
                .await?;

            Ok(row.map(|r| r.get::<_, i32>(0)))
        } else {
            Ok(None)
        }
    }
}

pub struct Cookies(CookieJar);

impl Cookies {
    pub fn has_auth_cookie(&self) -> bool {
        self.0.get("__auth").is_some()
    }
}

#[async_trait::async_trait]
impl<B> FromRequest<B> for Cookies
where
    B: Send,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self, Self::Rejection> {
        let mut jar = CookieJar::new();

        if let Some(Ok(cookie)) = req
            .headers()
            .and_then(|h| h.get(COOKIE))
            .map(|c| c.to_str())
        {
            for cookie in cookie.split(';') {
                if let Ok(cookie) = Cookie::parse_encoded(cookie) {
                    jar.add(cookie.into_owned());
                }
            }
        }

        Ok(Cookies(jar))
    }
}
