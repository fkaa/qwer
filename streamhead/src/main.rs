use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        Extension, Path,
    },
    handler::get,
    response::{Html, IntoResponse},
    AddExtensionLayer, Router,
};
use tracing::*;

use sh_media::{FilterGraph, FrameAnalyzerFilter, MediaFrameQueue};

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Default)]
pub struct StreamRepository {
    pub streams: HashMap<String, MediaFrameQueue>,
}

#[derive(Clone)]
pub struct AppData {
    pub stream_repo: Arc<RwLock<StreamRepository>>,
}

async fn handler() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}

async fn rtmp_ingest(
    app: String,
    request: sh_ingest_rtmp::RtmpRequest,
    repo: Arc<RwLock<StreamRepository>>,
) -> anyhow::Result<()> {
    use sh_ingest_rtmp::RtmpReadFilter;

    let session = request.authenticate().await?;
    let queue = MediaFrameQueue::new();
    let rtmp_filter = RtmpReadFilter::new(session);
    let rtmp_analyzer = FrameAnalyzerFilter::read(Box::new(rtmp_filter));

    let mut graph = FilterGraph::new(Box::new(rtmp_analyzer), Box::new(queue.clone()));

    info!("Starting a stream at '{}'", app);

    repo.write().unwrap().streams.insert(app.clone(), queue);

    if let Err(e) = graph.run().await {
        error!("Error while reading from RTMP stream: {}", e);
    }

    info!("Stopping a stream at '{}'", app);

    repo.write().unwrap().streams.remove(&app);

    Ok(())
}

async fn listen_rtmp(addr: String, repo: Arc<RwLock<StreamRepository>>) -> anyhow::Result<()> {
    use sh_ingest_rtmp::RtmpListener;

    info!("Listening for RTMP at {}", addr);
    let listener = RtmpListener::bind(addr).await?;

    loop {
        let (req, app, key) = match listener.accept().await {
            Ok((r, a, k)) => (r, a, k),
            Err(e) => {
                warn!("Error while accepting RTMP connection: {}", e);
                continue;
            }
        };

        info!("Got a RTMP session from {} on stream '{}'", req.addr(), app);
        if authenticate_rtmp_stream(&app, &key) {
            let repo = repo.clone();
            tokio::spawn(async move {
                if let Err(e) = rtmp_ingest(app, req, repo).await {
                    error!("Error while ingesting RTMP: {}", e);
                }
            });
        } else {
            // TODO: disconnect properly?
        }
    }
}

fn authenticate_rtmp_stream(_app_name: &str, supplied_stream_key: &str) -> bool {
    std::env::var("STREAM_KEY")
        .map(|key| key == supplied_stream_key)
        .unwrap_or(true)
}

pub async fn websocket_video(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| handle_websocket_video_response(socket, stream, data.clone()))
}

async fn handle_websocket_video_response(socket: WebSocket, stream: String, data: Arc<AppData>) {
    let queue_receiver = data
        .stream_repo
        .read()
        .unwrap()
        .streams
        .get(&stream)
        .map(|s| s.get_receiver());

    if let Some(mut queue_receiver) = queue_receiver {
        debug!("Found a stream at {}", stream);

        if let Err(e) = sh_transport_mse::start_websocket_filters(socket, &mut queue_receiver).await
        {
            error!("{}", e);
        }
    } else {
        debug!("Did not find a stream at {}", stream);
    }
}

async fn start(web_addr: &str, rtmp_addr: String) -> anyhow::Result<()> {
    let stream_repo = Arc::new(RwLock::new(StreamRepository::default()));

    let repo = stream_repo.clone();
    tokio::spawn(async {
        if let Err(e) = listen_rtmp(rtmp_addr, repo).await {
            error!("Error while listening on RTMP: {}", e);
        }
    });

    info!("Starting webserver at {}", web_addr);
    let data = AppData { stream_repo };

    let app = Router::new()
        .route("/", get(handler))
        .route("/transport/mse/:stream", get(websocket_video))
        //.route("/streams", get(streams))
        .layer(AddExtensionLayer::new(Arc::new(data)));

    hyper::Server::bind(&web_addr.parse()?)
        .tcp_nodelay(true)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::TRACE)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let _ = dotenv::dotenv();

    let web_addr =
        std::env::var("WEB_BIND_ADDRESS").unwrap_or_else(|_| String::from("0.0.0.0:8080"));
    let rtmp_addr =
        std::env::var("RTMP_BIND_ADDRESS").unwrap_or_else(|_| String::from("127.0.0.1:1935"));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { start(&web_addr, rtmp_addr).await })?;

    Ok(())
}
