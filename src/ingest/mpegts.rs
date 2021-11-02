use std::cell::RefCell;

use std::collections::{HashMap, VecDeque};

use std::sync::{Arc, RwLock};
use std::time::Instant;

use byteorder::{BigEndian, ByteOrder};
use futures_util::{StreamExt, TryStreamExt};

use mpeg2ts_reader::{demultiplex, packet, packet_filter_switch, pes, psi, StreamType};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        BodyStream, Extension, Path,
    },
    response::IntoResponse,
};
use h264_reader::{
    annexb::{AnnexBReader, NalReader},
    nal::{
        pps::{PicParameterSet, PpsError},
        sps::{SeqParameterSet, SpsError},
        GenericNalSwitch, NalHandler, NalHeader, NalSwitch, UnitType,
    },
    rbsp::decode_nal,
    Context,
};

use async_channel::{Receiver, Sender};
use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::media::*;

use crate::{AppData, ContextLogger, StreamRepository};

use slog::{debug, error};

pub struct BodyStreamFilter {
    stream: BodyStream,
    buffer: BytesMut,
}

impl BodyStreamFilter {
    pub fn new(stream: BodyStream) -> BodyStreamFilter {
        BodyStreamFilter {
            stream,
            buffer: BytesMut::with_capacity(188 * 8),
        }
    }
}

#[async_trait::async_trait]
impl ByteReadFilter for BodyStreamFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn read(&mut self) -> anyhow::Result<Bytes> {
        loop {
            if self.buffer.len() >= 188 * 8 {
                let buf = self.buffer.split_to(188 * 8);
                return Ok(buf.freeze());
            } else {
                let chunk = self.stream.next().await.unwrap()?;
                // println!("chunk len: {}", chunk.len());
                self.buffer.extend_from_slice(&chunk);
            }
        }
    }
}

pub async fn mpegts_ingest(
    Path(stream_id): Path<String>,
    stream: BodyStream,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    let logger = data.logger.scope();

    debug!(logger, "Received MPEG-TS request for '{}'", stream_id);

    if let Err(e) = start_mpegts_stream(&logger, stream_id, stream, &data.stream_repo).await {
        error!(logger, "MPEG-TS ingest failed: {}", e);
    }
}

pub async fn start_mpegts_stream(
    logger: &ContextLogger,
    sid: String,
    stream: BodyStream,
    stream_repo: &Arc<RwLock<StreamRepository>>,
) -> anyhow::Result<()> {
    let queue = MediaFrameQueue::new(logger.clone());
    let read_filter = BodyStreamFilter::new(stream);
    let mpegts_filter = MpegTsReadFilter::new(Box::new(read_filter));

    let mpegts_analyzer = FrameAnalyzerFilter::read(logger.clone(), Box::new(mpegts_filter));

    let mut graph = FilterGraph::new(Box::new(mpegts_analyzer), Box::new(queue.clone()));

    stream_repo
        .write()
        .unwrap()
        .streams
        .insert(sid.clone(), queue);

    let result = graph.run().await;

    stream_repo.write().unwrap().streams.remove(&sid);

    result
}

#[derive(Debug)]
pub struct MpegTsPacket {
    id: packet::Pid,
    payload: BytesMut,
    pts: Option<pes::Timestamp>,
    dts: Option<pes::Timestamp>,
    instant: Instant,
}

pub struct MpegTsReadFilter {
    ctx: DumpDemuxContext,
    demux: demultiplex::Demultiplex<DumpDemuxContext>,
    stream: Option<Stream>,
    queue: VecDeque<MpegTsPacket>,
    read_filter: Box<dyn ByteReadFilter + Send + Sync + Unpin>,
}

impl MpegTsReadFilter {
    pub fn new(read_filter: Box<dyn ByteReadFilter + Send + Sync + Unpin>) -> Self {
        let mut ctx = DumpDemuxContext::new();
        let mut demux = demultiplex::Demultiplex::new(&mut ctx);
        Self {
            ctx,
            demux,
            stream: None,
            queue: VecDeque::new(),
            read_filter,
        }
    }

