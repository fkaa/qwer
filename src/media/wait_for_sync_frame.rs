use async_channel::{Receiver, Sender};

use tokio::fs::File;

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::collections::HashMap;

use bytes::Bytes;

use crate::ContextLogger;

use super::{Frame, FrameDependency, Stream, FrameWriteFilter, FrameReadFilter};

use slog::{info, debug};

pub struct WaitForSyncFrameFilter {
    logger: ContextLogger,
    streams: Vec<Stream>,
    target: Box<dyn FrameWriteFilter + Send + Unpin>,
    has_seen_sync_frame: bool,
    discarded: u32,
}

impl WaitForSyncFrameFilter {
    pub fn new(logger: ContextLogger, target: Box<dyn FrameWriteFilter + Send + Unpin>) -> Self {
        Self {
            logger,
            streams: Vec::new(),
            target,
            has_seen_sync_frame: false,
            discarded: 0,
        }
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for WaitForSyncFrameFilter {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()> {
        self.streams = streams.clone();
        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        if let FrameDependency::None = frame.dependency {
            if !self.has_seen_sync_frame && frame.stream.is_video() {
                debug!(self.logger, "Found keyframe after discarding {} frames!", self.discarded);
                self.target.start(self.streams.clone()).await?;
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
