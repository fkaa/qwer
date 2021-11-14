use axum::{
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        Extension, Path,
    },
    handler::get,
    response::{Html, IntoResponse, sse::{Event, KeepAlive, Sse}},
    AddExtensionLayer, Router,
};
use tokio::sync::broadcast::{self, Sender, Receiver};
use tokio_stream::wrappers::BroadcastStream;
use futures::{future, Stream};
use serde::{Serialize, Deserialize};
use tracing::*;

use scuffed_proto::stream_info::{
    stream_info_server::{StreamInfoServer, StreamInfo},
    stream_reply::{
        StreamExisting, StreamStarted, StreamStopped, ViewerJoin, ViewerLeave, StreamType,
    },
    StreamRequest,
    StreamReply,
};
use sh_media::{FilterGraph, FrameAnalyzerFilter, MediaFrameQueueReceiver, MediaFrameQueue};

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    net::SocketAddr,
    pin::Pin,
};

pub struct StreamMetadata {
    queue: MediaFrameQueue,
    viewers: u32,
}

impl StreamMetadata {
    pub fn new(queue: MediaFrameQueue) -> Self {
        StreamMetadata {
            queue,
            viewers: 0,
        }
    }
}


pub struct StreamInfoService {
    data: Arc<AppData>,
}

type ResponseStream = Pin<Box<dyn Stream<Item = Result<StreamReply, tonic::Status>> + Send + Sync>>;

#[async_trait::async_trait]
impl StreamInfo for StreamInfoService {
    type ListenStream = ResponseStream;