    async fn read_packet(&mut self) -> anyhow::Result<MpegTsPacket> {
        loop {
            let bytes = self.read_filter.read().await?;

            self.demux.push(&mut self.ctx, &bytes);

            if let Some(packet) = self.ctx.take_packet() {
                return Ok(packet);
            }
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for MpegTsReadFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        self.read_filter.start().await?;

        let packet = self.read_packet().await?;

        let parameter_sets = find_parameter_sets(&packet.payload);
        dbg!(&parameter_sets);
        let codec_info = get_codec_info(parameter_sets)?;

        let stream = Stream {
            id: u16::from(packet.id) as u32,
            codec: Arc::new(codec_info),
            timebase: Fraction::new(1, 90_000),
        };

        self.stream = Some(stream.clone());

        self.queue.push_front(packet);

        Ok(vec![stream])
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        let packet = self.read_packet().await?;
        let stream = self.stream.clone().unwrap();

        let pts = packet.dts.or(packet.pts).map(|t| t.value()).unwrap();
        let time = MediaTime {
            pts: pts,
            dts: Some(pts),
            timebase: Fraction::new(1, 90_000),
        };

        //dbg!(&time);

        let (is_sync, is_non_sync) = find_idr_nuts(&packet.payload);

        assert!((is_sync && !is_non_sync) || (!is_sync && is_non_sync));

        let dependency = if is_sync {
            FrameDependency::None
        } else {
            FrameDependency::Backwards
        };

        let buffer = frame_nal_unit(&packet.payload, H264BitstreamFraming::Avc).freeze();
        // let buffer = packet.payload.freeze();

        Ok(Frame {
            time,
            dependency,
            buffer,
            stream,
            received: packet.instant,
        })
    }
}

fn find_idr_nuts(bytes: &[u8]) -> (bool, bool) {
    let mut s = NalSwitch::default();
    s.put_handler(
        UnitType::SliceLayerWithoutPartitioningIdr,
        Box::new(RefCell::new(IdrHandler)),
    );
    s.put_handler(
        UnitType::SliceLayerWithoutPartitioningNonIdr,
        Box::new(RefCell::new(NonIdrHandler)),
    );

    let mut ctx = Context::new(NutScanner(false, false));

    let mut reader = AnnexBReader::new(s);
    reader.start(&mut ctx);
    reader.push(&mut ctx, bytes);
    reader.end_units(&mut ctx);

    (ctx.user_context.0, ctx.user_context.1)
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

fn get_codec_info(parameter_sets: ParameterSetContext) -> anyhow::Result<CodecInfo> {
    let (sps_bytes, sps) = parameter_sets.sps.unwrap();
    let (pps_bytes, _pps) = parameter_sets.pps.unwrap();

    let sps = sps.unwrap();

    // dbg!(&sps);

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

// This macro invocation creates an enum called DumpFilterSwitch, encapsulating all possible ways
// that this application may handle transport stream packets.  Each enum variant is just a wrapper
// around an implementation of the PacketFilter trait
packet_filter_switch! {
    DumpFilterSwitch<DumpDemuxContext> {
        // the DumpFilterSwitch::Pes variant will perform the logic actually specific to this
        // application,
        Pes: pes::PesPacketFilter<DumpDemuxContext,PtsDumpElementaryStreamConsumer>,

        // these definitions are boilerplate required by the framework,
        Pat: demultiplex::PatPacketFilter<DumpDemuxContext>,
        Pmt: demultiplex::PmtPacketFilter<DumpDemuxContext>,

        // this variant will be used when we want to ignore data in the transport stream that this
        // application does not care about
        Null: demultiplex::NullPacketFilter<DumpDemuxContext>,
    }
}

#[derive(Debug, Default)]
pub struct ParameterSetContext {
    pub sps: Option<(Vec<u8>, Result<SeqParameterSet, SpsError>)>,
    pub pps: Option<(Vec<u8>, Result<PicParameterSet, PpsError>)>,
}

#[derive(Copy, Clone)]
pub enum H264BitstreamFraming {
    Avc,
    AnnexB,
}

struct NalFramerContext {
    bytes: BytesMut,
}

impl NalFramerContext {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: BytesMut::with_capacity(capacity),
        }
    }
}

struct NalFramer {
    framing: H264BitstreamFraming,
    nal_start: usize,
}

impl NalFramer {
    pub fn new(framing: H264BitstreamFraming) -> Self {
        Self {
            framing,
            nal_start: 0,
        }
    }
}

impl NalHandler for NalFramer {
    type Ctx = NalFramerContext;

