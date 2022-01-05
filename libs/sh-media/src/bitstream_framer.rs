use std::cell::RefCell;

use h264_reader::{
    annexb::AnnexBReader,
    nal::{NalHandler, NalHeader, NalSwitch, UnitType},
    Context,
};

use super::{BitstreamFraming, Frame, FrameWriteFilter, Stream};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use tracing::*;

const THREE_BYTE_STARTCODE: [u8; 3] = [0, 0, 1];
const FOUR_BYTE_STARTCODE: [u8; 4] = [0, 0, 0, 1];

/// Parses a H.26x bitstream framed in AVC format (length prefix) into NAL units.
fn parse_bitstream_length_field<F: Fn(&mut Bytes) -> usize>(
    mut bitstream: Bytes,
    length_size: usize,
    read: F,
) -> Vec<Bytes> {
    let mut nal_units = Vec::new();

    while bitstream.len() > length_size {
        let nal_unit_len = read(&mut bitstream); // BigEndian::read_u32(&nal_units[i..]) as usize;

        let nal_unit = bitstream.split_to(nal_unit_len);
        nal_units.push(nal_unit);
    }

    nal_units
}

/// Parses a H.26x bitstream framed in Annex B format (start codes) into NAL units.
fn parse_bitstream_start_codes(bitstream: Bytes) -> Vec<Bytes> {
    let mut s = NalSwitch::default(); // new(NalFramer::new());
    s.put_handler(
        UnitType::SliceLayerWithoutPartitioningNonIdr,
        Box::new(RefCell::new(GenericNalHandler::with_capacity(1024 * 32))),
    );
    s.put_handler(
        UnitType::SliceLayerWithoutPartitioningIdr,
        Box::new(RefCell::new(GenericNalHandler::with_capacity(1024 * 512))),
    );
    s.put_handler(
        UnitType::SEI,
        Box::new(RefCell::new(GenericNalHandler::with_capacity(1024))),
    );
    s.put_handler(
        UnitType::SeqParameterSet,
        Box::new(RefCell::new(GenericNalHandler::with_capacity(32))),
    );
    s.put_handler(
        UnitType::PicParameterSet,
        Box::new(RefCell::new(GenericNalHandler::with_capacity(32))),
    );

    let framing_context = NalFramerContext::new();

    let mut ctx = Context::new(framing_context);
    let mut reader = AnnexBReader::new(s);
    reader.start(&mut ctx);
    reader.push(&mut ctx, &bitstream);
    reader.end_units(&mut ctx);

    ctx.user_context
        .nal_units
        .into_iter()
        .map(|b| b.freeze())
        .collect()
}

/// Parses a H.26x bitstream in a given [BitstreamFraming] into NAL units.
pub fn parse_bitstream(bitstream: Bytes, source: BitstreamFraming) -> Vec<Bytes> {
    match source {
        BitstreamFraming::TwoByteLength => {
            parse_bitstream_length_field(bitstream, 2, |b| b.get_u16() as usize)
        }
        BitstreamFraming::FourByteLength => {
            parse_bitstream_length_field(bitstream, 4, |b| b.get_u32() as usize)
        }
        BitstreamFraming::FourByteStartCode => parse_bitstream_start_codes(bitstream),
    }
}

/// Frames NAL units with a given start code before each NAL.
fn frame_nal_units_with_start_codes<T: AsRef<[u8]>>(nal_units: &[T], codes: &[u8]) -> BytesMut {
    let mut bitstream = BytesMut::new();

    for nut in nal_units {
        let slice = nut.as_ref();

        bitstream.extend_from_slice(codes);
        bitstream.extend_from_slice(slice);
    }

    bitstream
}

/// Frames NAL units with a length prefix before each NAL.
fn frame_nal_units_with_length<F: Fn(&mut dyn BufMut, usize), T: AsRef<[u8]>>(
    nal_units: &[T],
    write: F,
) -> BytesMut {
    let mut bitstream = BytesMut::new();

    for nut in nal_units {
        let slice = nut.as_ref();

        write(&mut bitstream, slice.len());
        bitstream.extend_from_slice(slice);
    }

    bitstream
}

/// Frame NAL units with the specified [BitstreamFraming].
pub fn frame_nal_units<T: AsRef<[u8]>>(nal_units: &[T], target: BitstreamFraming) -> BytesMut {
    match target {
        BitstreamFraming::TwoByteLength => {
            frame_nal_units_with_length(nal_units, |b, l| b.put_u16(l as u16))
        }
        BitstreamFraming::FourByteLength => {
            frame_nal_units_with_length(nal_units, |b, l| b.put_u32(l as u32))
        }
        BitstreamFraming::FourByteStartCode => {
            frame_nal_units_with_start_codes(nal_units, &FOUR_BYTE_STARTCODE[..])
        }
    }
}

