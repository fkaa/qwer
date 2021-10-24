use crate::{
    AudioCodecInfo, ByteReadFilter, CodecInfo, CodecTypeInfo, Fraction, Frame, FrameDependency,
    FrameReadFilter, MediaTime, SoundType, Stream, VideoCodecInfo,
    VideoCodecSpecificInfo, MediaFrameQueue, FrameAnalyzerFilter, FilterGraph, ByteWriteFilter2,
    BitstreamFraming,
};

use bytes::Bytes;
use failure::Fail;
use fmp4::AvcDecoderConfigurationRecord;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    SinkExt,
};
use h264_reader::{
    annexb::{AnnexBReader},
    nal::{
        pps::{PicParameterSet, PpsError},
        sps::{SeqParameterSet, SpsError},
        NalHandler, NalHeader, NalSwitch, UnitType,
    },
    rbsp::decode_nal,
    Context,
};
use crate::logger::*;
use crate::StreamRepository;

use slog::{debug, info, warn};

use tokio::net::{tcp, TcpListener, TcpStream};

use rml_rtmp::{
    chunk_io::Packet,
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult, StreamMetadata},
    time::RtmpTimestamp,
};
use stop_token::StopSource;

use std::{cell::RefCell, collections::VecDeque, io::{self, Cursor}, sync::{Arc, RwLock}, time::Instant};

const RTMP_TIMEBASE: Fraction = Fraction::new(1, 1000);

#[derive(Debug, Default)]
pub struct ParameterSetContext {
    pub sps: Option<(Vec<u8>, Result<SeqParameterSet, SpsError>)>,
    pub pps: Option<(Vec<u8>, Result<PicParameterSet, PpsError>)>,
}

pub struct RtmpReadFilter {
    logger: ContextLogger,

    read_filter: TcpReadFilter,
    stop_source: StopSource,

    rtmp_server_session: ServerSession,
    rtmp_tx: Sender<Packet>,

    video_stream: Option<Stream>,
    video_time: u64,
    prev_video_time: Option<RtmpTimestamp>,

    audio_stream: Option<Stream>,
    audio_time: u64,
    prev_audio_time: Option<RtmpTimestamp>,

    frames: VecDeque<Frame>,
}

async fn rtmp_write_task(
    logger: &ContextLogger,
    mut write_filter: TcpWriteFilter,
    mut rtmp_rx: Receiver<Packet>,
) -> anyhow::Result<()> {
    use crate::media::ByteWriteFilter2;
    use futures::stream::StreamExt;

    debug!(logger, "Starting RTMP write task");

    loop {
        while let Some(pkt) = rtmp_rx.next().await {
            write_filter.write(pkt.bytes.into()).await?;
        }
    }

    Ok(())
}

impl RtmpReadFilter {
    pub fn new(
        logger: ContextLogger,
        read_filter: TcpReadFilter,
        write_filter: TcpWriteFilter,
        rtmp_server_session: ServerSession,
    ) -> Self {
        let (rtmp_tx, rtmp_rx) = channel(50);
        let stop_source = StopSource::new();

        let write_task_logger = logger.scope();
        tokio::spawn(stop_source.stop_token().stop_future(async move {
            match rtmp_write_task(&write_task_logger, write_filter, rtmp_rx).await {
                Ok(()) => {
                    debug!(write_task_logger, "RTMP write task finished without errors");
                }
                Err(e) => {
                    warn!(write_task_logger, "RTMP write task finished with error: {}", e);
                }
            }
        }));

        RtmpReadFilter {
            logger,
            read_filter,
            stop_source,

            rtmp_server_session,
            rtmp_tx,

            video_stream: None,
            video_time: 0,
            prev_video_time: None,

            audio_stream: None,
            audio_time: 0,
            prev_audio_time: None,

            frames: VecDeque::new(),
        }
    }

    fn assign_audio_stream(&mut self, tag: flvparse::AudioTag) -> anyhow::Result<()> {
        let codec_info = get_audio_codec_info(&tag)?;

        self.audio_stream = Some(Stream {
            id: 1,
            codec: Arc::new(codec_info),
            timebase: RTMP_TIMEBASE.clone(),
        });

        Ok(())
    }

