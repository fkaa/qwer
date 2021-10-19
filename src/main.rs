use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, RwLock};

use async_channel::Receiver;
use rml_rtmp::{
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult},
};
use stop_token::{StopSource, StopToken};
use tokio::net::{tcp, TcpListener, TcpStream};
use futures::{StreamExt, SinkExt, stream::SplitSink};

mod media;
mod mp4;
mod mp42;
// mod srt;
// mod mpegts;
mod rtmp;
mod logger;

use logger::*;
use slog::{error, warn, debug, info};

use bytes::Bytes;
use bytes::{BufMut, BytesMut};
use media::*;
use mp4::{FragmentedMp4WriteFilter, Mp4Metadata, Mp4Segment};
use rtmp::RtmpReadFilter;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension,
        Path,
    },
    http::StatusCode,
    Router,
    response::{IntoResponse, Html},
    handler::get,
    AddExtensionLayer,
};

/*use actix::{Actor, Addr, AsyncContext, Context, Handler, Message, StreamHandler};
use actix_web::{
    dev::Body, get, http::StatusCode, web, App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_actors::ws;*/


#[derive(Default)]
struct StreamRepository {
    streams: HashMap<String, MediaFrameQueue>,
}

/*impl Handler<FindStream> for StreamRepository {
    type Result = Option<MediaFrameQueueReceiver>;

    fn handle(&mut self, msg: FindStream, _ctx: &mut Self::Context) -> Self::Result {
        self.streams.get(&msg.0).map(|queue| queue.get_receiver())
    }
}*/

enum WebSocketMessage {
    Codec(u8, u8, u8),
    Segment(bytes::Bytes, Mp4Segment),
    Init(bytes::Bytes),
}

struct StartWebSocketMedia;

/*impl Handler<WebSocketMessage> for WebSocketMediaStream {
    type Result = ();

    fn handle(&mut self, msg: WebSocketMessage, ctx: &mut Self::Context) -> Self::Result {
        match msg {
            WebSocketMessage::Codec(profile, constraints, level) => {
                debug!(self.logger, "Sending codec message");

                let mut framed = BytesMut::with_capacity(4);
                framed.put_u8(0);
                framed.put_u8(profile);
                framed.put_u8(constraints);
                framed.put_u8(level);

                ctx.binary(framed);
            }
            WebSocketMessage::Init(bytes) => {
                debug!(self.logger, "Sending init message");

                let mut framed = BytesMut::with_capacity(bytes.len() + 1);
                framed.put_u8(1);
                framed.extend_from_slice(&bytes);

                ctx.binary(framed);
            }
            WebSocketMessage::Segment(bytes, _segment) => {
                let mut framed = BytesMut::with_capacity(bytes.len() + 1);
                framed.put_u8(1);
                framed.extend_from_slice(&bytes);

                ctx.binary(framed);
            }
        }
    }
}*/

/*impl StreamHandler<WebSocketMessage> for WebSocketMediaStream {
    fn handle(&mut self, msg: WebSocketMessage, ctx: &mut Self::Context) {
        match msg {
            WebSocketMessage::Codec(codec) => {

            }
            WebSocketMessage::Segment(bytes, segment) => {

                ctx.binary(bytes);
            }
        }
    }
}*/

fn spawn_graph(logger: ContextLogger, mut graph: FilterGraph, token: &StopToken) {
    let fut = async move {
        debug!(logger, "Starting graph");

        if let Err(e) = graph.run().await {
            error!(logger, "{:?}", e);
        }
    };

    tokio::spawn(token.stop_future(fut));
}

fn get_codec_from_stream(stream: &Stream) -> anyhow::Result<(u8, u8, u8)> {
    if let Some(VideoCodecSpecificInfo::H264 {
        profile_indication,
        profile_compatibility,
        level_indication,
        ..
    }) = stream.codec.video().map(|v| &v.extra)
    {
        Ok((
            *profile_indication,
            *profile_compatibility,
            *level_indication,
        ))
    } else {
        todo!()
    }
}