    fn start(&mut self, ctx: &mut Context<Self::Ctx>, _header: NalHeader) {
        self.nal_start = ctx.user_context.bytes.len();
        // if framing is AVC we simply patch the prefix with the length when it has ended
        ctx.user_context.bytes.extend(&[0, 0, 0, 1]);
    }

    fn push(&mut self, ctx: &mut Context<Self::Ctx>, buf: &[u8]) {
        ctx.user_context.bytes.extend(buf);
    }

    fn end(&mut self, ctx: &mut Context<Self::Ctx>) {
        if let H264BitstreamFraming::Avc = self.framing {
            let len = ctx.user_context.bytes.len() - self.nal_start - 4;

            BigEndian::write_u32(
                &mut ctx.user_context.bytes[self.nal_start..(self.nal_start + 4)],
                len as _,
            );
        }
    }
}

fn frame_nal_unit(nal: &[u8], framing: H264BitstreamFraming) -> BytesMut {
    let s = GenericNalSwitch::new(NalFramer::new(framing));
    let framing_context = NalFramerContext::with_capacity(nal.len());

    let mut ctx = Context::new(framing_context);
    let mut reader = AnnexBReader::new(s);
    reader.start(&mut ctx);
    reader.push(&mut ctx, nal);
    reader.end_units(&mut ctx);

    ctx.user_context.bytes
}

struct NutScanner(bool, bool);
pub struct SpsHandler;
pub struct PpsHandler;

struct IdrHandler;
struct NonIdrHandler;

impl NalHandler for IdrHandler {
    type Ctx = NutScanner;

    fn start(&mut self, ctx: &mut Context<Self::Ctx>, _header: NalHeader) {
        ctx.user_context.0 = true;
    }

    fn push(&mut self, _ctx: &mut Context<Self::Ctx>, _buf: &[u8]) {}

    fn end(&mut self, _ctx: &mut Context<Self::Ctx>) {}
}

impl NalHandler for NonIdrHandler {
    type Ctx = NutScanner;

    fn start(&mut self, ctx: &mut Context<Self::Ctx>, _header: NalHeader) {
        ctx.user_context.1 = true;
    }

    fn push(&mut self, _ctx: &mut Context<Self::Ctx>, _buf: &[u8]) {}

    fn end(&mut self, _ctx: &mut Context<Self::Ctx>) {}
}

impl NalHandler for SpsHandler {
    type Ctx = ParameterSetContext;

    fn start(&mut self, _ctx: &mut Context<Self::Ctx>, _header: NalHeader) {}

    fn push(&mut self, ctx: &mut Context<Self::Ctx>, buf: &[u8]) {
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
        ctx.user_context.pps = Some((
            buf.to_vec(),
            PicParameterSet::from_bytes(ctx, &decode_nal(&buf[1..])),
        ));
    }

    fn end(&mut self, _ctx: &mut Context<Self::Ctx>) {}
}

pub struct DumpDemuxContext {
    changeset: demultiplex::FilterChangeset<DumpFilterSwitch>,
    available_packets: VecDeque<MpegTsPacket>,
    unfinished_packets: HashMap<packet::Pid, MpegTsPacket>,
    instant: Instant,
}
impl DumpDemuxContext {
    pub fn new() -> Self {
        DumpDemuxContext {
            changeset: demultiplex::FilterChangeset::default(),
            available_packets: VecDeque::new(),
            unfinished_packets: HashMap::new(),
            instant: Instant::now(),
        }
    }

