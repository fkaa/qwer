use account::session::AccountSessionService;
use axum::{
    body::{boxed, Body, BoxBody, Bytes, Full, HttpBody},
    extract::{Extension, Path},
    response::{IntoResponse, Redirect},
    routing::get,
    AddExtensionLayer, BoxError, Router,
};

use anyhow::Context;
use askama::Template;
use bb8::ManageConnection;
use futures::{future, StreamExt};
use http::{Response, StatusCode};

use tokio::sync::mpsc::{self, Sender};
use tonic::transport::Endpoint;
use tracing::*;

use qw_proto::{
    stream_auth::stream_auth_service_server::StreamAuthServiceServer,
    stream_info::{
        stream_info_client::StreamInfoClient, stream_reply::StreamType, StreamReply, StreamRequest,
    },
};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use std::{env, time::Duration};
use std::{
    net::{SocketAddr, ToSocketAddrs},
    sync::Arc,
};

mod stream_auth;
mod stream_service;

mod account;

use crate::stream_auth::ScuffedStreamAuthService;

pub type PostgresManager = bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>;
pub type PostgresPool = bb8::Pool<PostgresManager>;
pub type PostgresConnection<'a> = bb8::PooledConnection<'a, PostgresManager>;

refinery::embed_migrations!("./migrations");

pub fn unwrap_response<B>(result: anyhow::Result<Response<B>>) -> Response<BoxBody>
where
    B: HttpBody<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
{
    match result {
        Ok(resp) => resp.map(|b| boxed(b)),
        Err(e) => Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(boxed(Full::from(format!("{:?}", e))))
            .unwrap(),
    }
}

#[derive(Clone)]
pub struct AppData {
    pub pool: Arc<PostgresPool>,
    pub session_service: Arc<AccountSessionService>,
    pub transport_address: String,
    pub smtp_server: String,
    pub smtp_user: String,
    pub smtp_pass: String,
    pub secret_key: String,
    pub web_url: String,
    pub site_domain: String,
}

pub(crate) struct AskamaTemplate<'a, T>(&'a T);

impl<'a, T: askama::Template> IntoResponse for AskamaTemplate<'a, T> {
    fn into_response(self) -> Response<BoxBody> {
        let mut buffer = String::with_capacity(self.0.size_hint());
        if let Err(e) = self.0.render_into(&mut buffer) {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(boxed(Body::from(format!("{:?}", e))))
                .unwrap();
        }

        Response::builder()
            .status(StatusCode::OK)
            .body(boxed(Body::from(buffer)))
            .unwrap()
    }
}

#[derive(Template)]
#[template(path = "stream.html")]
struct StreamTemplate<'a> {
    transport_address: &'a str,
    stream: &'a str,
}

