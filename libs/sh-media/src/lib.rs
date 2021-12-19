#![allow(dead_code, unused_variables)]

use async_channel::{Receiver, Sender};

use std::{fmt, sync::Arc, time::Instant};

use bytes::Bytes;

mod bitstream_framer;
mod file_writer;
mod frame_analyzer;
mod media_frame_queue;
mod tcp;
mod wait_for_sync_frame;

pub use bitstream_framer::*;
pub use file_writer::*;
pub use frame_analyzer::*;
pub use media_frame_queue::*;
pub use tcp::*;
pub use wait_for_sync_frame::*;

#[derive(Copy, Clone)]
pub struct Fraction {
    pub numerator: u32,
    pub denominator: u32,
}

impl Fraction {
    pub const fn new(numerator: u32, denominator: u32) -> Self {
        Fraction {
            numerator,
            denominator,
        }
    }

    pub fn simplify(&self) -> Fraction {
        use gcd::Gcd;

        let divisor = self.numerator.gcd(self.denominator);

        Fraction::new(self.numerator / divisor, self.denominator / divisor)
    }

    pub fn decimal(&self) -> f32 {
        self.numerator as f32 / self.denominator as f32
    }
}

impl fmt::Display for Fraction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

impl fmt::Debug for Fraction {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self)
    }
}

/// Describes how H26x NAL units are framed.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum BitstreamFraming {
    /// NAL units are prefixed with a 4 byte length integer. Used by
    /// the 'AVC1' fourcc, mainly for storage in MP4 files.
    FourByteLength,

    TwoByteLength,

    // /// NAL units are prefixed with a 3 byte start code '00 00 01'.
    // ThreeByteStartCode,
    /// NAL units are prefixed with a 4 byte start code '00 00 00 01'.
    FourByteStartCode,
}

impl BitstreamFraming {
    pub fn is_start_code(&self) -> bool {
        matches!(
            self,
            //BitstreamFraming::ThreeByteStartCode |
            BitstreamFraming::FourByteStartCode
        )
    }
}

#[derive(Clone)]
pub enum VideoCodecSpecificInfo {
    H264 {
        bitstream_format: BitstreamFraming,
        profile_indication: u8,
        profile_compatibility: u8,
        level_indication: u8,
        sps: Arc<Vec<u8>>,
        pps: Arc<Vec<u8>>,
    },
}

#[derive(Clone)]
pub struct VideoCodecInfo {
    pub width: u32,
    pub height: u32,
    pub extra: VideoCodecSpecificInfo,
}

impl VideoCodecInfo {
    pub fn parameter_sets(&self) -> Option<Vec<u8>> {
        let VideoCodecSpecificInfo::H264 {
            sps,
            pps,
            ..
        } = &self.extra;

        let nuts = [sps.as_slice(), pps.as_slice()];

        Some(frame_nal_units(&nuts[..], BitstreamFraming::FourByteLength).to_vec())
    }
}

impl fmt::Debug for VideoCodecInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.extra {
            VideoCodecSpecificInfo::H264 { sps, .. } => {
                use h264_reader::{nal::sps::SeqParameterSet, rbsp::decode_nal};

                let sps = SeqParameterSet::from_bytes(&decode_nal(&sps[1..])).unwrap();

                //dbg!(&sps);

                let aspect_ratio = sps
                    .vui_parameters
                    .as_ref()
                    .and_then(|vui| vui.aspect_ratio_info.as_ref().and_then(|a| a.get()));

                let frame_rate = sps.vui_parameters.as_ref().and_then(|vui| {
                    vui.timing_info
                        .as_ref()
                        .map(|t| Fraction::new(t.time_scale, t.num_units_in_tick))
                });

                write!(
                    f,
                    "H264 ({:?}) {:?} {}x{}",
                    sps.profile(),
                    sps.chroma_info.chroma_format,
                    self.width,
                    self.height
                )?;

                let dar = Fraction::new(self.width, self.height).simplify();

                if let Some((a, b)) = aspect_ratio {
                    write!(
                        f,
                        " [DAR {}:{} SAR {}:{}]",
                        dar.numerator, dar.denominator, a, b
                    )?;
                } else {
                    write!(f, " [DAR {}:{}]", dar.numerator, dar.denominator)?;
                }

                if let Some(fps) = frame_rate {
                    write!(
                        f,
                        " {:.3} fps",
                        fps.numerator as f32 / fps.denominator as f32
                    )?;
                }

                Ok(())
            }
        }
    }
}