/// Converts a H.26x bitstream from a source [BitstreamFraming] to a
/// target [BitstreamFraming].
fn convert_bitstream(
    bitstream: Bytes,
    source: BitstreamFraming,
    target: BitstreamFraming,
) -> Bytes {
    if source == target {
        return bitstream;
    }

    let nal_units = parse_bitstream(bitstream, source);
    frame_nal_units(&nal_units[..], target).freeze()
}

pub fn is_video_nal_unit(nal: &Bytes) -> bool {
    matches!(
        nut_header(nal),
        Some(UnitType::SeqParameterSet)
            | Some(UnitType::PicParameterSet)
            | Some(UnitType::SliceLayerWithoutPartitioningNonIdr)
            | Some(UnitType::SliceLayerWithoutPartitioningIdr)
    )
}

pub fn nut_header(nal: &Bytes) -> Option<UnitType> {
    NalHeader::new(nal[0]).map(|h| h.nal_unit_type()).ok()
}

struct NalFramerContext {
    nal_units: Vec<BytesMut>,
    current_nal_unit: Vec<u8>,
}

impl NalFramerContext {
    pub fn new() -> Self {
        Self {
            nal_units: Vec::new(),
            current_nal_unit: Vec::new(),
        }
    }
}

struct GenericNalHandler {
    capacity: usize,
    nut: Option<BytesMut>,
}

impl GenericNalHandler {
    fn with_capacity(size: usize) -> Self {
        GenericNalHandler {
            capacity: size,
            nut: Some(BytesMut::with_capacity(size)),
        }
    }
}

impl NalHandler for GenericNalHandler {
    type Ctx = NalFramerContext;

    fn start(&mut self, ctx: &mut Context<Self::Ctx>, header: NalHeader) {
        self.nut = Some(BytesMut::with_capacity(self.capacity));
    }

    fn push(&mut self, ctx: &mut Context<Self::Ctx>, buf: &[u8]) {
        if let Some(ref mut nut) = &mut self.nut {
            nut.extend(buf);
        }
    }

    fn end(&mut self, ctx: &mut Context<Self::Ctx>) {
        if let Some(nut) = self.nut.take() {
            ctx.user_context.nal_units.push(nut);
        }
    }
}

struct NalFramer {
    nal_start: usize,
}

impl NalFramer {
    pub fn new() -> Self {
        Self { nal_start: 0 }
    }
}

impl NalHandler for NalFramer {
    type Ctx = NalFramerContext;

    fn start(&mut self, _ctx: &mut Context<Self::Ctx>, _header: NalHeader) {}

    fn push(&mut self, ctx: &mut Context<Self::Ctx>, buf: &[u8]) {
        ctx.user_context.current_nal_unit.extend(buf);
    }

    fn end(&mut self, ctx: &mut Context<Self::Ctx>) {
        let mut nal_unit = Vec::new();

        std::mem::swap(&mut nal_unit, &mut ctx.user_context.current_nal_unit);
        ctx.user_context
            .nal_units
            .push(BytesMut::from(&nal_unit[..]));
    }
}

pub struct BitstreamFramerFilter {
    streams: Vec<Stream>,
    stream_target_framings: Vec<(u32, BitstreamFraming)>,
    target_framing: BitstreamFraming,
    target: Box<dyn FrameWriteFilter + Send + Unpin>,
}

impl BitstreamFramerFilter {
    pub fn new(
        target_framing: BitstreamFraming,
        target: Box<dyn FrameWriteFilter + Send + Unpin>,
    ) -> Self {
        Self {
            streams: Vec::new(),
            stream_target_framings: Vec::new(),
            target_framing,
            target,
        }
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for BitstreamFramerFilter {
    async fn start(&mut self, mut streams: Vec<Stream>) -> anyhow::Result<()> {
        for stream in &mut streams {
            if let Some(bitstream_format) = stream.bitstream_format() {
                if bitstream_format != self.target_framing {
                    debug!(
                        "Converting from {:?} to {:?}",
                        bitstream_format, self.target_framing
                    );

                    stream.set_bitstream_format(self.target_framing);

                    /*debug!(
                        self.logger,
                        "Converted after {:?}",
                        stream.bitstream_format()
                    );*/
                    self.stream_target_framings
                        .push((stream.id, bitstream_format));
                }
            }
        }

        self.streams = streams.clone();

        self.target.start(streams).await?;

        Ok(())
    }

    async fn write(&mut self, mut frame: Frame) -> anyhow::Result<()> {
        if let Some(&(_, source_format)) = self
            .stream_target_framings
            .iter()
            .find(|(id, _)| *id == frame.stream.id)
        {
            frame.buffer = convert_bitstream(frame.buffer, source_format, self.target_framing);
            frame.stream = self
                .streams
                .iter()
                .find(|s| s.id == frame.stream.id)
                .unwrap()
                .clone();
        }

        self.target.write(frame).await?;

        Ok(())
    }
}
