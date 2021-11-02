use std::collections::HashMap;
use std::time::Instant;

use crate::ContextLogger;

use super::{Frame, FrameReadFilter, FrameWriteFilter, MediaTime, Stream};

use slog::{trace};

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
    streams: Vec<Stream>,
    filter: ReadOrWriteFilter,
    metrics: HashMap<u32, StreamMetrics>,
}

impl FrameAnalyzerFilter {
    pub fn read(logger: ContextLogger, target: Box<dyn FrameReadFilter + Send + Unpin>) -> Self {
        Self {
            logger,
            streams: Vec::new(),
            filter: ReadOrWriteFilter::Read(target),
            metrics: HashMap::new(),
        }
    }

    pub fn write(logger: ContextLogger, target: Box<dyn FrameWriteFilter + Send + Unpin>) -> Self {
        Self {
            logger,
            streams: Vec::new(),
            filter: ReadOrWriteFilter::Write(target),
            metrics: HashMap::new(),
        }
    }

    fn report(&mut self, frame: &Frame) {
        let stream_id = frame.stream.id;

        if let Some(report) = self
            .metrics
            .entry(stream_id)
            .or_insert(StreamMetrics::new())
            .add(&frame)
        {
            if self.streams.iter().find(|s| s.id == stream_id).is_some() {
                let action = if matches!(self.filter, ReadOrWriteFilter::Read(_)) {
                    "reading"
                } else {
                    "writing"
                };

                trace!(
                    self.logger,
                    "Metrics for {} stream #{} fps: {}, std-dev: {:.3} ms, bitrate: {}/s",
                    action,
                    stream_id,
                    report.fps,
                    report.std_dev,
                    bytesize::ByteSize(report.bitrate)
                )
            }
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for FrameAnalyzerFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        let streams = self.filter.read().start().await?;

        self.streams = streams.clone();

        Ok(streams)
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        let frame = self.filter.read().read().await?;

        self.report(&frame);

        Ok(frame)
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for FrameAnalyzerFilter {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()> {
        self.streams = streams.clone();

        self.filter.write().start(streams).await?;

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        self.report(&frame);

        self.filter.write().write(frame).await?;

        Ok(())
    }
}