/*fn spawn_websocket_feeder(
    logger: ContextLogger,
    receiver: Receiver<anyhow::Result<StreamMessage<Mp4Metadata>>>,
    addr: Addr<WebSocketMediaStream>,
    token: &StopToken,
) {
    let fut = async move {
        debug!(logger, "Starting websocket feeder");

        while let Ok(Ok(message)) = receiver.recv().await {
            match message {
                StreamMessage::Start(stream) => {
                    let (profile, constraint, level) = get_codec_from_stream(&stream).unwrap();
                    addr.send(WebSocketMessage::Codec(profile, constraint, level))
                        .await
                        .unwrap();
                }
                StreamMessage::Frame(bytes, Mp4Metadata::Segment(segment)) => {
                    addr.send(WebSocketMessage::Segment(bytes, segment))
                        .await
                        .unwrap();
                }
                StreamMessage::Frame(bytes, Mp4Metadata::Init) => {
                    addr.send(WebSocketMessage::Init(bytes)).await.unwrap();
                }
            }
        }
    };

    tokio::spawn(token.stop_future(fut));
}

struct WebSocketMediaStream {
    logger: ContextLogger,
    address: String,
    queue: Option<MediaFrameQueueReceiver>,
    stop_source: StopSource,
    paused: bool,
    sent_keyframe: bool,
}

impl WebSocketMediaStream {
    fn new(logger: ContextLogger, address: String, queue: MediaFrameQueueReceiver) -> Self {
        let stop_source = StopSource::new();

        WebSocketMediaStream {
            logger,
            address,
            queue: Some(queue),
            stop_source,
            paused: true,
            sent_keyframe: false,
        }
    }

    fn start_stream(&mut self, addr: Addr<WebSocketMediaStream>) {
        let stop_token = self.stop_source.stop_token();

        let graph_logger = self.logger.scope();

        // read
        let queue_reader = self.queue.take().unwrap();

        // write
        let (output_filter, receiver) = StreamWriteFilter::new();
        let fmp4_filter = Box::new(FragmentedMp4WriteFilter::new(Box::new(output_filter)));
        let write_analyzer = Box::new(FrameAnalyzerFilter::write(graph_logger.clone(), fmp4_filter));
        let write_filter = WaitForSyncFrameFilter::new(
            graph_logger.clone(),
            write_analyzer,
        );

        let graph = FilterGraph::new(Box::new(queue_reader), Box::new(write_filter));

        spawn_graph(graph_logger, graph, &stop_token);
        spawn_websocket_feeder(self.logger.scope(), receiver, addr, &stop_token);

        self.paused = false;
    }
}

impl Actor for WebSocketMediaStream {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocketMediaStream {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Text(txt)) if txt == "start" => {
                self.start_stream(ctx.address());
            }
            Ok(ws::Message::Close(_)) => {
                ctx.close(None);
            }
            _ => {
                println!("{:?}", msg);
            }
        }
    }
}*/


/*#[get("/http/{filename}")]
async fn stream_video(
    req: HttpRequest,
    _stream: web::Payload,
    data: web::Data<AppData>,
) -> HttpResponse {
    let path: std::path::PathBuf = req.match_info().query("filename").parse().unwrap();

    let f = path.file_name().unwrap().to_str().unwrap().to_string();

    let queue_receiver = data.stream_repo.send(FindStream(f.clone())).await;

    if let Ok(Some(queue_receiver)) = queue_receiver {
        let graph_logger = data.logger.scope();
        // write
        let (output_filter, receiver) = ByteStreamWriteFilter::new();
        let write_filter = WaitForSyncFrameFilter::new(
            graph_logger.clone(),
            Box::new(mp42::FragmentedMp4WriteFilter::new(Box::new(output_filter)),
        ));

        //let file = tokio::fs::File::create("test.fmp4").await.unwrap();
        //let write_filter = WaitForSyncFrameFilter::new(Box::new(mp42::FragmentedMp4WriteFilter::new(Box::new(FileWriteFilter::new(file)))));
        //let mut graph = FilterGraph::new(Box::new(queue_receiver), Box::new(write_filter));
        let mut graph = FilterGraph::new(Box::new(queue_receiver), Box::new(write_filter));

        tokio::spawn(async move {
            debug!(graph_logger, "Starting graph");
            graph.run().await.unwrap();
        });

        use futures::StreamExt;
        HttpResponse::Ok().streaming(receiver.map(|x| x.map_err(|e| HttpStreamingError(e))))
    } else {
        HttpResponse::NotFound().body("nope!")
    }
}*/

