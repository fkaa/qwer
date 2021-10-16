use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io;
use std::io::Error;

use async_channel::Receiver;
use rml_rtmp::{
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult},
};
use stop_token::{StopSource, StopToken};
use tokio::net::{tcp, TcpListener, TcpStream};

mod media;
mod mp4;
mod mp42;
// mod srt;
// mod mpegts;
mod rtmp;

use bytes::Bytes;
use bytes::{BufMut, BytesMut};
use log::*;
use media::*;
use mp4::{FragmentedMp4WriteFilter, Mp4Metadata, Mp4Segment};
use rtmp::RtmpReadFilter;

use actix::{Actor, Addr, AsyncContext, Context, Handler, Message, StreamHandler};
use actix_web::{
    dev::Body, get, http::StatusCode, web, App, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_actors::ws;

struct FindStream(String);

impl Message for FindStream {
    type Result = Option<MediaFrameQueueReceiver>;
}

struct RegisterStream(String, MediaFrameQueue);

impl Message for RegisterStream {
    type Result = ();
}

struct RemoveStream(String);

impl Message for RemoveStream {
    type Result = ();
}

#[derive(Default)]
struct StreamRepository {
    streams: HashMap<String, MediaFrameQueue>,
}

impl Actor for StreamRepository {
    type Context = Context<Self>;
}

impl Handler<FindStream> for StreamRepository {
    type Result = Option<MediaFrameQueueReceiver>;

    fn handle(&mut self, msg: FindStream, _ctx: &mut Self::Context) -> Self::Result {
        self.streams.get(&msg.0).map(|queue| queue.get_receiver())
    }
}

impl Handler<RegisterStream> for StreamRepository {
    type Result = ();

    fn handle(&mut self, msg: RegisterStream, _ctx: &mut Self::Context) -> Self::Result {
        self.streams.insert(msg.0, msg.1);
    }
}

impl Handler<RemoveStream> for StreamRepository {
    type Result = ();

    fn handle(&mut self, msg: RemoveStream, _ctx: &mut Self::Context) -> Self::Result {
        self.streams.remove(&msg.0);
    }
}

enum WebSocketMessage {
    Codec(u8, u8, u8),
    Segment(bytes::Bytes, Mp4Segment),
    Init(bytes::Bytes),
}

impl Message for WebSocketMessage {
    type Result = ();
}

struct StartWebSocketMedia;

impl Message for StartWebSocketMedia {
    type Result = ();
}

impl Handler<StartWebSocketMedia> for WebSocketMediaStream {
    type Result = ();

    fn handle(&mut self, _msg: StartWebSocketMedia, _ctx: &mut Self::Context) -> Self::Result {
        //self.start_stream(ctx.address());
    }
}

impl Handler<WebSocketMessage> for WebSocketMediaStream {
    type Result = ();

    fn handle(&mut self, msg: WebSocketMessage, ctx: &mut Self::Context) -> Self::Result {
        match msg {
            WebSocketMessage::Codec(profile, constraints, level) => {
                println!("codec! {} {} {}", profile, constraints, level);

                let mut framed = BytesMut::with_capacity(4);
                framed.put_u8(0);
                framed.put_u8(profile);
                framed.put_u8(constraints);
                framed.put_u8(level);

                ctx.binary(framed);
            }
            WebSocketMessage::Init(bytes) => {
                println!("init!");

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
}

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

fn spawn_graph(mut graph: FilterGraph, token: &StopToken) {
    let fut = async move {
        if let Err(e) = graph.run().await {
            error!("{:?}", e);
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

fn spawn_websocket_feeder(
    receiver: Receiver<anyhow::Result<StreamMessage<Mp4Metadata>>>,
    addr: Addr<WebSocketMediaStream>,
    token: &StopToken,
) {
    let fut = async move {
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
    address: String,
    queue: Option<MediaFrameQueueReceiver>,
    stop_source: StopSource,
    paused: bool,
    sent_keyframe: bool,
}

impl WebSocketMediaStream {
    fn new(address: String, queue: MediaFrameQueueReceiver) -> Self {
        let stop_source = StopSource::new();

        WebSocketMediaStream {
            address,
            queue: Some(queue),
            stop_source,
            paused: true,
            sent_keyframe: false,
        }
    }

    fn start_stream(&mut self, addr: Addr<WebSocketMediaStream>) {
        let stop_token = self.stop_source.stop_token();

        // read
        let queue_reader = self.queue.take().unwrap();

        // write
        let (output_filter, receiver) = StreamWriteFilter::new();
        let write_filter = WaitForSyncFrameFilter::new(Box::new(FragmentedMp4WriteFilter::new(
            Box::new(output_filter),
        )));

        let graph = FilterGraph::new(Box::new(queue_reader), Box::new(write_filter));

        spawn_graph(graph, &stop_token);
        spawn_websocket_feeder(receiver, addr, &stop_token);

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
}

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

async fn not_found(req: HttpRequest) -> impl Responder {
    println!("{:?}", req);
    HttpResponse::NotFound().body("Goodbye world!")
}

#[get("/http/{filename}")]
async fn stream_video(
    req: HttpRequest,
    _stream: web::Payload,
    data: web::Data<AppData>,
) -> HttpResponse {
    let path: std::path::PathBuf = req.match_info().query("filename").parse().unwrap();

    let f = path.file_name().unwrap().to_str().unwrap().to_string();

    let queue_receiver = data.stream_repo.send(FindStream(f.clone())).await;

    if let Ok(Some(queue_receiver)) = queue_receiver {
        // write
        let (output_filter, receiver) = ByteStreamWriteFilter::new();
        let write_filter = WaitForSyncFrameFilter::new(Box::new(
            mp42::FragmentedMp4WriteFilter::new(Box::new(output_filter)),
        ));

        //let file = tokio::fs::File::create("test.fmp4").await.unwrap();
        //let write_filter = WaitForSyncFrameFilter::new(Box::new(mp42::FragmentedMp4WriteFilter::new(Box::new(FileWriteFilter::new(file)))));
        //let mut graph = FilterGraph::new(Box::new(queue_receiver), Box::new(write_filter));
        let mut graph = FilterGraph::new(Box::new(queue_receiver), Box::new(write_filter));

        tokio::spawn(async move {
            graph.run().await.unwrap();
        });

        use futures::StreamExt;
        HttpResponse::Ok().streaming(receiver.map(|x| x.map_err(|e| HttpStreamingError(e))))
    } else {
        HttpResponse::NotFound().body("nope!")
    }
}

#[get("/ws/{filename}")]
async fn ws_stream_video(
    req: HttpRequest,
    stream: web::Payload,
    data: web::Data<AppData>,
) -> HttpResponse {
    let path: std::path::PathBuf = req.match_info().query("filename").parse().unwrap();

    let f = path.file_name().unwrap().to_str().unwrap().to_string();

    let queue_receiver = data.stream_repo.send(FindStream(f.clone())).await;

    if let Ok(Some(queue_receiver)) = queue_receiver {
        let ws_stream = WebSocketMediaStream::new(f, queue_receiver);
        let response = ws::start(ws_stream, &req, stream).unwrap();

        response
    } else {
        HttpResponse::NotFound().body("nope!")
    }
}

#[derive(Debug)]
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
}

async fn authenticate_rtmp_stream(_app_name: &str, _stream_key: &str) -> bool {
    true
}

async fn do_rtmp_handshake(
    read: &mut TcpReadFilter,
    write: &mut TcpWriteFilter,
) -> anyhow::Result<ServerSession> {
    use failure::Fail;
    use media::ByteWriteFilter2;

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
                    dbg!(&evt);

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

                            debug!("Accepted connection request");

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

                                debug!("Accepted publish stream request");
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

        debug!("reading from endpoint!");
        let bytes = read.read().await?;
        let results = session.handle_input(&bytes).map_err(|e| e.kind.compat())?;
        debug!(
            "got {} results from endpoint, {} total",
            results.len(),
            r.len()
        );
        r.extend(results);
    }
}

async fn handle_tcp_socket(
    socket: TcpStream,
    stream_repo: Addr<StreamRepository>,
) -> anyhow::Result<()> {
    let sid = String::from("test");

    let queue = MediaFrameQueue::new();

    let (mut read_filter, mut write_filter) = create_tcp_filters(socket, 188 * 8);

    let server_session = do_rtmp_handshake(&mut read_filter, &mut write_filter).await?;

    let rtmp_filter = RtmpReadFilter::new(read_filter, write_filter, server_session);

    //let file = tokio::fs::File::create("test.fmp4").await.unwrap();
    //let mp4_writer = mp42::FragmentedMp4WriteFilter::new(Box::new(FileWriteFilter::new(file)));
    //let mut graph = FilterGraph::new(Box::new(rtmp_filter), Box::new(mp4_writer));
    let mut graph = FilterGraph::new(Box::new(rtmp_filter), Box::new(queue.clone()));

    stream_repo.send(RegisterStream(sid.clone(), queue)).await?;

    if let Err(e) = graph.run().await {
        stream_repo.send(RemoveStream(sid)).await.unwrap();
        Err(e)
    } else {
        stream_repo.send(RemoveStream(sid)).await.unwrap();
        Ok(())
    }
}

fn spawn_listen_for_tcp(stream_repo: Addr<StreamRepository>) {
    let fut = async move {
        let listener = TcpListener::bind("localhost:1935").await.unwrap();

        info!(
            "Listening for RTMP connections at {:?}",
            listener.local_addr()
        );

        loop {
            let (socket, addr) = listener.accept().await.unwrap();
            debug!("RTMP connection from {:?}", addr);

            let stream_repo = stream_repo.clone();
            tokio::spawn(async move {
                match handle_tcp_socket(socket, stream_repo).await {
                    Ok(_) => info!("Finished streaming from RTMP"),
                    Err(e) => warn!("Error while streaming from RTMP: {:?}", e),
                }
            });
        }
    };

    tokio::spawn(fut);
}

fn create_tcp_filters(socket: TcpStream, buffer: usize) -> (TcpReadFilter, TcpWriteFilter) {
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
    stream_repo: Addr<StreamRepository>,
}

fn get_conn_info(connection: &dyn std::any::Any, _data: &mut actix_web::dev::Extensions) {
    if let Some(_sock) = connection.downcast_ref::<TcpStream>() {
        //sock.set_nodelay(true);
    } else {
        unreachable!("connection should only be plaintext since no TLS is set up");
    }
}

#[actix_rt::main]
async fn start() -> Result<(), Error> {
    let stream_repo = StreamRepository::default().start();

    spawn_listen_for_tcp(stream_repo.clone());

    let data = AppData { stream_repo };

    HttpServer::new(move || {
        App::new()
            .data(data.clone())
            .service(hello)
            .service(ws_stream_video)
            .service(stream_video)
            .default_service(web::route().to(not_found))
    })
    .on_connect(get_conn_info)
    .bind("0.0.0.0:8080")?
    .run()
    .await?;

    Ok(())
    /*loop {

        if let Some((_instant, bytes)) = srt_socket.try_next().await? {
        use tokio::io::AsyncWriteExt;
            file.write_all(&bytes).await?;
            //println!("Received {:?} packets", count);
            demux.push(&mut ctx, &bytes);
            count += 1;
        }
    }*/
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    start();

    /*let mut rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {

        let file = tokio::fs::File::create("test.fmp4").await.unwrap();

        let srt_socket = SrtSocketBuilder::new_listen()
            .local_port(3333)
            .connect()
            .await.unwrap();

        let srt_reader = SrtReadFilter::new(srt_socket);
        let file = tokio::fs::File::create("test.fmp4").await.unwrap();
        let mp4_writer = FragmentedMp4WriteFilter::new(Box::new(FileWriteFilter::new(file)));

        let mut graph = FilterGraph::new(Box::new(srt_reader), Box::new(mp4_writer));

        graph.run().await.unwrap();
    });*/

    Ok(())
}
