use qw_proto::stream_info::stream_reply::StreamStats;
use tokio::sync::broadcast::{Sender};

use std::time::{Duration, Instant};

use sh_media::{Frame, FrameReadFilter, Stream};

pub struct BandwidthAnalyzerFilter {
    filter: Box<dyn FrameReadFilter + Send + Unpin>,
    send: Sender<StreamStats>,
    last_report: Instant,
    is_ingest: bool,
    stream_id: i32,
    bytes: u32,
}

impl BandwidthAnalyzerFilter {
    pub fn new(
        filter: Box<dyn FrameReadFilter + Send + Unpin>,
        stream_id: i32,
        is_ingest: bool,
        send: Sender<StreamStats>,
    ) -> Self {
        BandwidthAnalyzerFilter {
            filter,
            send,
            last_report: Instant::now(),
            is_ingest,
            stream_id,
            bytes: 0,
        }
    }

    fn analyze(&mut self, frame: &Frame) {
        let now = Instant::now();

        self.bytes += frame.buffer.len() as u32;

        if now - self.last_report > Duration::from_secs(5) {
            if let Ok(_) = self.send.send(StreamStats {
                stream_session_id: self.stream_id,
                bytes_since_last_stats: self.bytes,
                is_ingest: self.is_ingest,
            }) {
                self.bytes = 0;
            }

            self.last_report = now;
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for BandwidthAnalyzerFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        let streams = self.filter.start().await?;

        Ok(streams)
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        let frame = self.filter.read().await?;

        self.analyze(&frame);

        Ok(frame)
    }
}