    async fn listen(&self, _request: tonic::Request<StreamRequest>) -> Result<tonic::Response<Self::ListenStream>, tonic::Status> {
        use futures::StreamExt;

        debug!("listen RPC call");

        let (events, stream) = self.data.stream_repo
            .write()
            .unwrap()
            .subscribe();

        let event_stream = futures::stream::iter(events).map(|e| Some(e));

        let stream = event_stream
            .chain(stream)
            .map(|msg|
                 msg.ok_or(tonic::Status::aborted("stream overflowed")).map(|msg| StreamReply {
                     stream_type: Some(msg)
                 })
            );

        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

pub struct StreamRepository {
    pub streams: HashMap<String, StreamMetadata>,
    send: Sender<StreamType>,
    // channels: Vec<Sender<StreamEvent>>,
}

impl StreamRepository {
    pub fn new() -> Self {
        let (send, _) = broadcast::channel(512);

        StreamRepository {
            streams: HashMap::new(),
            send,
        }
    }

    pub fn start_stream(&mut self, name: String, queue: MediaFrameQueue) {
        let meta = StreamMetadata::new(queue);
        self.streams.insert(name.clone(), meta);
        self.send_event(StreamType::StreamStarted(StreamStarted { name }));
    }

    pub fn stop_stream(&mut self, name: String) {
        self.streams.remove(&name);
        self.send_event(StreamType::StreamStopped(StreamStopped { name }));
    }

    pub fn viewer_join(&mut self, stream: String) {
        if let Some(meta) = self.streams.get_mut(&stream) {
            meta.viewers += 1;
        }
        self.send_event(StreamType::ViewerJoin(ViewerJoin { stream }));
    }

    pub fn viewer_disconnect(&mut self, stream: String) {
        if let Some(meta) = self.streams.get_mut(&stream) {
            meta.viewers -= 1;
        }
        self.send_event(StreamType::ViewerLeave(ViewerLeave { stream }));
    }

    pub fn subscribe(&mut self) -> (Vec<StreamType>, impl Stream<Item=Option<StreamType>>) {
        use futures::StreamExt;

        let events = self.streams
            .iter()
            .map(|(name, meta)| StreamType::StreamExisting(StreamExisting { name: name.clone(), viewers: meta.viewers }))
            .collect::<Vec<_>>();

        let recv = self.send.subscribe();

        (events, BroadcastStream::new(recv).map(|r| r.ok()))
    }

    fn send_event(&self, event: StreamType) {
        debug!("Sending event: {:?}", event);
        let _ = self.send.send(event);
    }
}

#[derive(Clone)]
pub struct AppData {
    pub stream_repo: Arc<RwLock<StreamRepository>>,
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

    repo.write().unwrap().start_stream(app.clone(), queue);

    if let Err(e) = graph.run().await {
        error!("Error while reading from RTMP stream: {}", e);
    }

    info!("Stopping a stream at '{}'", app);

    repo.write().unwrap().stop_stream(app);

    Ok(())
}

async fn listen_rtmp(addr: SocketAddr, repo: Arc<RwLock<StreamRepository>>) -> anyhow::Result<()> {
    use sh_ingest_rtmp::RtmpListener;

    info!("Listening for RTMP at {}", addr);
    let listener = RtmpListener::bind(addr).await?;

    loop {
        let (req, app, key) = listener.accept().await?;

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

    ws.on_upgrade(move |socket| handle_websocket_video_response(socket, stream, data))
}

struct ViewGuard(String, Arc<AppData>);

impl ViewGuard {
    pub fn attach(stream: String, data: Arc<AppData>) -> Option<(MediaFrameQueueReceiver, Self)> {
        let mut repo = data.stream_repo.write().unwrap();

        let receiver = if let Some(meta) = repo.streams.get(&stream) {
            Some(meta.queue.get_receiver())
        } else {
            None
        };

        if let Some(receiver) = receiver {
            repo.viewer_join(stream.clone());
            drop(repo);

            let guard = ViewGuard(stream, data);

            Some((receiver, guard))
        } else {
            None
        }
    }
}

impl Drop for ViewGuard {
    fn drop(&mut self) {
        self.1.stream_repo.write().unwrap().viewer_disconnect(self.0.clone());
    }
}

async fn handle_websocket_video_response(socket: WebSocket, stream: String, data: Arc<AppData>) {
    if let Some((mut queue_receiver, _guard)) = ViewGuard::attach(stream.clone(), data) {
        debug!("Found a stream at {}", stream);

        if let Err(e) = sh_transport_mse::start_websocket_filters(socket, &mut queue_receiver).await
        {
            error!("{}", e);
        }
    } else {
        debug!("Did not find a stream at {}", stream);
    }
}

async fn start(rtmp_addr: SocketAddr, web_addr: SocketAddr, rpc_addr: SocketAddr) -> anyhow::Result<()> {
    let stream_repo = Arc::new(RwLock::new(StreamRepository::new()));

    let repo = stream_repo.clone();
    tokio::spawn(async move {
        if let Err(e) = listen_rtmp(rtmp_addr, repo).await {
            error!("Error while listening on RTMP: {}", e);
        }
    });

    let data = Arc::new(AppData { stream_repo });

    let app = Router::new()
        .route("/transport/mse/:stream", get(websocket_video))
        .layer(AddExtensionLayer::new(data.clone()));

    let ws_task = tokio::spawn(async move {
        debug!("Listening for WebSocket requests on {}", web_addr);
        hyper::Server::bind(&web_addr)
            .tcp_nodelay(true)
            .serve(app.into_make_service())
            .await
            .unwrap();

        debug!("Finished listening for WebSocket requests");
    });

    let service = StreamInfoService { data: data.clone() };

    let rpc_task = tokio::spawn(async move {
        debug!("Listening for RPC calls on {}", rpc_addr);

        tonic::transport::Server::builder()
            .add_service(StreamInfoServer::new(service))
            .serve(rpc_addr)
            .await
            .unwrap();

        debug!("Finished listening for RPC calls");
    });

    future::select(ws_task, rpc_task).await;

    debug!("Either WebSocket or RPC server finished");

    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::net::ToSocketAddrs;

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let _ = dotenv::dotenv();

    let rtmp_addr = std::env::var("INGEST_RTMP_ADDR")
        .unwrap_or_else(|_| "localhost:1935".into())
        .to_socket_addrs()?.next().expect("Failed to resolve INGEST_RTMP_ADDR");
    let web_addr = std::env::var("INGEST_WEB_ADDR")
        .unwrap_or_else(|_| "localhost:8080".into())
        .to_socket_addrs()?.next().expect("Failed to resolve INGEST_WEB_ADDR");
    let rpc_addr = std::env::var("INGEST_RPC_ADDR")
        .unwrap_or_else(|_| "localhost:8081".into())
        .to_socket_addrs()?.next().expect("Failed to resolve INGEST_RPC_ADDR");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { start(rtmp_addr, web_addr, rpc_addr).await })?;

    Ok(())
}