/*#[get("/ws/{filename}")]
async fn ws_stream_video(
    req: HttpRequest,
    stream: web::Payload,
    data: web::Data<AppData>,
) -> HttpResponse {
    let logger = data.logger.scope();

    let path: std::path::PathBuf = req.match_info().query("filename").parse().unwrap();

    let f = path.file_name().unwrap().to_str().unwrap().to_string();

    let queue_receiver = data.stream_repo.send(FindStream(f.clone())).await;

    debug!(logger, "Received websocket request for {} from {:?}", f, req.peer_addr());

    if let Ok(Some(queue_receiver)) = queue_receiver {
        debug!(data.logger, "Found a stream at {}", f);

        let ws_stream = WebSocketMediaStream::new(logger.scope(), f, queue_receiver);
        let response = ws::start(ws_stream, &req, stream).unwrap();

        response
    } else {
        debug!(logger, "Did not find a stream at {}", f);

        HttpResponse::NotFound().body("nope!")
    }
}*/

/*#[derive(Debug, thiserror::Error)]
struct HttpStreamingError(anyhow::Error);

impl fmt::Display for HttpStreamingError {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(formatter, "{}", self.0)
    }
}

impl actix_web::error::ResponseError for HttpStreamingError {
    fn status_code(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn error_response(&self) -> HttpResponse<Body> {
        HttpResponse::InternalServerError().body(Body::None)
    }
}*/

async fn authenticate_rtmp_stream(_app_name: &str, supplied_stream_key: &str) -> bool {
    std::env::var("STREAM_KEY").map(|key| key == supplied_stream_key).unwrap_or(true)
}

async fn do_rtmp_handshake(
    logger: &ContextLogger,
    read: &mut TcpReadFilter,
    write: &mut TcpWriteFilter,
) -> anyhow::Result<ServerSession> {
    use failure::Fail;

    let mut handshake = Handshake::new(PeerType::Server);

    let (response, remaining) = loop {
        let bytes = read.read().await?;
        let response = match handshake.process_bytes(&bytes) {
            Ok(HandshakeProcessResult::InProgress { response_bytes }) => response_bytes,
            Ok(HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            }) => break (response_bytes, remaining_bytes),
            Err(e) => return Err(e.kind.compat().into()),
        };

        write.write(response.into()).await?;
    };

    write.write(response.into()).await?;

    let config = ServerSessionConfig::new();
    let (mut session, initial_results) = ServerSession::new(config).map_err(|e| e.kind.compat())?;

    let results = session
        .handle_input(&remaining)
        .map_err(|e| e.kind.compat())?;

    /*for result in  {
        match result {
            ServerSessionResult::OutboundResponse(packet) => write.write(packet.bytes.into()).await?,
            ServerSessionResult::RaisedEvent(evt) => {
                dbg!(evt);
            }
            ServerSessionResult::UnhandleableMessageReceived(payload) => {},

        }
    }*/

    let mut authenticated = false;
    let mut r = VecDeque::new();
    let mut application_name = None;

    r.extend(results.into_iter().chain(initial_results.into_iter()));

    loop {
        while let Some(res) = r.pop_front() {
            match res {
                ServerSessionResult::OutboundResponse(packet) => {
                    write.write(packet.bytes.into()).await?
                }
                ServerSessionResult::RaisedEvent(evt) => {
                    // dbg!(&evt);

                    match evt {
                        ServerSessionEvent::ConnectionRequested {
                            request_id,
                            app_name,
                        } => {
                            r.extend(
                                session
                                    .accept_request(request_id)
                                    .map_err(|e| e.kind.compat())?,
                            );
                            /*for result in  {
                                if let ServerSessionResult::OutboundResponse(packet) = result {
                                    write.write(packet.bytes.into()).await?;
                                }
                            }*/

                            debug!(logger, "Accepted connection request");

                            application_name = Some(app_name);
                        }
                        ServerSessionEvent::PublishStreamRequested {
                            request_id,
                            app_name,
                            stream_key,
                            mode: _,
                        } => {
                            if authenticate_rtmp_stream(&app_name, &stream_key).await {
                                r.extend(
                                    session
                                        .accept_request(request_id)
                                        .map_err(|e| e.kind.compat())?,
                                );
                                /*for result in session.accept_request(request_id).map_err(|e| e.kind.compat())? {
                                    if let ServerSessionResult::OutboundResponse(packet) = result {
                                        write.write(packet.bytes.into()).await?;
                                    }
                                }*/

                                authenticated = true;

                                debug!(logger, "Accepted publish stream request");
                            }
                        }
                        _ => {}
                    }
                }
                ServerSessionResult::UnhandleableMessageReceived(_payload) => {}
            }
        }

        if authenticated {
            return Ok(session);
        }

        // debug!("reading from endpoint!");
        let bytes = read.read().await?;
        let results = session.handle_input(&bytes).map_err(|e| e.kind.compat())?;
        /*debug!(
            "got {} results from endpoint, {} total",
            results.len(),
            r.len()
        );*/
        r.extend(results);
    }
}