async fn stream_page(
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> Response<BoxBody> {
    let template = StreamTemplate {
        transport_address: &data.transport_address,
        stream: &stream,
    };

    AskamaTemplate(&template).into_response()
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {}

async fn index_page() -> Response<BoxBody> {
    let template = IndexTemplate {};

    AskamaTemplate(&template).into_response()
}

struct StreamItem {
    name: String,
    viewers: u32,
}

#[derive(Template)]
#[template(path = "streams.html")]
struct StreamsTemplate<'a> {
    transport_address: &'a str,
    streams: &'a [StreamItem],
}

async fn streams_page(Extension(data): Extension<Arc<AppData>>) -> Response<BoxBody> {
    let conn = data.pool.get().await.unwrap();

    let sessions = stream_service::get_active_stream_sessions(&conn)
        .await
        .unwrap()
        .into_iter()
        .map(|s| StreamItem {
            name: s.account_name,
            viewers: s.viewer_count as _,
        })
        .collect::<Vec<_>>();

    let template = StreamsTemplate {
        transport_address: &data.transport_address,
        streams: &sessions,
    };

    AskamaTemplate(&template).into_response()
}

async fn run_migrations(
    manager: &bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>,
) -> anyhow::Result<()> {
    let runner = migrations::runner();
    let migrations = runner.get_migrations();
    let mut conn = manager.connect().await?;

    info!(
        "Found the following migrations: {}",
        migrations
            .iter()
            .map(|m| format!("{}", m))
            .collect::<Vec<_>>()
            .join(", ")
    );

    runner.run_async(&mut conn).await?;

    let latest_migration = runner.get_last_applied_migration_async(&mut conn).await?;
    if let Some(migration) = latest_migration {
        info!("Latest applied migration: {}", format!("{}", migration));
    }

    Ok(())
}

async fn start() -> anyhow::Result<()> {
    let ingest_web_addr = env("INGEST_WEB_ADDR", "localhost:8080");
    let ingest_rpc_addr = env("INGEST_RPC_ADDR", "localhost:8081");

    let scuffed_rpc_addr = resolve_env_addr("QW_RPC_ADDR", "localhost:9082");
    let scuffed_addr = resolve_env_addr("QW_WEB_ADDR", "localhost:9082");

    let manager = bb8_postgres::PostgresConnectionManager::new_from_stringlike(
        &env::var("DATABASE_URL").context("DATABASE_URL not set")?,
        tokio_postgres::NoTls,
    )
    .context("failed to create database connection")?;

    run_migrations(&manager)
        .await
        .context("failed to run migrations")?;

    let pool = Arc::new(
        bb8::Pool::builder()
            .build(manager)
            .await
            .context("failed to build database pool")?,
    );

    let (send, recv) = mpsc::channel(1024);

    let mut stream_session_service = stream_service::StreamSessionService::new(recv, pool.clone());
    stream_session_service
        .start()
        .await
        .context("failed to start stream session")?;

    let _stream_session_poll_task = tokio::spawn(async move {
        stream_session_service.poll().await.unwrap();
    });

    let scuffed_service = ScuffedStreamAuthService::new(pool.clone());

    let smtp_server = env::var("QW_SMTP_SERVER").context("QW_SMTP_SERVER not set")?;
    let smtp_user = env::var("QW_SMTP_USER").context("QW_SMTP_USER not set")?;
    let smtp_pass = env::var("QW_SMTP_PASS").context("QW_SMTP_PASS not set")?;
    let secret_key = env::var("QW_SECRET_KEY").context("QW_SECRET_KEY not set")?;
    let web_url = env::var("QW_WEB_URL").context("QW_WEB_URL not set")?;
    let site_domain = env::var("QW_DOMAIN").context("QW_DOMAIN not set")?;

    let session_service = Arc::new(AccountSessionService::new(
        site_domain.clone(),
        &secret_key,
    ));

    let data = AppData {
        transport_address: ingest_web_addr,
        session_service,
        smtp_server,
        smtp_user,
        smtp_pass,
        secret_key,
        web_url,
        site_domain,
        pool: pool.clone(),
    };

    let app = Router::new()
        .route("/:stream", get(stream_page))
        .route("/", get(index_page))
        .route("/streams", get(streams_page))
        .route(
            "/help",
            get(|| async { Redirect::permanent("/help/".parse().unwrap()) }),
        )
        .nest("/account", account::api_route())
        .layer(AddExtensionLayer::new(Arc::new(data)));

    spawn_stream_info_loop(ingest_rpc_addr, send);

    let rpc_task = tokio::spawn(async move {
        debug!("Listening for RPC calls on {}", scuffed_rpc_addr);

        tonic::transport::Server::builder()
            .add_service(StreamAuthServiceServer::new(scuffed_service))
            .serve(scuffed_rpc_addr)
            .await
            .unwrap();

        debug!("Finished listening for RPC calls");
    });

    let webserver_task = tokio::spawn(async move {
        hyper::Server::bind(&scuffed_addr)
            .serve(app.into_make_service())
            .await
            .unwrap();
    });

    future::select(webserver_task, rpc_task).await;

    debug!("Either web server or RPC server finished");

    Ok(())
}

async fn stream_info_listen(
    ingest_rpc_addr: String,
    send: Sender<StreamType>,
) -> anyhow::Result<()> {
    let client_endpoint = Endpoint::from_shared(ingest_rpc_addr.clone())
        .unwrap()
        .connect_lazy();
    let mut client = StreamInfoClient::new(client_endpoint);

    debug!("Listening to stream info from {}", ingest_rpc_addr);

    let stream = client
        .listen(StreamRequest {
            stats_interval_seconds: 5,
        })
        .await;

    let mut stream = stream.map(|s| s.into_inner())?;

    while let Some(msg) = stream.next().await {
        let StreamReply { stream_type } = msg?;
        if let Some(ty) = stream_type {
            debug!("{:?}", ty);

            send.send(ty.clone()).await.unwrap();
        }
    }

    Ok(())
}

fn spawn_stream_info_loop(ingest_rpc_addr: String, send: Sender<StreamType>) {
    tokio::spawn(async move {
        use backoff::{future::retry, ExponentialBackoff};

        let mut backoff = ExponentialBackoff::default();
        backoff.max_elapsed_time = Some(Duration::from_secs_f64(10.0));

        let _ = retry(backoff, || async {
            if let Err(e) = stream_info_listen(ingest_rpc_addr.clone(), send.clone()).await {
                warn!("Error while listening for stream info: {}", e);

                Err(backoff::Error::Transient(e))
            } else {
                Ok(())
            }
        })
        .await;
    });
}

fn env(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.into())
}

fn resolve_env_addr(var: &str, default: &str) -> SocketAddr {
    std::env::var(var)
        .unwrap_or_else(|_| default.into())
        .to_socket_addrs()
        .unwrap()
        .next()
        .expect(&format!("Failed to resolve {}", var))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenv::dotenv();

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("debug"))
        .unwrap();

    let subscriber = FmtSubscriber::builder().with_env_filter(filter).finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { start().await })?;

    Ok(())
}
