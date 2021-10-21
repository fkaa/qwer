use async_channel::{Receiver, Sender};

use tokio::fs::File;

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::collections::HashMap;

use bytes::Bytes;

use crate::ContextLogger;

use slog::{info, debug};

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

pub enum VideoCodecSpecificInfo {
    H264 {
        // TODO: profile etc?
        profile_indication: u8,
        profile_compatibility: u8,
        level_indication: u8,
        sps: Arc<Vec<u8>>,
        pps: Arc<Vec<u8>>,
    },
}

pub struct VideoCodecInfo {
    pub width: u32,
    pub height: u32,
    pub extra: VideoCodecSpecificInfo,
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

#[derive(Debug)]
pub enum SoundType {
    Mono,
    Stereo,
}

#[derive(Debug)]
pub struct AudioCodecInfo {
    pub sample_rate: u32,
    pub sample_bpp: u32,
    pub sound_type: SoundType,
    // pub extra: VideoCodecSpecificInfo,
}

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

    pub received: std::time::Instant,
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
    time / original.denominator as u64 * new.denominator as u64
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
        let stream = self.read.start().await?;

        self.write.start(stream).await?;

        loop {
            let frame = self.read.read().await?;
            // dbg!(&frame);
            self.write.write(frame).await?;
        }

        Ok(())
    }
}

#[async_trait::async_trait]
pub trait FrameWriteFilter {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()>;
    async fn write(&mut self, frame: Frame) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
pub trait FrameReadFilter {
    async fn start(&mut self) -> anyhow::Result<Stream>;
    async fn read(&mut self) -> anyhow::Result<Frame>;
}

#[async_trait::async_trait]
pub trait ByteWriteFilter<T: 'static> {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()>;
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

pub enum StreamMessage<T> {
    Start(Stream),
    Frame(bytes::Bytes, T),
}

pub struct StreamWriteFilter<T> {
    tx: Sender<anyhow::Result<StreamMessage<T>>>,
}

impl<T: Send + Unpin + 'static> StreamWriteFilter<T> {
    pub fn new() -> (Self, Receiver<anyhow::Result<StreamMessage<T>>>) {
        let (tx, rx) = async_channel::unbounded();

        (Self { tx }, rx)
    }
}

#[async_trait::async_trait]
impl<T: Send + Unpin + Sync + 'static> ByteWriteFilter<T> for StreamWriteFilter<T> {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()> {
        self.tx.send(Ok(StreamMessage::Start(stream))).await?;

        Ok(())
    }

    async fn write(&mut self, data: anyhow::Result<(bytes::Bytes, T)>) -> anyhow::Result<()> {
        self.tx
            .send(data.map(|(b, m)| StreamMessage::Frame(b, m)))
            .await?;

        Ok(())
    }
}

pub struct FileWriteFilter {
    file: File,
}

impl FileWriteFilter {
    pub fn new(file: File) -> Self {
        Self { file }
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for FileWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        self.file.write_all(&bytes).await?;
        self.file.flush().await?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl<T: Send + Unpin + 'static> ByteWriteFilter<T> for FileWriteFilter {
    async fn start(&mut self, _stream: Stream) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write(&mut self, bytes: anyhow::Result<(bytes::Bytes, T)>) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        let bytes = bytes?.0;

        self.file.write_all(&bytes).await?;
        self.file.flush().await?;

        Ok(())
    }
}

enum ReadOrWriteFilter {
    Read(Box<dyn FrameReadFilter + Send + Unpin>),
    Write(Box<dyn FrameWriteFilter + Send + Unpin>),
}

impl ReadOrWriteFilter {
    fn read(&mut self) -> &mut Box<dyn FrameReadFilter + Send + Unpin> {
        if let ReadOrWriteFilter::Read(read) = self {
            read
        } else {
            panic!("Tried to get read filter from write analyzer");
        }
    }

    fn write(&mut self) -> &mut Box<dyn FrameWriteFilter + Send + Unpin> {
        if let ReadOrWriteFilter::Write(write) = self {
            write
        } else {
            panic!("Tried to get write filter from read analyzer");
        }
    }
}

pub struct StreamMetrics {
    buffer: e_ring::Ring<f32, 1000>,
    index: u16,
    data_size: u64,
    last_time: Option<MediaTime>,
    last_report: Instant,
}

pub struct MetricsReport {
    std_dev: f32,
    fps: f32,
    bitrate: u64,
}

impl StreamMetrics {
    fn new() -> Self {
        StreamMetrics {
            buffer: Default::default(),
            index: 0,
            data_size: 0,
            last_time: None,
            last_report: Instant::now(),
        }
    }

    fn add(&mut self, frame: &Frame) -> Option<MetricsReport> {
        let last_time = self.last_time.as_ref().unwrap_or(&frame.time);

        let media_duration: std::time::Duration = (&frame.time - last_time).into();

        self.last_time = Some(frame.time.clone());
        self.data_size += frame.buffer.len() as u64;
        self.buffer.append(media_duration.as_secs_f32());

        self.index += 1;
        if self.index == 1000 {
            let now = Instant::now();
            let duration = now - self.last_report;

            let fps = self.buffer.avg();
            let var = self.buffer.var(Some(fps));
            let bitrate = (self.data_size as f32 / duration.as_secs_f32()) as u64;

            self.index = 0;
            self.data_size = 0;
            self.last_report = now;

            Some(MetricsReport {
                std_dev: var.sqrt() * 1000.0,
                fps: 1.0 / fps,
                bitrate,
            })

        } else {
            None
        }
    }
}

pub struct FrameAnalyzerFilter {
    logger: ContextLogger,
    stream: Option<Stream>,
    filter: ReadOrWriteFilter,
    metrics: HashMap<u32, StreamMetrics>,
}

