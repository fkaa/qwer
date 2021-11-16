use av_format::buffer::AccReader;
use av_mp4::boxes::codec::avcc::AvcDecoderConfigurationRecord;
use bytes::Bytes;
use failure::Fail;
use futures::{
    channel::mpsc::{channel, Receiver, Sender},
    SinkExt,
};
use h264_reader::{
    annexb::AnnexBReader,
    nal::{
        pps::{PicParameterSet, PpsError},
        sps::{SeqParameterSet, SpsError},
        NalHandler, NalHeader, NalSwitch, UnitType,
    },
    rbsp::decode_nal,
    Context,
};
use rml_rtmp::{
    chunk_io::Packet,
    handshake::{Handshake, HandshakeProcessResult, PeerType},
    sessions::{
        ServerSession, ServerSessionConfig, ServerSessionEvent, ServerSessionResult, StreamMetadata,
    },
    time::RtmpTimestamp,
};
use tokio::net::TcpListener;
use tracing::*;

use sh_media::{
    split_tcp_filters, AudioCodecInfo, AudioCodecSpecificInfo, BitstreamFraming, ByteReadFilter,
    ByteWriteFilter2, CodecInfo, CodecTypeInfo, Fraction, Frame, FrameDependency, FrameReadFilter,
    MediaTime, SoundType, Stream, TcpReadFilter, TcpWriteFilter, VideoCodecInfo,
    VideoCodecSpecificInfo,
};

use std::{
    cell::RefCell, collections::VecDeque, io::Cursor, net::SocketAddr, sync::Arc, time::Instant,
};

const RTMP_TIMEBASE: Fraction = Fraction::new(1, 1000);
const RTMP_AAC_TIMEBASE: Fraction = Fraction::new(1, 48000);