// TODO improve debug formatting
#[derive(Debug, Clone)]
pub enum AudioCodecSpecificInfo {
    Aac { extra: Vec<u8> },
}

impl AudioCodecSpecificInfo {
    pub fn decoder_specific_data(&self) -> Option<Vec<u8>> {
        match self {
            Self::Aac { extra } => Some(extra.clone()),
        }
    }
}

#[derive(Clone, Debug)]
pub enum SoundType {
    Mono,
    Stereo,
}

#[derive(Clone, Debug)]
pub struct AudioCodecInfo {
    pub sample_rate: u32,
    pub sample_bpp: u32,
    pub sound_type: SoundType,
    pub extra: AudioCodecSpecificInfo,
}

#[derive(Clone)]
pub enum CodecTypeInfo {
    Video(VideoCodecInfo),
    Audio(AudioCodecInfo),
}

impl fmt::Debug for CodecTypeInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CodecTypeInfo::Video(video) => write!(f, "{:?}", video),
            CodecTypeInfo::Audio(audio) => write!(f, "{:?}", audio),
        }
    }
}

#[derive(Clone)]
pub struct CodecInfo {
    pub name: &'static str,
    pub properties: CodecTypeInfo,
}

impl CodecInfo {
    pub fn video(&self) -> Option<&VideoCodecInfo> {
        if let CodecTypeInfo::Video(video) = &self.properties {
            Some(video)
        } else {
            None
        }
    }

    pub fn audio(&self) -> Option<&AudioCodecInfo> {
        if let CodecTypeInfo::Audio(audio) = &self.properties {
            Some(audio)
        } else {
            None
        }
    }
}

impl fmt::Debug for CodecInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.properties)
    }
}

#[derive(Debug, Clone)]
pub struct Stream {
    pub id: u32,
    pub codec: Arc<CodecInfo>,
    pub timebase: Fraction,
}

impl Stream {
    /*pub fn with_bitstream_format(&self) -> Stream {
            let codec = if let CodecTypeInfo::Video(VideoCodecInfo { extra: VideoCodecSpecificInfo::H264 { mut bitstream_format, .. }, .. }) = &self.codec.properties {
                let new_codec = (*self.codec).clone();

                Arc::new(new_codec)
            } else {
                self.codec.clone()
            };

            Stream {
                id: self.id,
                codec,
                timebase: self.timebase,
            }
    }*/
    pub fn parameter_sets(&self) -> Option<Vec<u8>> {
        if let CodecTypeInfo::Video(info) = &self.codec.properties
        {
            info.parameter_sets()
        } else {
            None
        }
    }

    pub fn set_bitstream_format(&mut self, format: BitstreamFraming) {
        if let CodecTypeInfo::Video(VideoCodecInfo {
            extra:
                VideoCodecSpecificInfo::H264 {
                    ref mut bitstream_format,
                    ..
                },
            ..
        }) = Arc::make_mut(&mut self.codec).properties
        {
            *bitstream_format = format;
        }
    }

    pub fn bitstream_format(&self) -> Option<BitstreamFraming> {
        if let CodecTypeInfo::Video(VideoCodecInfo {
            extra: VideoCodecSpecificInfo::H264 {
                bitstream_format, ..
            },
            ..
        }) = self.codec.properties
        {
            Some(bitstream_format)
        } else {
            None
        }
    }

    pub fn is_video(&self) -> bool {
        matches!(self.codec.properties, CodecTypeInfo::Video(_))
    }

