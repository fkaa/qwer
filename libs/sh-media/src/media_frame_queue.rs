use super::{Frame, FrameReadFilter, FrameWriteFilter, Stream};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use tracing::*;

/// A queue which broadcasts [`Frame`] to multiple readers.
#[derive(Clone, Default)]
pub struct MediaFrameQueue {
    // FIXME: alternative to mutex here?
    targets: Arc<Mutex<Vec<async_channel::Sender<Frame>>>>,
    streams: Arc<Mutex<Vec<Stream>>>,
}

impl MediaFrameQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, frame: Frame) {
        let mut targets = self.targets.lock().unwrap();

        /*let lens = targets
            .iter()
            .map(|send| send.len().to_string())
            .collect::<Vec<_>>();

        debug!("Queue buffers: {}", lens.join(","));*/

        #[allow(clippy::needless_collect)]
        let targets_to_remove = targets
            .iter()
            .map(|send| send.try_send(frame.clone()))
            .enumerate()
            .filter_map(|(idx, res)| res.err().map(|e| (idx, e)))
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

    pub fn get_streams(&self) -> Vec<Stream> {
        let streams = &*self.streams.lock().unwrap();

        streams.clone()
    }

    pub fn get_receiver(&self) -> MediaFrameQueueReceiver {
        let (send, recv) = async_channel::bounded(1024);

        debug!("Adding frame queue target");

        let mut targets = self.targets.lock().unwrap();
        targets.push(send);

        let streams = &*self.streams.lock().unwrap();

        MediaFrameQueueReceiver::new(streams.clone(), recv)
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for MediaFrameQueue {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()> {
        *self.streams.lock().unwrap() = streams;

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        self.push(frame);

        Ok(())
    }
}

/// A pull filter which reads [`MediaFrame`]s from a [`MediaFrameQueue`].
pub struct MediaFrameQueueReceiver {
    streams: Vec<Stream>,
    recv: async_channel::Receiver<Frame>,
}

impl MediaFrameQueueReceiver {
    fn new(streams: Vec<Stream>, recv: async_channel::Receiver<Frame>) -> Self {
        MediaFrameQueueReceiver { streams, recv }
    }
}

#[async_trait::async_trait]
impl FrameReadFilter for MediaFrameQueueReceiver {
    async fn start(&mut self) -> anyhow::Result<Vec<Stream>> {
        Ok(self.streams.clone())
    }

    async fn read(&mut self) -> anyhow::Result<Frame> {
        // FIXME: on buffer overflow (channel closed), raise an error to the
        //        parent filter graph

        let frame = self
            .recv
            .recv()
            .await
            .context("failed to read frame from queue")?;

        Ok(frame)
    }
}
