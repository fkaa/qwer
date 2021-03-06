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
use sh_ingest_rtmp::RtmpRequest;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::broadcast::{self, Receiver, Sender},
    task,
    time::timeout,
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
        StreamMetadata, StreamReply, StreamRequest,
    },
};
use sh_media::{
    wait_for_sync_frame, BitstreamFramerFilter, BitstreamFraming, ByteStreamWriteFilter,
    ByteWriteFilter2, Frame, FrameAnalyzerFilter, FrameReadFilter, FrameWriteFilter,
    MediaFrameQueue, MediaFrameQueueReceiver,
};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use std::{
    collections::HashMap,
    net::{SocketAddr, ToSocketAddrs},
    pin::Pin,
    sync::{Arc, RwLock},
    time::Duration,
};

use crate::{
    bandwidth_analyzer::BandwidthAnalyzerFilter, snapshot_provider::SnapshotProviderFilter,
};

mod bandwidth_analyzer;
mod snapshot_provider;

pub struct StreamState {
    queue: MediaFrameQueue,
    viewers: u32,
    snapshot: Arc<RwLock<Option<Frame>>>,
    meta: StreamMetadata,
}

impl StreamState {
    pub fn new(
        queue: MediaFrameQueue,
        snapshot: Arc<RwLock<Option<Frame>>>,
        meta: StreamMetadata,
    ) -> Self {
        StreamState {
            queue,
            viewers: 0,
            snapshot,
            meta,
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

        let event_stream = futures::stream::iter(events).map(Some);

        let stream = event_stream.chain(stream).map(|msg| {
            msg.ok_or_else(|| tonic::Status::aborted("stream overflowed"))
                .map(|msg| StreamReply {
                    stream_type: Some(msg),
                })
        });

        Ok(tonic::Response::new(Box::pin(stream)))
    }
}

pub struct StreamRepository {
    pub stream_mapping: HashMap<String, i32>,
    pub streams: HashMap<i32, StreamState>,
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
        info: StreamMetadata,
    ) {
        debug!("Starting stream with id {stream_session_id}");
        let meta = StreamState::new(queue, snapshot, info.clone());
        self.streams.insert(stream_session_id, meta);
        self.stream_mapping.insert(stream, stream_session_id);
        self.send_event(StreamType::StreamStarted(StreamStarted {
            stream_session_id,
            meta: Some(info),
        }));
    }

    pub fn stop_stream(&mut self, stream_session_id: i32) {
        debug!("Stopping stream with id {stream_session_id}");
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
            .map(|(id, state)| {
                StreamType::StreamExisting(StreamExisting {
                    stream_session_id: *id,
                    viewers: state.viewers,
                    meta: Some(state.meta.clone()),
                })
            })
            .collect::<Vec<_>>();

        let recv = self.send.subscribe();

        let event_stream = BroadcastStream::new(recv).map(|r| r.ok());
        let stream_stats =
            BroadcastStream::new(stream_stats).map(|s| s.map(StreamType::StreamStats).ok());
        let merged_stream = tokio_stream::StreamExt::merge(event_stream, stream_stats);

        (events, merged_stream)
    }

    fn send_event(&self, event: StreamType) {
        debug!("Sending event: {:?}", event);
        let _ = self.send.send(event);
    }
}

