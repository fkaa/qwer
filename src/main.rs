use axum::{
    handler::{get, post},
    response::Html,
    AddExtensionLayer, Router,
};

use slog::info;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

mod logger;
mod media;
mod mp4;
mod webrtc;

mod transport {
    pub mod http;
    pub mod mse;
    pub mod webrtc;
}

mod ingest {
    pub mod mpegts;
    pub mod rtmp;
}

use logger::*;
use media::*;

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
    let data = AppData {
        stream_repo,
        logger: web_logger,
    };

    let app = Router::new()
        .route("/", get(handler))
        .route(
            "/ingest/mpegts/:stream",
            post(ingest::mpegts::mpegts_ingest),
        )
        .route(
            "/transport/webrtc/:stream",
            get(transport::webrtc::websocket_webrtc_signalling),
        )
        .route(
            "/transport/mse/:stream",
            get(transport::mse::websocket_video),
        )
        .route("/transport/http/:stream", get(transport::http::http_video))
        .layer(AddExtensionLayer::new(Arc::new(data)));

    hyper::Server::bind(&web_addr.parse()?)
        .tcp_nodelay(true)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_root, logger) = logger::initialize();
    // tracing_subscriber::fmt::init();
    let _ = dotenv::dotenv();

    let web_addr =
        std::env::var("WEB_BIND_ADDRESS").unwrap_or_else(|_| String::from("0.0.0.0:8080"));
    let rtmp_addr =
        std::env::var("RTMP_BIND_ADDRESS").unwrap_or_else(|_| String::from("127.0.0.1:1935"));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { start(logger, &web_addr, rtmp_addr).await })?;

    Ok(())
}
