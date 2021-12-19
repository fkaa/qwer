use axum::{
    body::{self, boxed, Empty, StreamBody},
    extract::{
        ws::{WebSocket, WebSocketUpgrade},
        Extension, Path,
    },
    response::IntoResponse,
    routing::get,
    AddExtensionLayer, Router,
};
use futures::{future, Stream};
use hyper::{Response, StatusCode};
use sh_fmp4::FragmentedMp4WriteFilter;
use tokio::{
    sync::broadcast::{self, Receiver, Sender},
    task,
};
use tokio_stream::wrappers::BroadcastStream;
use tonic::transport::{Channel, Endpoint};
use tracing::*;

use qw_proto::{
    stream_auth::{stream_auth_service_client::StreamAuthServiceClient, IngestRequest},
    stream_info::{
        stream_info_server::{StreamInfo, StreamInfoServer},
        stream_reply::{
            StreamExisting, StreamStarted, StreamStats, StreamStopped, StreamType, ViewerJoin,
            ViewerLeave,
        },
        StreamReply, StreamRequest,
    },
};
use sh_media::{
    wait_for_sync_frame, BitstreamFramerFilter, BitstreamFraming, ByteStreamWriteFilter,
    ByteWriteFilter2, FilterGraph, Frame, FrameAnalyzerFilter, FrameReadFilter, FrameWriteFilter,
    MediaFrameQueue, MediaFrameQueueReceiver,
};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    pin::Pin,
    sync::{Arc, RwLock},
};

use crate::{
    bandwidth_analyzer::BandwidthAnalyzerFilter, snapshot_provider::SnapshotProviderFilter,
};

mod bandwidth_analyzer;
mod snapshot_provider;

pub struct StreamMetadata {
    queue: MediaFrameQueue,
    viewers: u32,
    snapshot: Arc<RwLock<Option<Frame>>>,
}

