use axum::{
    Router,
    response::{Html},
    handler::get,
    AddExtensionLayer,
};

use slog::{info};

use std::collections::{HashMap};
use std::sync::{Arc, RwLock};

mod media;
mod mp4;
mod logger;

mod transport {
    pub mod webrtc;
    pub mod mse;
    pub mod http;
}

mod ingest {
    pub mod rtmp;
}

use media::*;
use logger::*;

#[derive(Default)]
pub struct StreamRepository {
    pub streams: HashMap<String, MediaFrameQueue>,
}

#[derive(Clone)]
pub struct AppData {
    pub stream_repo: Arc<RwLock<StreamRepository>>,
    pub logger: ContextLogger,
}

async fn handler() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}

async fn start(logger: ContextLogger, web_addr: &str, rtmp_addr: String) -> anyhow::Result<()> {
    let stream_repo = Arc::new(RwLock::new(StreamRepository::default()));

    ingest::rtmp::listen(logger.scope(), stream_repo.clone(), rtmp_addr);

    let web_logger = logger.scope();
    info!(web_logger, "Starting webserver at {}", web_addr);
    let data = AppData { stream_repo, logger: web_logger, };

    let app = Router::new()
        .route("/", get(handler))
        .route("/mse/:stream", get(transport::mse::websocket_video))
        .route("/http/:stream", get(transport::http::http_video))
        .layer(AddExtensionLayer::new(Arc::new(data)));

    axum::Server::bind(&web_addr.parse()?)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, logger) = logger::initialize();
    tracing_subscriber::fmt::init();
    let _ = dotenv::dotenv();


    let web_addr =
        std::env::var("WEB_BIND_ADDRESS").unwrap_or_else(|_| String::from("127.0.0.1:8080"));
    let rtmp_addr =
        std::env::var("RTMP_BIND_ADDRESS").unwrap_or_else(|_| String::from("127.0.0.1:1935"));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            start(logger, &web_addr, rtmp_addr).await
        })?;

    Ok(())
}