    fn assign_video_stream(
        &mut self,
        _tag: flvparse::VideoTag,
        packet: flvparse::AvcVideoPacket,
    ) -> anyhow::Result<()> {
        let codec_info = match packet.packet_type {
            flvparse::AvcPacketType::SequenceHeader => get_codec_from_mp4(&packet)?,
            flvparse::AvcPacketType::NALU => get_codec_from_nalu(&packet)?,
            _ => anyhow::bail!("Unsupported AVC packet type: {:?}", packet.packet_type),
        };

        self.video_stream = Some(Stream {
            id: 0,
            codec: Arc::new(codec_info),
            timebase: RTMP_TIMEBASE.clone(),
        });

        Ok(())
    }

    fn add_video_frame(&mut self, data: Bytes, timestamp: RtmpTimestamp) -> anyhow::Result<()> {
        let (video_tag, video_packet) = parse_video_tag(&data)?;

        if self.video_stream.is_none() {
            self.assign_video_stream(video_tag, video_packet)?;
            return Ok(());
        }

        if self.prev_video_time.is_none() {
            self.prev_video_time = Some(timestamp);
        }

        let diff = timestamp - self.prev_video_time.unwrap_or(RtmpTimestamp::new(0));

        self.video_time += diff.value as u64;

        let time = MediaTime {
            pts: self.video_time,
            dts: None,
            timebase: RTMP_TIMEBASE.clone(),
        };


            // let nals = crate::parse_bitstream(Bytes::from(video_packet.avc_data.to_vec()), crate::BitstreamFraming::FourByteLength);

            // info!(self.logger, "RTMP ours: {}", nals.iter().map(|n| format!("{:?}({})", crate::nut_header(n), n.len())).collect::<Vec<_>>().join(","));

        let frame = Frame {
            time,
            dependency: if video_tag.header.frame_type == flvparse::FrameType::Key {
                FrameDependency::None
            } else {
                FrameDependency::Backwards
            },
            buffer: video_packet.avc_data.to_vec().into(),
            stream: self.video_stream.clone().unwrap(),
            received: Instant::now(),
        };

        self.frames.push_back(frame);

        self.prev_video_time = Some(timestamp);

        Ok(())
    }

    fn add_audio_frame(&mut self, data: Bytes, timestamp: RtmpTimestamp) -> anyhow::Result<()> {
        let audio_tag = parse_audio_tag(&data)?;

        if self.audio_stream.is_none() {
            self.assign_audio_stream(audio_tag)?;
            return Ok(());
        }

        if self.prev_audio_time.is_none() {
            self.prev_audio_time = Some(timestamp);
        }

        let diff = timestamp - self.prev_audio_time.unwrap_or(RtmpTimestamp::new(0));

        self.audio_time += diff.value as u64;

        let time = MediaTime {
            pts: self.audio_time,
            dts: None,
            timebase: RTMP_TIMEBASE.clone(),
        };

        let frame = Frame {
            time,
            dependency: FrameDependency::None,

            buffer: data,
            stream: self.audio_stream.clone().unwrap(),
            received: Instant::now(),
        };

        self.frames.push_back(frame);

        self.prev_audio_time = Some(timestamp);

        Ok(())
    }