async fn handle_tcp_socket(
    logger: &ContextLogger,
    socket: TcpStream,
    stream_repo: Arc<RwLock<StreamRepository>>,
) -> anyhow::Result<()> {
    let sid = String::from("test");

    let (mut read_filter, mut write_filter) = create_tcp_filters(socket, 188 * 8);

    let server_session = do_rtmp_handshake(&logger, &mut read_filter, &mut write_filter).await?;

    let filter_logger = logger.scope();

    let queue = MediaFrameQueue::new(filter_logger.clone());
    let rtmp_filter = RtmpReadFilter::new(filter_logger.clone(), read_filter, write_filter, server_session);
    let rtmp_analyzer = FrameAnalyzerFilter::read(filter_logger.clone(), Box::new(rtmp_filter));

    //let file = tokio::fs::File::create("test.fmp4").await.unwrap();
    //let mp4_writer = mp42::FragmentedMp4WriteFilter::new(Box::new(FileWriteFilter::new(file)));
    //let mut graph = FilterGraph::new(Box::new(rtmp_filter), Box::new(mp4_writer));
    let mut graph = FilterGraph::new(Box::new(rtmp_analyzer), Box::new(queue.clone()));

    stream_repo.write().unwrap().streams.insert(sid.clone(), queue);

    let result = graph.run().await;

    stream_repo.write().unwrap().streams.remove(&sid);

    result
}

fn spawn_listen_for_tcp(
    logger: ContextLogger,
    stream_repo: Arc<RwLock<StreamRepository>>,
    rtmp_addr: String)
{
    let fut = async move {
        info!(
            logger,
            "Listening for RTMP connections at {}",
            rtmp_addr
        );

        let listener = TcpListener::bind(rtmp_addr).await.unwrap();

        loop {
            let (socket, addr) = listener.accept().await.unwrap();
            info!(logger, "RTMP connection from {:?}", addr);

            let stream_repo = stream_repo.clone();
            let logger = logger.scope();
            tokio::spawn(async move {
                info!(logger, "Establishing RTMP connection from {:?}", socket.peer_addr());

                match handle_tcp_socket(&logger, socket, stream_repo).await {
                    Ok(_) => info!(logger, "Finished streaming from RTMP"),
                    Err(e) => warn!(logger, "Error while streaming from RTMP: {:?}", e),
                }
            });
        }
    };

    tokio::spawn(fut);
}

fn create_tcp_filters(
    socket: TcpStream,
    buffer: usize) -> (TcpReadFilter, TcpWriteFilter)
{
    let (read, write) = socket.into_split();

    (TcpReadFilter::new(read, buffer), TcpWriteFilter::new(write))
}

pub struct TcpWriteFilter {
    write: tcp::OwnedWriteHalf,
}

impl TcpWriteFilter {
    pub fn new(write: tcp::OwnedWriteHalf) -> Self {
        Self { write }
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for TcpWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        let _ = self.write.write(&bytes).await?;

        Ok(())
    }
}

