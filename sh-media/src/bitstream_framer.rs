use h264_reader::{
    annexb::{AnnexBReader, NalReader},
    nal::{GenericNalSwitch, NalHandler, NalHeader, UnitType},
    Context,
};

use super::{BitstreamFraming, Frame, FrameReadFilter, FrameWriteFilter, Stream};
use bytes::{Buf, BufMut, Bytes, BytesMut};

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
    let s = GenericNalSwitch::new(NalFramer::new());
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
fn frame_nal_units_with_start_codes(nal_units: Vec<Bytes>, codes: &[u8]) -> BytesMut {
    let mut bitstream = BytesMut::new();

    for nut in nal_units {
        bitstream.extend_from_slice(codes);
        bitstream.extend_from_slice(&nut[..]);
    }

    bitstream
}

/// Frames NAL units with a length prefix before each NAL.
fn frame_nal_units_with_length<F: Fn(&mut dyn BufMut, usize)>(
    nal_units: Vec<Bytes>,
    write: F,
) -> BytesMut {
    let mut bitstream = BytesMut::new();

    for nut in nal_units {
        write(&mut bitstream, nut.len());
        bitstream.extend_from_slice(&nut[..]);
    }

    bitstream
}

/// Frame NAL units with the specified [BitstreamFraming].
pub fn frame_nal_units(nal_units: Vec<Bytes>, target: BitstreamFraming) -> BytesMut {
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
    frame_nal_units(nal_units, target).freeze()
}

pub fn is_video_nal_unit(nal: &Bytes) -> bool {
    matches!(
        NalHeader::new(nal[0]).map(|h| h.nal_unit_type()),
        Ok(UnitType::SeqParameterSet)
            | Ok(UnitType::PicParameterSet)
            | Ok(UnitType::SliceLayerWithoutPartitioningNonIdr)
            | Ok(UnitType::SliceLayerWithoutPartitioningIdr)
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
                    /*debug!(
                        self.logger,
                        "Converting from {:?} to {:?}", bitstream_format, self.target_framing
                    );*/

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