impl Default for StreamRepository {
    fn default() -> Self {
        Self::new()
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
    name: String,
    request: sh_ingest_rtmp::RtmpRequest,
    sender: Sender<StreamStats>,
    repo: Arc<RwLock<StreamRepository>>,
) -> anyhow::Result<()> {
    use sh_ingest_rtmp::RtmpReadFilter;

    let session = timeout(Duration::from_secs(5), request.authenticate()).await??;
    let rtmp_meta = session.stream_metadata().clone();

    let mut queue = MediaFrameQueue::new();
    let rtmp_filter = RtmpReadFilter::new(session);
    let rtmp_analyzer = FrameAnalyzerFilter::read(Box::new(rtmp_filter));
    let bw_analyzer = BandwidthAnalyzerFilter::new(Box::new(rtmp_analyzer), id, true, sender);

    let snapshot = Arc::new(RwLock::new(None));
    let mut snapshot_provider =
        SnapshotProviderFilter::new(Box::new(bw_analyzer), snapshot.clone());

    // let mut graph = FilterGraph::new(Box::new(snapshot_provider), Box::new(queue.clone()));

    let streams = snapshot_provider.start().await?;

    let parameter_sets = streams.iter().find_map(|s| s.parameter_sets());

    queue.start(streams).await?;

    let meta = StreamMetadata {
        video_encoder: rtmp_meta.encoder,
        video_bitrate_kbps: rtmp_meta.video_bitrate_kbps,
        parameter_sets,
    };

    info!("Starting a stream for {} with id {}", name, id);

    repo.write()
        .unwrap()
        .start_stream(id, name.clone(), queue.clone(), snapshot, meta);

    async fn stream(
        mut queue: MediaFrameQueue,
        mut snapshot_provider: SnapshotProviderFilter,
    ) -> anyhow::Result<()> {
        loop {
            let frame = snapshot_provider.read().await?;
            queue.write(frame).await?;
        }
    }

    if let Err(e) = stream(queue, snapshot_provider).await {
        error!("Error while ingesting: {:?}", e);
    }

    info!("Stopping a stream at '{}'", name);

    repo.write().unwrap().stop_stream(id);

    Ok(())
}

async fn process_rtmp_ingest(
    socket: TcpStream,
    addr: SocketAddr,
    client: StreamAuthServiceClient<Channel>,
    data: Arc<AppData>,
) -> anyhow::Result<()> {
    let (req, app, key) = timeout(
        Duration::from_secs(5),
        RtmpRequest::from_socket(socket, addr),
    )
    .await??;

    info!("Got a RTMP session from {} with app {}", req.addr(), app);

    let repo = data.stream_repo.clone();
    let sender = data.stream_stat_sender.clone();
    let mut client = client.clone();
    let is_public = app == "public";

    let (id, name) = authenticate_rtmp_stream(&mut client, &key, is_public).await?;

    rtmp_ingest(id, name, req, sender, repo).await?;

    Ok(())
}

async fn listen_rtmp(
    addr: SocketAddr,
    client: StreamAuthServiceClient<Channel>,
    data: Arc<AppData>,
) -> anyhow::Result<()> {
    info!("Listening for RTMP at {}", addr);

    let listener = TcpListener::bind(addr).await?;

    loop {
        match listener.accept().await {
            Ok((socket, addr)) => {
                info!("Got a TCP connection from {}", addr);

                let client = client.clone();
                let data = data.clone();
                tokio::spawn(async move {
                    if let Err(e) = process_rtmp_ingest(socket, addr, client, data).await {
                        error!("Failed to process RTMP ingest: {:?}", e);
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept TCP connection: {:?}", e);
            }
        }
    }
}

async fn authenticate_rtmp_stream(
    client: &mut StreamAuthServiceClient<Channel>,
    supplied_stream_key: &str,
    is_public_stream: bool,
) -> anyhow::Result<(i32, String)> {
    let request = IngestRequest {
        stream_key: supplied_stream_key.into(),
        is_unlisted: !is_public_stream,
    };
    let response = client.request_stream_ingest(request).await?.into_inner();
    Ok((response.stream_session_id, response.streamer_name))
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
            false,
            sender,
        ));
        let (output_filter, bytes_rx) = ByteStreamWriteFilter::new();
        let output_filter = Box::new(output_filter);

        task::spawn(async move {
            if let Err(e) = stream_http_video(bw_analyzer, output_filter, guard).await {
                error!("Failed to stream video: {:?}", e);
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
            .viewer_disconnect(self.0);
    }
}

async fn handle_websocket_video_response(socket: WebSocket, stream: String, data: Arc<AppData>) {
    if let Some((queue_receiver, guard)) = ViewGuard::attach(stream.clone(), &data) {
        debug!("Found a stream at {}", stream);

        let sender = data.stream_stat_sender.clone();
        let mut bw_analyzer =
            BandwidthAnalyzerFilter::new(Box::new(queue_receiver), guard.0, false, sender);

        if let Err(e) = sh_transport_mse::start_websocket_filters(socket, &mut bw_analyzer).await {
            error!("Failed to run WebSocket filters: {:?}", e);
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
                .body(body::Full::from(format!("Failed to get snapshot: {:?}", e)))
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
        .unwrap_or_else(|| panic!("Failed to resolve {}", var))
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
                error!("Error while listening on RTMP: {:?}", e);
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
