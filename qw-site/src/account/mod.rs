use axum::{
    routing::{get, post},
    Router,
};

pub mod activation;
pub mod dashboard;
pub mod login;
pub mod page;
pub mod password_reset;
pub mod registration;
pub mod session;

pub fn api_route() -> Router {
    Router::new()
        .route(
            "/send-account-email",
            post(registration::send_account_email_post_handler),
        )
        .route(
            "/new-stream-key",
            post(activation::generate_new_stream_key_post_handler),
        )
        .route("/dashboard", get(dashboard::dashboard_page_get_handler))
        .route(
            "/activate/:secret",
            get(activation::create_account_page_get_handler),
        )
        .route("/activate", post(activation::create_account_post_handler))
        .route(
            "/forgot-password",
            get(password_reset::forgot_password_page_get_handler),
        )
        .route(
            "/forgot-password",
            post(password_reset::forgot_password_post_handler),
        )
        .route(
            "/reset/:secret",
            get(password_reset::change_password_page_get_handler),
        )
        .route("/reset", post(password_reset::reset_password_post_handler))
        .route("/", get(page::account_page_get_handler))
        .route(
            "/login",
            get(login::login_page_get_handler).post(login::login_account_post_handler),
        )
}