impl StreamMetadata {
    pub fn new(queue: MediaFrameQueue, snapshot: Arc<RwLock<Option<Frame>>>) -> Self {
        StreamMetadata {
            queue,
            viewers: 0,
            snapshot,
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

    async fn listen(
        &self,
        _request: tonic::Request<StreamRequest>,
    ) -> Result<tonic::Response<Self::ListenStream>, tonic::Status> {
        use futures::StreamExt;

        debug!("listen RPC call");

        let (events, stream) = self
            .data
            .stream_repo
            .write()
            .unwrap()
            .subscribe(self.data.stream_stat_sender.subscribe());

        let event_stream = futures::stream::iter(events).map(|e| Some(e));

        let stream = event_stream.chain(stream).map(|msg| {
            msg.ok_or(tonic::Status::aborted("stream overflowed"))
                .map(|msg| StreamReply {
                    stream_type: Some(msg),
                })
        });

        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

pub struct StreamRepository {
    pub stream_mapping: HashMap<String, i32>,
    pub streams: HashMap<i32, StreamMetadata>,
    send: Sender<StreamType>,
    // channels: Vec<Sender<StreamEvent>>,
}

impl StreamRepository {
    pub fn new() -> Self {
        let (send, _) = broadcast::channel(512);

        StreamRepository {
            stream_mapping: HashMap::new(),
            streams: HashMap::new(),
            send,
        }
    }

    pub fn start_stream(
        &mut self,
        stream_session_id: i32,
        stream: String,
        queue: MediaFrameQueue,
        snapshot: Arc<RwLock<Option<Frame>>>,
    ) {
        let meta = StreamMetadata::new(queue, snapshot);
        self.streams.insert(stream_session_id, meta);
        self.stream_mapping.insert(stream, stream_session_id);
        self.send_event(StreamType::StreamStarted(StreamStarted {
            stream_session_id,
        }));
    }

    pub fn stop_stream(&mut self, stream_session_id: i32) {
        self.streams.remove(&stream_session_id);
        self.send_event(StreamType::StreamStopped(StreamStopped {
            stream_session_id,
        }));
    }

    pub fn viewer_join(&mut self, stream_session_id: i32) {
        if let Some(meta) = self.streams.get_mut(&stream_session_id) {
            meta.viewers += 1;
        }
        self.send_event(StreamType::ViewerJoin(ViewerJoin { stream_session_id }));
    }

    pub fn viewer_disconnect(&mut self, stream_session_id: i32) {
        if let Some(meta) = self.streams.get_mut(&stream_session_id) {
            meta.viewers -= 1;
        }
        self.send_event(StreamType::ViewerLeave(ViewerLeave { stream_session_id }));
    }

    pub fn subscribe(
        &mut self,
        stream_stats: Receiver<StreamStats>,
    ) -> (Vec<StreamType>, impl Stream<Item = Option<StreamType>>) {
        use futures::StreamExt;

        let events = self
            .streams
            .iter()
            .map(|(id, meta)| {
                StreamType::StreamExisting(StreamExisting {
                    stream_session_id: *id,
                    viewers: meta.viewers,
                })
            })
            .collect::<Vec<_>>();

        let recv = self.send.subscribe();

        let event_stream = BroadcastStream::new(recv).map(|r| r.ok());
        let stream_stats =
            BroadcastStream::new(stream_stats).map(|s| s.map(|s| StreamType::StreamStats(s)).ok());
        let merged_stream = tokio_stream::StreamExt::merge(event_stream, stream_stats);

        (events, merged_stream)
    }

    fn send_event(&self, event: StreamType) {
        debug!("Sending event: {:?}", event);
        let _ = self.send.send(event);
    }
}

#[derive(Clone)]
pub struct AppData {
    pub stream_repo: Arc<RwLock<StreamRepository>>,
    pub client: StreamAuthServiceClient<Channel>,
    pub stream_stat_sender: Sender<StreamStats>,
}

async fn rtmp_ingest(
    id: i32,
    app: String,
    request: sh_ingest_rtmp::RtmpRequest,
    sender: Sender<StreamStats>,
    repo: Arc<RwLock<StreamRepository>>,
) -> anyhow::Result<()> {
    use sh_ingest_rtmp::RtmpReadFilter;

    let session = request.authenticate().await?;
    let queue = MediaFrameQueue::new();
    let rtmp_filter = RtmpReadFilter::new(session);
    let rtmp_analyzer = FrameAnalyzerFilter::read(Box::new(rtmp_filter));
    let bw_analyzer = BandwidthAnalyzerFilter::new(Box::new(rtmp_analyzer), id, sender);

    let snapshot = Arc::new(RwLock::new(None));
    let snapshot_provider = SnapshotProviderFilter::new(Box::new(bw_analyzer), snapshot.clone());

    let mut graph = FilterGraph::new(Box::new(snapshot_provider), Box::new(queue.clone()));

    info!("Starting a stream at '{}'", app);

    repo.write()
        .unwrap()
        .start_stream(id, app.clone(), queue, snapshot);

    if let Err(e) = graph.run().await {
        error!("Error while reading from RTMP stream: {}", e);
    }

    info!("Stopping a stream at '{}'", app);

    repo.write().unwrap().stop_stream(id);

    Ok(())
}

async fn listen_rtmp(
    addr: SocketAddr,
    client: StreamAuthServiceClient<Channel>,
    data: Arc<AppData>,
) -> anyhow::Result<()> {
    use sh_ingest_rtmp::RtmpListener;

    info!("Listening for RTMP at {}", addr);
    let listener = RtmpListener::bind(addr).await?;

    loop {
        match listener.accept().await {
            Ok((req, app, key)) => {
                info!("Got a RTMP session from {} on stream '{}'", req.addr(), app);

                let repo = data.stream_repo.clone();
                let sender = data.stream_stat_sender.clone();
                let mut client = client.clone();

                tokio::spawn(async move {
                    match authenticate_rtmp_stream(&mut client, &app, &key).await {
                        Ok(id) => {
                            if let Err(e) = rtmp_ingest(id, app, req, sender, repo).await {
                                error!("Error while ingesting RTMP: {}", e);
                            }
                        }
                        Err(e) => {
                            error!("Failed to authenticate stream ingest: {}", e);
                        }
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept RTMP connection: {}", e);
            }
        }
    }
}

async fn authenticate_rtmp_stream(
    client: &mut StreamAuthServiceClient<Channel>,
    app_name: &str,
    supplied_stream_key: &str,
) -> anyhow::Result<i32> {
    let request = IngestRequest {
        name: app_name.into(),
        stream_key: supplied_stream_key.into(),
    };
    let response = client.request_stream_ingest(request).await?;

    Ok(response.into_inner().stream_session_id)
}

pub async fn http_video(
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received HTTP request for '{}'", stream);

    if let Some((queue_receiver, guard)) = ViewGuard::attach(stream.clone(), &data) {
        debug!("Found a stream at {}", stream);

        let sender = data.stream_stat_sender.clone();
        let bw_analyzer = Box::new(BandwidthAnalyzerFilter::new(
            Box::new(queue_receiver),
            guard.0,
            sender,
        ));
        let (output_filter, bytes_rx) = ByteStreamWriteFilter::new();
        let output_filter = Box::new(output_filter);

        task::spawn(async move {
            if let Err(e) = stream_http_video(bw_analyzer, output_filter, guard).await {
                error!("{}", e);
            }
        });

        boxed(StreamBody::new(bytes_rx))
    } else {
        debug!("Did not find a stream at {}", stream);

        boxed(
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Empty::new())
                .unwrap(),
        )
    }
}

async fn stream_http_video(
    mut read: Box<dyn FrameReadFilter + Unpin + Send>,
    output: Box<dyn ByteWriteFilter2 + Unpin + Send>,
    _guard: ViewGuard,
) -> anyhow::Result<()> {
    // write
    let fmp4_filter = Box::new(FragmentedMp4WriteFilter::new(output));
    let write_analyzer = Box::new(FrameAnalyzerFilter::write(fmp4_filter));
    let mut write = Box::new(BitstreamFramerFilter::new(
        BitstreamFraming::FourByteLength,
        write_analyzer,
    ));

    let streams = read.start().await?;
    let first_frame = wait_for_sync_frame(&mut *read).await?;

    write.start(streams).await?;
    write.write(first_frame).await?;

    loop {
        let frame = read.read().await?;
        write.write(frame).await?;
    }
}

pub async fn websocket_video(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| handle_websocket_video_response(socket, stream, data))
}

struct ViewGuard(i32, Arc<AppData>);

impl ViewGuard {
    pub fn attach(stream: String, data: &Arc<AppData>) -> Option<(MediaFrameQueueReceiver, Self)> {
        let mut repo = data.stream_repo.write().unwrap();

        let stream_id = *repo.stream_mapping.get(&stream)?;

        let receiver = repo
            .streams
            .get(&stream_id)
            .map(|s| s.queue.get_receiver())?;

        repo.viewer_join(stream_id);
        drop(repo);

        let guard = ViewGuard(stream_id, data.clone());

        Some((receiver, guard))
    }
}

impl Drop for ViewGuard {
    fn drop(&mut self) {
        self.1
            .stream_repo
            .write()
            .unwrap()
            .viewer_disconnect(self.0.clone());
    }
}

async fn handle_websocket_video_response(socket: WebSocket, stream: String, data: Arc<AppData>) {
    if let Some((queue_receiver, guard)) = ViewGuard::attach(stream.clone(), &data) {
        debug!("Found a stream at {}", stream);

        let sender = data.stream_stat_sender.clone();
        let mut bw_analyzer =
            BandwidthAnalyzerFilter::new(Box::new(queue_receiver), guard.0, sender);

        if let Err(e) = sh_transport_mse::start_websocket_filters(socket, &mut bw_analyzer).await {
            error!("{}", e);
        }
    } else {
        debug!("Did not find a stream at {}", stream);
    }
}

pub async fn snapshot(
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received snapshot request for '{}'", stream);

    let repo = data.stream_repo.write().unwrap();

    let stream_id = repo.stream_mapping.get(&stream);
    let meta = stream_id.and_then(|id| repo.streams.get(id));

    if let Some(frame) = meta.and_then(|m| m.snapshot.read().unwrap().clone()) {
        match sh_fmp4::single_frame_fmp4(frame) {
            Ok(bytes) => Response::builder()
                .header("Content-Type", "video/mp4")
                .header("Access-Control-Allow-Origin", "*")
                .status(StatusCode::OK)
                .body(body::Full::from(bytes))
                .unwrap(),
            Err(e) => Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(body::Full::from(format!("Failed to get snapshot: {}", e)))
                .unwrap(),
        }
    } else {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(body::Full::from("Failed to find snapshot"))
            .unwrap()
    }
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

async fn start() -> anyhow::Result<()> {
    let ingest_rtmp_addr = resolve_env_addr("INGEST_RTMP_ADDR", "localhost:1935");
    let ingest_web_addr = resolve_env_addr("INGEST_WEB_ADDR", "localhost:8080");
    let ingest_rpc_addr = resolve_env_addr("INGEST_RPC_ADDR", "localhost:8081");

    let scuffed_rpc_addr = env("SCUFFED_RPC_ADDR", "localhost:9082");

    let stream_repo = Arc::new(RwLock::new(StreamRepository::new()));

    let client_endpoint = Endpoint::from_shared(scuffed_rpc_addr)
        .unwrap()
        .connect_lazy();
    let client = StreamAuthServiceClient::new(client_endpoint);

    let (stream_stat_sender, _) = broadcast::channel(512);
    let data = Arc::new(AppData {
        stream_repo,
        client: client.clone(),
        stream_stat_sender,
    });

    {
        let data = data.clone();
        tokio::spawn(async move {
            if let Err(e) = listen_rtmp(ingest_rtmp_addr, client, data).await {
                error!("Error while listening on RTMP: {}", e);
            }
        });
    }

    let app = Router::new()
        .route("/transport/mse/:stream", get(websocket_video))
        .route("/transport/http/:stream", get(http_video))
        .route("/snapshot/:stream", get(snapshot))
        .layer(AddExtensionLayer::new(data.clone()));

    let ws_task = tokio::spawn(async move {
        debug!("Listening for WebSocket requests on {}", ingest_web_addr);
        hyper::Server::bind(&ingest_web_addr)
            .tcp_nodelay(true)
            .serve(app.into_make_service())
            .await
            .unwrap();

        debug!("Finished listening for WebSocket requests");
    });

    let service = StreamInfoService { data: data.clone() };

    let rpc_task = tokio::spawn(async move {
        debug!("Listening for RPC calls on {}", ingest_rpc_addr);

        tonic::transport::Server::builder()
            .add_service(StreamInfoServer::new(service))
            .serve(ingest_rpc_addr)
            .await
            .unwrap();

        debug!("Finished listening for RPC calls");
    });

    future::select(ws_task, rpc_task).await;

    debug!("Either WebSocket or RPC server finished");

    Ok(())
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