impl FrameAnalyzerFilter {
    pub fn read(logger: ContextLogger, target: Box<dyn FrameReadFilter + Send + Unpin>) -> Self {
        Self {
            logger,
            stream: None,
            filter: ReadOrWriteFilter::Read(target),
            metrics: HashMap::new(),
        }
    }

    pub fn write(logger: ContextLogger, target: Box<dyn FrameWriteFilter + Send + Unpin>) -> Self {
        Self {
            logger,
            stream: None,
            filter: ReadOrWriteFilter::Write(target),
            metrics: HashMap::new(),
        }
    }

    fn report(&mut self, frame: &Frame) {
        let stream_id = frame.stream.id;

        if let Some(report) = self.metrics.entry(stream_id).or_insert(StreamMetrics::new()).add(&frame) {
            if let Some(_stream) = self.stream.iter().find(|s| s.id == stream_id) {

            }
            let action = if matches!(self.filter, ReadOrWriteFilter::Read(_)) { "reading" } else { "writing" };
            info!(self.logger, "Metrics for {} stream #{} fps: {}, std-dev: {:.3} ms, bitrate: {}/s", action, stream_id, report.fps, report.std_dev, bytesize::ByteSize(report.bitrate))
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for FrameAnalyzerFilter {
    async fn start(&mut self) -> anyhow::Result<Stream> {
        Ok(self.filter.read().start().await?)
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        let frame = self.filter.read().read().await?;

        self.report(&frame);

        Ok(frame)
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for FrameAnalyzerFilter {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()> {
        self.stream = Some(stream.clone());

        self.filter.write().start(stream).await?;

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        self.report(&frame);

        self.filter.write().write(frame).await?;

        Ok(())
    }
}

pub struct WaitForSyncFrameFilter {
    logger: ContextLogger,
    stream: Option<Stream>,
    target: Box<dyn FrameWriteFilter + Send + Unpin>,
    has_seen_sync_frame: bool,
    discarded: u32,
}

impl WaitForSyncFrameFilter {
    pub fn new(logger: ContextLogger, target: Box<dyn FrameWriteFilter + Send + Unpin>) -> Self {
        Self {
            logger,
            stream: None,
            target,
            has_seen_sync_frame: false,
            discarded: 0,
        }
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for WaitForSyncFrameFilter {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()> {
        self.stream = Some(stream);
        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        if let FrameDependency::None = frame.dependency {
            if !self.has_seen_sync_frame && frame.stream.is_video() {
                debug!(self.logger, "Found keyframe after discarding {} frames!", self.discarded);
                self.target.start(self.stream.take().unwrap()).await?;
                self.has_seen_sync_frame = true;
            }
        }

        if self.has_seen_sync_frame {
            self.target.write(frame).await
        } else {
            self.discarded += 1;
            Ok(())
        }
    }
}

/// A queue which broadcasts [`Frame`] to multiple readers.
#[derive(Clone)]
pub struct MediaFrameQueue {
    logger: ContextLogger,
    // FIXME: alternative to mutex here?
    targets: Arc<Mutex<Vec<async_channel::Sender<Frame>>>>,
    stream: Arc<Mutex<Option<Stream>>>,
}

impl MediaFrameQueue {
    pub fn new(logger: ContextLogger) -> Self {
        MediaFrameQueue {
            logger,
            targets: Arc::new(Mutex::new(Vec::new())),
            stream: Arc::new(Mutex::new(None)),
        }
    }

    pub fn push(&self, frame: Frame) {
        let mut targets = self.targets.lock().unwrap();

        /*let lens = targets
            .iter()
            .map(|send| send.len().to_string())
            .collect::<Vec<_>>();

        debug!("Queue buffers: {}", lens.join(","));*/

        let targets_to_remove = targets
            .iter()
            .map(|send| send.try_send(frame.clone()))
            .enumerate()
            .filter_map(|(i, r)| r.err().map(|e| (i, e)))
            .collect::<Vec<_>>();

        for (idx, result) in targets_to_remove.into_iter().rev() {
            use async_channel::TrySendError;

            match result {
                TrySendError::Full(_) => {
                    debug!(self.logger, "Closing frame queue target due to channel overflow")
                }
                TrySendError::Closed(_) => {
                    debug!(self.logger, "Closing frame queue target due to channel disconnection.")
                }
            }

            targets.remove(idx);
        }
    }

    pub fn get_receiver(&self) -> MediaFrameQueueReceiver {
        let (send, recv) = async_channel::bounded(50);

        debug!(self.logger, "Adding frame queue target");

        let mut targets = self.targets.lock().unwrap();
        targets.push(send);

        let stream = &*self.stream.lock().unwrap();

        MediaFrameQueueReceiver::new(stream.clone().unwrap(), recv)
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for MediaFrameQueue {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()> {
        *self.stream.lock().unwrap() = Some(stream);

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        self.push(frame);

        Ok(())
    }
}

/// A pull filter which reads [`MediaFrame`]s from a [`MediaFrameQueue`].
pub struct MediaFrameQueueReceiver {
    stream: Stream,
    recv: async_channel::Receiver<Frame>,
}

impl MediaFrameQueueReceiver {
    fn new(stream: Stream, recv: async_channel::Receiver<Frame>) -> Self {
        MediaFrameQueueReceiver { stream, recv }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for MediaFrameQueueReceiver {
    async fn start(&mut self) -> anyhow::Result<Stream> {
        Ok(self.stream.clone())
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        // FIXME: on buffer overflow (channel closed), raise an error to the
        //        parent filter graph

        Ok(self.recv.recv().await?)
    }
}