#[derive(Debug, thiserror::Error)]
pub enum RtmpError {
    #[error("IO error: {0}")]
    TokioIo(#[from] tokio::io::Error),

    //#[error("IO error: {0}")]
    //Io(#[from] std::io::Error),
    #[error("{0}")]
    Handshake(rml_rtmp::handshake::HandshakeErrorKind),

    #[error("{0}")]
    ServerSession(rml_rtmp::sessions::ServerSessionErrorKind),

    #[error("Failed to parse video tag")]
    ParseVideoTag,

    #[error("Failed to parse audio tag")]
    ParseAudioTag,

    #[error("Failed to parse AVC video packet")]
    ParseAvcPacket,

    #[error("{0}")]
    Error(#[from] anyhow::Error),
}

pub struct RtmpListener {
    listener: TcpListener,
}

impl RtmpListener {
    pub async fn bind(addr: String) -> Result<Self, RtmpError> {
        let listener = TcpListener::bind(addr).await?;

        Ok(RtmpListener { listener })
    }

    pub async fn accept(&self) -> Result<(RtmpRequest, String, String), RtmpError> {
        let (socket, addr) = self.listener.accept().await?;
        socket.set_nodelay(true)?;

        let (mut read, mut write) = split_tcp_filters(socket, 188 * 8);
        let (server_session, results, request_id, app, key) =
            process(&mut read, &mut write).await?;

        let request = RtmpRequest {
            write,
            read,
            addr,
            request_id,
            results,
            server_session,
        };

        Ok((request, app, key))
    }
}

pub struct RtmpRequest {
    write: TcpWriteFilter,
    read: TcpReadFilter,
    addr: SocketAddr,
    request_id: u32,
    results: VecDeque<ServerSessionResult>,
    server_session: ServerSession,
}

impl RtmpRequest {
    pub async fn authenticate(mut self) -> Result<RtmpSession, RtmpError> {
        let results = self
            .server_session
            .accept_request(self.request_id)
            .map_err(|e| RtmpError::ServerSession(e.kind))?;

        self.results.extend(results);

        Ok(RtmpSession {
            write: self.write,
            read: self.read,
            server_session: self.server_session,
            results: self.results,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

pub struct RtmpSession {
    write: TcpWriteFilter,
    read: TcpReadFilter,
    server_session: ServerSession,
    results: VecDeque<ServerSessionResult>,
}

#[derive(Debug, Default)]
pub struct ParameterSetContext {
    pub sps: Option<(Vec<u8>, Result<SeqParameterSet, SpsError>)>,
    pub pps: Option<(Vec<u8>, Result<PicParameterSet, PpsError>)>,
}

pub struct RtmpReadFilter {
    read_filter: TcpReadFilter,
    // stop_source: StopSource,
    rtmp_server_session: ServerSession,
    rtmp_tx: Sender<Packet>,

    video_stream: Option<Stream>,
    video_time: u64,
    prev_video_time: Option<RtmpTimestamp>,

    audio_stream: Option<Stream>,
    audio_time: u64,
    prev_audio_time: Option<RtmpTimestamp>,

    results: VecDeque<ServerSessionResult>,
    frames: VecDeque<Frame>,
}

async fn rtmp_write_task(
    mut write_filter: TcpWriteFilter,
    mut rtmp_rx: Receiver<Packet>,
) -> anyhow::Result<()> {
    use futures::stream::StreamExt;

    debug!("Starting RTMP write task");

    while let Some(pkt) = rtmp_rx.next().await {
        write_filter.write(pkt.bytes.into()).await?;
    }

    Ok(())
}

impl RtmpReadFilter {
    pub fn new(session: RtmpSession) -> Self {
        let (rtmp_tx, rtmp_rx) = channel(500);
        // let stop_source = StopSource::new();

        tokio::spawn(/*stop_source.stop_token().stop_future(*/ async move {
            match rtmp_write_task(session.write, rtmp_rx).await {
                Ok(()) => {
                    debug!("RTMP write task finished without errors");
                }
                Err(e) => {
                    warn!("RTMP write task finished with error: {}", e);
                }
            }
        });

        RtmpReadFilter {
            read_filter: session.read,
            // stop_source,
            rtmp_server_session: session.server_session,
            rtmp_tx,

            video_stream: None,
            video_time: 0,
            prev_video_time: None,

            audio_stream: None,
            audio_time: 0,
            prev_audio_time: None,

            results: session.results,
            frames: VecDeque::new(),
        }
    }

    fn assign_audio_stream(&mut self, tag: flvparse::AudioTag) -> anyhow::Result<()> {
        let codec_info = get_audio_codec_info(&tag)?;

        self.audio_stream = Some(Stream {
            id: 1,
            codec: Arc::new(codec_info),
            timebase: RTMP_AAC_TIMEBASE.clone(),
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

        let time = time.in_base(RTMP_AAC_TIMEBASE);

        let frame = Frame {
            time,
            dependency: FrameDependency::None,

            buffer: Bytes::from(audio_tag.body.data[1..].to_vec()),
            stream: self.audio_stream.clone().unwrap(),
            received: Instant::now(),
        };

        self.frames.push_back(frame);

        self.prev_audio_time = Some(timestamp);

        Ok(())
    }

    async fn wait_for_metadata(&mut self) -> anyhow::Result<StreamMetadata> {
        // debug!(self.logger, "Waiting for metadata");

        loop {
            let bytes = self.read_filter.read().await?;
            for res in self
                .rtmp_server_session
                .handle_input(&bytes)
                .map_err(|e| e.kind.compat())?
            {
                match res {
                    ServerSessionResult::OutboundResponse(pkt) => self.rtmp_tx.send(pkt).await?,
                    ServerSessionResult::RaisedEvent(evt) => match evt {
                        ServerSessionEvent::StreamMetadataChanged {
                            app_name: _,
                            stream_key: _,
                            metadata,
                        } => {
                            return Ok(metadata);
                        }
                        _ => {}
                    },
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

    async fn process_results<I: IntoIterator<Item = ServerSessionResult>>(
        &mut self,
        results: I,
    ) -> anyhow::Result<()> {
        for result in results.into_iter() {
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

    async fn get_frame(&mut self) -> anyhow::Result<Frame> {
        loop {
            if let Some(frame) = self.frames.pop_front() {
                return Ok(frame);
            }

            self.fetch().await?;
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for RtmpReadFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        let mut deq = VecDeque::new();
        std::mem::swap(&mut deq, &mut self.results);
        self.process_results(deq).await?;

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
            debug!("Video: {:?}", video);
        }
        if let Some(ref audio) = self.audio_stream {
            debug!("Audio: {:?}", audio);
        }

        let streams = [self.video_stream.clone(), self.audio_stream.clone()];

        Ok(std::array::IntoIter::new(streams)
            .into_iter()
            .filter_map(|x| x)
            .collect())
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        Ok(self.get_frame().await?)
    }
}

fn parse_video_tag(data: &[u8]) -> anyhow::Result<(flvparse::VideoTag, flvparse::AvcVideoPacket)> {
    let tag = flvparse::VideoTag::parse(&data, data.len())
        .map(|(_, t)| t)
        .map_err(|_| RtmpError::ParseVideoTag)?;

    let packet = flvparse::avc_video_packet(&tag.body.data, tag.body.data.len())
        .map(|(_, p)| p)
        .map_err(|_| RtmpError::ParseAvcPacket)?;

    Ok((tag, packet))
}

fn parse_audio_tag(data: &[u8]) -> anyhow::Result<flvparse::AudioTag> {
    let tag = flvparse::AudioTag::parse(&data, data.len())
        .map(|(_, t)| t)
        .map_err(|_| RtmpError::ParseAudioTag)?;

    Ok(tag)
}

fn get_codec_from_nalu(packet: &flvparse::AvcVideoPacket) -> anyhow::Result<CodecInfo> {
    let parameter_sets = find_parameter_sets(&packet.avc_data);
    let codec_info = get_video_codec_info(parameter_sets)?;

    Ok(codec_info)
}

fn get_codec_from_mp4(packet: &flvparse::AvcVideoPacket) -> anyhow::Result<CodecInfo> {
    let cursor = Cursor::new(packet.avc_data);
    let mut reader = AccReader::new(cursor);
    let mut record = AvcDecoderConfigurationRecord::read(&mut reader)?;

    /*error!(
        "get_codec_from_mp4 SPS: {}",
        base64::encode(&record.sequence_parameter_set)
    );*/
    // FIXME Always uses first set
    let sps = SeqParameterSet::from_bytes(&decode_nal(&record.sequence_parameter_sets[0].0[1..]))
        .map_err(|e| anyhow::anyhow!("{:?}", e))?;

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
                // FIXME Always uses first set
                sps: Arc::new(record.sequence_parameter_sets.remove(0).0),
                pps: Arc::new(record.picture_parameter_sets.remove(0).0),
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
            extra: AudioCodecSpecificInfo::Aac {
                extra: match tag.body.data[0] {
                    // TODO Maybe this doesn't have to be owned
                    0 => tag.body.data[1..].to_owned(), // AudioSpecificConfig
                    1 => unimplemented!("Raw AAC frame data"),
                    _ => panic!("Unknown AACPacketType"),
                },
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

async fn process(
    read: &mut TcpReadFilter,
    write: &mut TcpWriteFilter,
) -> Result<
    (
        ServerSession,
        VecDeque<ServerSessionResult>,
        u32,
        String,
        String,
    ),
    RtmpError,
> {
    let mut handshake = Handshake::new(PeerType::Server);

    // Do initial RTMP handshake
    let (response, remaining) = loop {
        let bytes = read.read().await?;
        let response = match handshake.process_bytes(&bytes) {
            Ok(HandshakeProcessResult::InProgress { response_bytes }) => response_bytes,
            Ok(HandshakeProcessResult::Completed {
                response_bytes,
                remaining_bytes,
            }) => break (response_bytes, remaining_bytes),
            Err(e) => return Err(RtmpError::Handshake(e.kind)),
        };

        write.write(response.into()).await?;
    };

    write.write(response.into()).await?;

    // Create the RTMP session
    let config = ServerSessionConfig::new();
    let (mut session, initial_results) =
        ServerSession::new(config).map_err(|e| RtmpError::ServerSession(e.kind))?;

    let results = session
        .handle_input(&remaining)
        .map_err(|e| RtmpError::ServerSession(e.kind))?;

    let mut r = VecDeque::new();
    let mut stream_info = None;

    r.extend(results.into_iter().chain(initial_results.into_iter()));

    // TODO: add a timeout to the handshake process
    // Loop until we get a publish request
    loop {
        while let Some(res) = r.pop_front() {
            match res {
                ServerSessionResult::OutboundResponse(packet) => {
                    write.write(packet.bytes.into()).await?
                }
                ServerSessionResult::RaisedEvent(evt) => match evt {
                    ServerSessionEvent::ConnectionRequested {
                        request_id,
                        app_name: _,
                    } => {
                        r.extend(
                            session
                                .accept_request(request_id)
                                .map_err(|e| RtmpError::ServerSession(e.kind))?,
                        );

                        debug!("Accepted connection request");
                    }
                    ServerSessionEvent::PublishStreamRequested {
                        request_id,
                        app_name,
                        stream_key,
                        mode: _,
                    } => {
                        stream_info = Some((request_id, app_name, stream_key));
                    }
                    _ => {}
                },
                ServerSessionResult::UnhandleableMessageReceived(_payload) => {}
            }
        }

        // Return the partial session (unauthenticated) when we
        // receive a publish request
        if let Some((request_id, app, key)) = stream_info.take() {
            return Ok((session, r, request_id, app, key));
        }

        // debug!("reading from endpoint!");
        let bytes = read.read().await?;
        let results = session
            .handle_input(&bytes)
            .map_err(|e| RtmpError::ServerSession(e.kind))?;
        r.extend(results);
    }
}