    pub fn is_audio(&self) -> bool {
        matches!(self.codec.properties, CodecTypeInfo::Audio(_))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum FrameDependency {
    None,
    Backwards,
    BiDirectional,
}

#[derive(Clone)]
pub struct Frame {
    pub time: MediaTime,
    pub dependency: FrameDependency,

    pub buffer: Bytes,
    pub stream: Stream,

    pub received: Instant,
}

impl Frame {
    pub fn is_keyframe(&self) -> bool {
        matches!(self.dependency, FrameDependency::None)
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Frame")
            .field("time", &format_args!("{:?}", self.time))
            .field("dependency", &format_args!("{:?}", self.dependency))
            .field("buffer", &format_args!("[u8; {}]", self.buffer.len()))
            .field("stream", &format_args!("{:?}", self.stream))
            .finish()
    }
}

#[derive(Clone)]
pub struct MediaDuration {
    pub duration: i64,
    pub timebase: Fraction,
}

impl From<MediaDuration> for chrono::Duration {
    fn from(t: MediaDuration) -> chrono::Duration {
        chrono::Duration::nanoseconds(
            (1_000_000_000f64 * (t.duration as f64 / t.timebase.denominator as f64)) as i64,
        )
    }
}

impl From<MediaDuration> for std::time::Duration {
    fn from(t: MediaDuration) -> std::time::Duration {
        std::time::Duration::from_nanos(
            (1_000_000_000f64 * (t.duration as f64 / t.timebase.denominator as f64)) as u64,
        )
    }
}

#[derive(Clone)]
pub struct MediaTime {
    pub pts: u64,
    pub dts: Option<u64>,
    pub timebase: Fraction,
}

impl std::ops::Sub for &MediaTime {
    type Output = MediaDuration;

    fn sub(self, rhs: &MediaTime) -> Self::Output {
        MediaDuration {
            duration: self.pts as i64 - rhs.pts as i64,
            timebase: self.timebase,
        }
    }
}

impl std::ops::Sub for MediaTime {
    type Output = MediaDuration;

    fn sub(self, rhs: MediaTime) -> Self::Output {
        MediaDuration {
            duration: self.pts as i64 - rhs.pts as i64,
            timebase: self.timebase,
        }
    }
}

impl fmt::Debug for MediaTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}pts ", self.pts)?;

        if let Some(dts) = self.dts {
            write!(f, "{}dts ", dts)?
        }

        write!(f, "{:?}base", self.timebase)?;

        Ok(())
    }
}

impl MediaTime {
    pub fn since(&self, rhs: &MediaTime) -> MediaDuration {
        MediaDuration {
            duration: self.pts as i64 - rhs.pts as i64,
            timebase: self.timebase,
        }
    }

    pub fn in_base(&self, new_timebase: Fraction) -> MediaTime {
        let pts = convert_timebase(self.pts, self.timebase, new_timebase);
        let dts = self
            .dts
            .map(|ts| convert_timebase(ts, self.timebase, new_timebase));

        MediaTime {
            pts,
            dts,
            timebase: new_timebase,
        }
    }
}

fn convert_timebase(time: u64, original: Fraction, new: Fraction) -> u64 {
    time * new.denominator as u64 / original.denominator as u64
}

#[test]
fn con_test() {
    assert_eq!(
        1000,
        convert_timebase(500, Fraction::new(1, 500), Fraction::new(1, 1000))
    );
}

pub struct FilterGraph {
    read: Box<dyn FrameReadFilter + Unpin + Send>,
    write: Box<dyn FrameWriteFilter + Unpin + Send>,
}

impl FilterGraph {
    pub fn new(
        read: Box<dyn FrameReadFilter + Unpin + Send>,
        write: Box<dyn FrameWriteFilter + Unpin + Send>,
    ) -> Self {
        Self { read, write }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        let streams = self.read.start().await?;

        self.write.start(streams).await?;

        loop {
            let frame = self.read.read().await?;
            // dbg!(&frame);
            self.write.write(frame).await?;
        }
    }
}

#[async_trait::async_trait]
pub trait FrameWriteFilter {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()>;
    async fn write(&mut self, frame: Frame) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait FrameReadFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>>;
    async fn read(&mut self) -> anyhow::Result<Frame>;
}

#[async_trait::async_trait]
pub trait ByteWriteFilter<T: 'static> {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()>;
    async fn write(&mut self, bytes: anyhow::Result<(bytes::Bytes, T)>) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait ByteWriteFilter2 {
    async fn start(&mut self) -> anyhow::Result<()>;
    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait ByteReadFilter {
    async fn start(&mut self) -> anyhow::Result<()>;
    async fn read(&mut self) -> anyhow::Result<Bytes>;
}

pub struct ByteStreamWriteFilter {
    tx: Sender<anyhow::Result<bytes::Bytes>>,
}

impl ByteStreamWriteFilter {
    pub fn new() -> (Self, Receiver<anyhow::Result<bytes::Bytes>>) {
        let (tx, rx) = async_channel::unbounded();

        (Self { tx }, rx)
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for ByteStreamWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn write(&mut self, data: bytes::Bytes) -> anyhow::Result<()> {
        self.tx.send(Ok(data)).await?;

        Ok(())
    }
}
