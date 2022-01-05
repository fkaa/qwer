use std::{
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use sh_media::{Frame, FrameReadFilter, Stream};

pub struct SnapshotProviderFilter {
    filter: Box<dyn FrameReadFilter + Send + Unpin>,
    last_report: Option<Instant>,
    snapshot: Arc<RwLock<Option<Frame>>>,
}

impl SnapshotProviderFilter {
    pub fn new(
        filter: Box<dyn FrameReadFilter + Send + Unpin>,
        snapshot: Arc<RwLock<Option<Frame>>>,
    ) -> Self {
        SnapshotProviderFilter {
            filter,
            last_report: None,
            snapshot,
        }
    }

    fn provide_snapshot(&mut self, frame: &Frame) {
        if frame.is_keyframe() && frame.stream.is_video() {
            let now = Instant::now();

            match self.last_report {
                Some(prev) => {
                    if now - prev > Duration::from_secs(10) {
                        self.last_report = Some(now);
                        *self.snapshot.write().unwrap() = Some(frame.clone());
                    }
                }
                None => {
                    self.last_report = Some(now);
                    *self.snapshot.write().unwrap() = Some(frame.clone());
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for SnapshotProviderFilter {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        let streams = self.filter.start().await?;

        Ok(streams)
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        let frame = self.filter.read().await?;

        self.provide_snapshot(&frame);

        Ok(frame)
    }
}