pub struct TcpReadFilter {
    socket: tcp::OwnedReadHalf,
    size: usize,
    buf: Vec<u8>,
}

impl TcpReadFilter {
    pub fn new(socket: tcp::OwnedReadHalf, size: usize) -> Self {
        Self {
            socket,
            size,
            buf: vec![0; size],
        }
    }
}

#[async_trait::async_trait]
impl ByteReadFilter for TcpReadFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn read(&mut self) -> anyhow::Result<Bytes> {
        use tokio::io::AsyncReadExt;

        loop {
            match self.socket.read(&mut self.buf).await {
                Ok(n) => {
                    if n == 0 {
                        return Err(anyhow::anyhow!("EOS!"));
                    }

                    return Ok(Bytes::copy_from_slice(&mut self.buf[..n]));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }
}

#[derive(Clone)]
struct AppData {
    stream_repo: Arc<RwLock<StreamRepository>>,
    logger: ContextLogger,
}

struct WebSocketWriteFilter {
    sink: SplitSink<WebSocket, Message>,
}

impl WebSocketWriteFilter {
    pub fn new(sink: SplitSink<WebSocket, Message>) -> Self {
        Self { sink }
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for WebSocketWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()> {
        self.sink.send(Message::Binary(bytes.to_vec())).await?;

        Ok(())
    }
}

async fn websocket_video(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    let logger = data.logger.scope();

    debug!(logger, "Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| {
        handle_websocket_video_response(logger, socket, stream, data.clone())
    })
}

async fn handle_websocket_video_response(
    logger: ContextLogger,
    mut socket: WebSocket,
    stream: String,
    data: Arc<AppData>)
{
    let queue_receiver = data.stream_repo.read().unwrap().streams.get(&stream).map(|s| s.get_receiver());


    if let Some(mut queue_receiver) = queue_receiver {
        debug!(logger, "Found a stream at {}", stream);


        if let Err(e) = start_websocket_filters(&logger, socket, &mut queue_receiver).await
        {
            error!(logger, "{}", e);
        }

        // let graph = FilterGraph::new(Box::new(queue_reader), Box::new(write_filter));


        //let ws_stream = WebSocketMediaStream::new(logger.scope(), f, queue_receiver);
        //let response = ws::start(ws_stream, &req, stream).unwrap();

        //response

    } else {
        debug!(logger, "Did not find a stream at {}", stream);

    }
}

async fn start_websocket_filters(
    logger: &ContextLogger,
    mut socket: WebSocket,
    read: &mut (dyn FrameReadFilter + Unpin + Send)) -> anyhow::Result<()>
{
    let stream = read.start().await?;
    let (profile, constraints, level) = get_codec_from_stream(&stream)?;

    let mut framed = BytesMut::with_capacity(4);
    framed.put_u8(profile);
    framed.put_u8(constraints);
    framed.put_u8(level);

    let (mut sender, mut receiver) = socket.split();
    sender.send(Message::Binary(framed.to_vec())).await?;

    // write
    let output_filter = WebSocketWriteFilter::new(sender);
    let fmp4_filter = Box::new(mp42::FragmentedMp4WriteFilter::new(Box::new(output_filter)));
    let write_analyzer = Box::new(FrameAnalyzerFilter::write(logger.clone(), fmp4_filter));
    let mut write_filter = WaitForSyncFrameFilter::new(
        logger.clone(),
        write_analyzer,
    );

    write_filter.start(stream).await?;

    loop {
        let frame = read.read().await?;
        write_filter.write(frame).await?;
    }

    Ok(())
}

async fn handler() -> Html<&'static str> {
    Html("<h1>Hello, World!</h1>")
}

async fn start(logger: ContextLogger, web_addr: &str, rtmp_addr: String) -> anyhow::Result<()> {
    let stream_repo = Arc::new(RwLock::new(StreamRepository::default()));

    spawn_listen_for_tcp(logger.scope(), stream_repo.clone(), rtmp_addr);

    let web_logger = logger.scope();
    info!(web_logger, "Starting webserver at {}", web_addr);
    let data = AppData { stream_repo, logger: web_logger, };

    let app = Router::new()
        .route("/", get(handler))
        .route("/ws/:stream", get(websocket_video))
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