    async fn wait_for_metadata(&mut self) -> anyhow::Result<StreamMetadata> {
        debug!(self.logger, "Waiting for metadata");

        loop {
            let bytes = self.read_filter.read().await?;
            for res in self
                .rtmp_server_session
                .handle_input(&bytes)
                .map_err(|e| e.kind.compat())?
            {
                match res {
                    ServerSessionResult::OutboundResponse(pkt) => self.rtmp_tx.send(pkt).await?,
                    ServerSessionResult::RaisedEvent(evt) => {
                        // dbg!(&evt);

                        match evt {
                            ServerSessionEvent::StreamMetadataChanged {
                                app_name: _,
                                stream_key: _,
                                metadata,
                            } => {
                                return Ok(metadata);
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    async fn process_event(&mut self, event: ServerSessionEvent) -> anyhow::Result<()> {
        match event {
            ServerSessionEvent::AudioDataReceived {
                app_name: _,
                stream_key: _,
                data,
                timestamp,
            } => {
                self.add_audio_frame(data, timestamp)?;
            }
            ServerSessionEvent::VideoDataReceived {
                app_name: _,
                stream_key: _,
                data,
                timestamp,
            } => {
                self.add_video_frame(data, timestamp)?;
            }
            _ => {}
        }

        Ok(())
    }

    async fn process_results(&mut self, results: Vec<ServerSessionResult>) -> anyhow::Result<()> {
        for result in results {
            match result {
                ServerSessionResult::OutboundResponse(pkt) => self.rtmp_tx.send(pkt).await?,
                ServerSessionResult::RaisedEvent(evt) => self.process_event(evt).await?,
                ServerSessionResult::UnhandleableMessageReceived(_payload) => {}
            }
        }

        Ok(())
    }

    async fn fetch(&mut self) -> anyhow::Result<()> {
        let bytes = self.read_filter.read().await?;
        let results = self
            .rtmp_server_session
            .handle_input(&bytes)
            .map_err(|e| e.kind.compat())?;

        self.process_results(results).await?;

        Ok(())
    }

    async fn try_get_frame(&mut self) -> anyhow::Result<Option<Frame>> {
        if let Some(frame) = self.frames.pop_front() {
            return Ok(Some(frame));
        }

        self.fetch().await?;

        Ok(self.frames.pop_front())
    }

    async fn get_frame(&mut self) -> anyhow::Result<Frame> {
        loop {
            if let Some(frame) = self.try_get_frame().await? {
                return Ok(frame);
            }
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for RtmpReadFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        self.read_filter.start().await?;

        let metadata = self.wait_for_metadata().await?;

        let expecting_video = metadata.video_width.is_some();
        let expecting_audio = metadata.audio_sample_rate.is_some();

        while (expecting_video && self.video_stream.is_none())
            || (expecting_audio && self.audio_stream.is_none())
        {
            self.fetch().await?;
        }

        if let Some(ref video) = self.video_stream {
            debug!(self.logger, "Video: {:?}", video);
        }
        if let Some(ref audio) = self.audio_stream {
            debug!(self.logger, "Audio: {:?}", audio);
        }

        let streams =
            [
                self.video_stream.clone(),
                self.audio_stream.clone()
            ];

        Ok(
            std::array::IntoIter::new(streams)
                .into_iter()
                .filter_map(|x| x)
                .collect()
        )
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        Ok(self.get_frame().await?)
    }
}

fn parse_video_tag(data: &[u8]) -> anyhow::Result<(flvparse::VideoTag, flvparse::AvcVideoPacket)> {
    let tag = flvparse::VideoTag::parse(&data, data.len())
        .map(|(_, t)| t)
        .map_err(|_| anyhow::anyhow!("Failed to parse video tag"))?;

    let packet = flvparse::avc_video_packet(&tag.body.data, tag.body.data.len())
        .map(|(_, p)| p)
        .map_err(|_| anyhow::anyhow!("Failed to parse AVC video packet"))?;

    Ok((tag, packet))
}

fn parse_audio_tag(data: &[u8]) -> anyhow::Result<flvparse::AudioTag> {
    flvparse::AudioTag::parse(&data, data.len())
        .map(|(_, t)| t)
        .map_err(|_| anyhow::anyhow!("Failed to parse audio tag"))
}

fn get_codec_from_nalu(packet: &flvparse::AvcVideoPacket) -> anyhow::Result<CodecInfo> {
    let parameter_sets = find_parameter_sets(&packet.avc_data);
    let codec_info = get_video_codec_info(parameter_sets)?;

    Ok(codec_info)
}

fn get_codec_from_mp4(packet: &flvparse::AvcVideoPacket) -> anyhow::Result<CodecInfo> {
    let mut reader = Cursor::new(packet.avc_data);
    let record = AvcDecoderConfigurationRecord::read(&mut reader)?;

    /*error!(
        "get_codec_from_mp4 SPS: {}",
        base64::encode(&record.sequence_parameter_set)
    );*/
    let sps = SeqParameterSet::from_bytes(&decode_nal(&record.sequence_parameter_set[1..]))
        .map_err(|e| anyhow::anyhow!("{:?}", e))?;

    //dbg!(&sps);

    let (width, height) = sps.pixel_dimensions().unwrap();

    Ok(CodecInfo {
        name: "h264",
        properties: CodecTypeInfo::Video(VideoCodecInfo {
            width,
            height,
            extra: VideoCodecSpecificInfo::H264 {
                bitstream_format: BitstreamFraming::FourByteLength,
                profile_indication: record.profile_indication,
                profile_compatibility: record.profile_compatibility,
                level_indication: record.level_indication,
                sps: Arc::new(record.sequence_parameter_set),
                pps: Arc::new(record.picture_parameter_set),
            },
        }),
    })
}

fn find_parameter_sets(bytes: &[u8]) -> ParameterSetContext {
    let mut s = NalSwitch::default();
    s.put_handler(
        UnitType::SeqParameterSet,
        Box::new(RefCell::new(SpsHandler)),
    );
    s.put_handler(
        UnitType::PicParameterSet,
        Box::new(RefCell::new(PpsHandler)),
    );

    let mut ctx = Context::new(ParameterSetContext::default());

    let mut reader = AnnexBReader::new(s);
    reader.start(&mut ctx);
    reader.push(&mut ctx, bytes);
    reader.end_units(&mut ctx);

    ctx.user_context
}

fn get_video_codec_info(parameter_sets: ParameterSetContext) -> anyhow::Result<CodecInfo> {
    let (sps_bytes, sps) = parameter_sets.sps.unwrap();
    let (pps_bytes, _pps) = parameter_sets.pps.unwrap();

    let sps = sps.unwrap();

    dbg!(&sps);

    let (width, height) = sps.pixel_dimensions().unwrap();

    let profile_indication = sps.profile_idc.into();
    let profile_compatibility = sps.constraint_flags.into();
    let level_indication = sps.level_idc;

    Ok(CodecInfo {
        name: "h264",
        properties: CodecTypeInfo::Video(VideoCodecInfo {
            width,
            height,
            extra: VideoCodecSpecificInfo::H264 {
                bitstream_format: BitstreamFraming::FourByteLength,
                profile_indication,
                profile_compatibility,
                level_indication,
                sps: Arc::new(sps_bytes),
                pps: Arc::new(pps_bytes),
            },
        }),
    })
}

fn get_audio_codec_info(tag: &flvparse::AudioTag) -> anyhow::Result<CodecInfo> {
    let name = match tag.header.sound_format {
        flvparse::SoundFormat::AAC => "AAC",
        _ => anyhow::bail!("Unsupported audio codec {:?}", tag.header.sound_format),
    };

    Ok(CodecInfo {
        name,
        properties: CodecTypeInfo::Audio(AudioCodecInfo {
            sample_rate: match tag.header.sound_rate {
                flvparse::SoundRate::_5_5KHZ => 5500,
                flvparse::SoundRate::_11KHZ => 11000,
                flvparse::SoundRate::_22KHZ => 22000,
                flvparse::SoundRate::_44KHZ => 44000,
            },
            sample_bpp: match tag.header.sound_size {
                flvparse::SoundSize::_8Bit => 8,
                flvparse::SoundSize::_16Bit => 16,
            },
            sound_type: match tag.header.sound_type {
                flvparse::SoundType::Mono => SoundType::Mono,
                flvparse::SoundType::Stereo => SoundType::Stereo,
            },
        }),
    })
}

pub struct SpsHandler;
pub struct PpsHandler;

impl NalHandler for SpsHandler {
    type Ctx = ParameterSetContext;

    fn start(&mut self, _ctx: &mut Context<Self::Ctx>, _header: NalHeader) {}

    fn push(&mut self, ctx: &mut Context<Self::Ctx>, buf: &[u8]) {
        // error!("handle SPS: {}", base64::encode(&buf[1..]));
        let sps = SeqParameterSet::from_bytes(&decode_nal(&buf[1..]));
        if let Ok(sps) = &sps {
            ctx.put_seq_param_set(sps.clone());
        }
        ctx.user_context.sps = Some((buf.to_vec(), sps));
    }

    fn end(&mut self, _ctx: &mut Context<Self::Ctx>) {}
}

impl NalHandler for PpsHandler {
    type Ctx = ParameterSetContext;

    fn start(&mut self, _ctx: &mut Context<Self::Ctx>, _header: NalHeader) {}

    fn push(&mut self, ctx: &mut Context<Self::Ctx>, buf: &[u8]) {
        // error!("handle PPS: {}", base64::encode(&buf[1..]));
        ctx.user_context.pps = Some((
            buf.to_vec(),
            PicParameterSet::from_bytes(ctx, &decode_nal(&buf[1..])),
        ));
    }

    fn end(&mut self, _ctx: &mut Context<Self::Ctx>) {}
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

async fn authenticate_rtmp_stream(_app_name: &str, supplied_stream_key: &str) -> bool {
    std::env::var("STREAM_KEY").map(|key| key == supplied_stream_key).unwrap_or(true)
}

pub fn listen(
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
