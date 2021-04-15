use async_channel::{Receiver, Sender};

use tokio::fs::File;

use std::fmt;
use std::sync::{Arc, Mutex};

use bytes::Bytes;

use log::debug;

#[derive(Copy, Clone)]
pub struct Fraction {
    pub numerator: u32,
    pub denominator: u32,
}

impl Fraction {
    pub fn new(numerator: u32, denominator: u32) -> Self {
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

pub enum CodecTypeInfo {
    Video(VideoCodecInfo),
}

impl fmt::Debug for CodecTypeInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CodecTypeInfo::Video(video) => write!(f, "{:?}", video),
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

#[derive(Clone)]
pub struct MediaTime {
    pub pts: u64,
    pub dts: Option<u64>,
    pub timebase: Fraction,
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

// use futures::stream::Stream;

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
pub trait ByteReadFilter {
    async fn start(&mut self) -> anyhow::Result<()>;
    async fn read(&mut self) -> anyhow::Result<Bytes>;
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

pub struct WaitForSyncFrameFilter {
    target: Box<dyn FrameWriteFilter + Send + Unpin>,
    has_seen_sync_frame: bool,
}

impl WaitForSyncFrameFilter {
    pub fn new(target: Box<dyn FrameWriteFilter + Send + Unpin>) -> Self {
        Self {
            target,
            has_seen_sync_frame: false,
        }
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for WaitForSyncFrameFilter {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()> {
        Ok(self.target.start(stream).await?)
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        if let FrameDependency::None = frame.dependency {
            self.has_seen_sync_frame = true;
        }

        if self.has_seen_sync_frame {
            self.target.write(frame).await
        } else {
            debug!("discarding!");
            Ok(())
        }
    }
}

/// A queue which broadcasts [`Frame`] to multiple readers.
#[derive(Clone)]
pub struct MediaFrameQueue {
    // FIXME: alternative to mutex here?
    targets: Arc<Mutex<Vec<async_channel::Sender<Frame>>>>,
    stream: Arc<Mutex<Option<Stream>>>,
}

impl MediaFrameQueue {
    pub fn new() -> Self {
        MediaFrameQueue {
            targets: Arc::new(Mutex::new(Vec::new())),
            stream: Arc::new(Mutex::new(None)),
        }
    }

    pub fn push(&self, frame: Frame) {
        let mut targets = self.targets.lock().unwrap();

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
                    debug!("Closing frame queue target due to channel overflow")
                }
                TrySendError::Closed(_) => {
                    debug!("Closing frame queue target due to channel disconnection.")
                }
            }

            targets.remove(idx);
        }
    }

    pub fn get_receiver(&self) -> MediaFrameQueueReceiver {
        let (send, recv) = async_channel::bounded(50);

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