    pub fn take_packet(&mut self) -> Option<MpegTsPacket> {
        // println!("take!");
        self.available_packets.pop_front()
    }
}
impl demultiplex::DemuxContext for DumpDemuxContext {
    type F = DumpFilterSwitch;
    fn filter_changeset(&mut self) -> &mut demultiplex::FilterChangeset<Self::F> {
        &mut self.changeset
    }
    fn construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> Self::F {
        self.do_construct(req)
    }
}

impl DumpDemuxContext {
    fn do_construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> DumpFilterSwitch {
        // dbg!(&req);
        match req {
            demultiplex::FilterRequest::ByPid(packet::Pid::PAT) => {
                DumpFilterSwitch::Pat(demultiplex::PatPacketFilter::default())
            }
            demultiplex::FilterRequest::ByPid(packet::Pid::STUFFING) => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            demultiplex::FilterRequest::ByPid(_) => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            demultiplex::FilterRequest::ByStream {
                stream_type: StreamType::H264,
                pmt,
                stream_info,
                ..
            } => PtsDumpElementaryStreamConsumer::construct(pmt, stream_info),
            demultiplex::FilterRequest::ByStream { .. } => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
            demultiplex::FilterRequest::Pmt {
                pid,
                program_number,
            } => DumpFilterSwitch::Pmt(demultiplex::PmtPacketFilter::new(pid, program_number)),
            demultiplex::FilterRequest::Nit { .. } => {
                DumpFilterSwitch::Null(demultiplex::NullPacketFilter::default())
            }
        }
    }
}

// Implement the ElementaryStreamConsumer to just dump and PTS/DTS timestamps to stdout
pub struct PtsDumpElementaryStreamConsumer {
    pid: packet::Pid,
    len: Option<usize>,
}
impl PtsDumpElementaryStreamConsumer {
    fn construct(
        _pmt_sect: &psi::pmt::PmtSection,
        stream_info: &psi::pmt::StreamInfo,
    ) -> DumpFilterSwitch {
        let filter = pes::PesPacketFilter::new(PtsDumpElementaryStreamConsumer {
            pid: stream_info.elementary_pid(),
            len: None,
        });
        DumpFilterSwitch::Pes(filter)
    }
}
impl pes::ElementaryStreamConsumer<DumpDemuxContext> for PtsDumpElementaryStreamConsumer {
    fn start_stream(&mut self, _ctx: &mut DumpDemuxContext) {
        // println!("start_stream");
    }
    fn begin_packet(&mut self, ctx: &mut DumpDemuxContext, header: pes::PesHeader) {
        // println!("begin_packet");
        // dbg!(header.pes_packet_length());
        match header.contents() {
            pes::PesContents::Parsed(Some(parsed)) => {
                let (pts, dts) = match parsed.pts_dts() {
                    Ok(pes::PtsDts::PtsOnly(Ok(pts))) => (Some(pts), None),
                    Ok(pes::PtsDts::Both {
                        pts: Ok(pts),
                        dts: Ok(dts),
                    }) => (Some(pts), Some(dts)),
                    _ => (None, None),
                };

                let mut payload = BytesMut::with_capacity(1024);
                payload.extend(parsed.payload());

                let packet = MpegTsPacket {
                    id: self.pid,
                    payload,
                    pts,
                    dts,
                    instant: ctx.instant,
                };
                ctx.unfinished_packets.insert(self.pid, packet);
            }
            pes::PesContents::Parsed(None) => {}
            pes::PesContents::Payload(payload) => {
                let packet = ctx.unfinished_packets.get_mut(&self.pid).unwrap();

                packet.payload.extend(payload);
            }
        }
    }
    fn continue_packet(&mut self, ctx: &mut DumpDemuxContext, data: &[u8]) {
        // println!("continue_packet: {}", data.len());
        //println!("continue: {:?}", self.pid);

        let packet = ctx.unfinished_packets.get_mut(&self.pid).unwrap();
        packet.payload.extend(data);
    }
    fn end_packet(&mut self, ctx: &mut DumpDemuxContext) {
        // println!("end_packet");
        let packet = ctx.unfinished_packets.remove(&self.pid).unwrap();

        // dbg!(packet.payload.len());

        ctx.available_packets.push_back(packet);
    }
    fn continuity_error(&mut self, _ctx: &mut DumpDemuxContext) {
        // println!("continuity_error");
    }
}
