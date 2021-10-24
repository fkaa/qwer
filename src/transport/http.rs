use axum::{
    extract::{
        Extension,
        Path,
    },
    body::{StreamBody},
    http::StatusCode,
};

use slog::{debug, error};

use bytes::Bytes;

use async_channel::Receiver;

use std::sync::{Arc};

use crate::{ContextLogger, AppData};
use crate::media::*;
use crate::mp4;

pub async fn http_video(
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> Result<StreamBody<Receiver<anyhow::Result<Bytes>>>, StatusCode> {
    let logger = data.logger.scope();

    debug!(logger, "Received websocket request for '{}'", stream);

    let queue_receiver = data.stream_repo.read().unwrap().streams.get(&stream).map(|s| s.get_receiver());

    if let Some(queue_receiver) = queue_receiver {
        debug!(logger, "Found a stream at {}", stream);


        start_http_filters(&logger, queue_receiver).await.map_err(|_e| StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        debug!(logger, "Did not find a stream at {}", stream);

        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn start_http_filters(
    logger: &ContextLogger,
    read: MediaFrameQueueReceiver,
) -> anyhow::Result<StreamBody<Receiver<anyhow::Result<Bytes>>>> {
    // write
    let (output_filter, rx): (_, async_channel::Receiver<anyhow::Result<Bytes>>) = ByteStreamWriteFilter::new();
    let fmp4_filter = Box::new(mp4::FragmentedMp4WriteFilter::new(Box::new(output_filter)));
    let write_analyzer = Box::new(FrameAnalyzerFilter::write(logger.clone(), fmp4_filter));
    let framer = Box::new(BitstreamFramerFilter::new(logger.clone(), BitstreamFraming::FourByteLength, write_analyzer));
    let write_filter = WaitForSyncFrameFilter::new(
        logger.clone(),
        framer,
    );

    let mut graph = FilterGraph::new(Box::new(read), Box::new(write_filter));

    let logger = logger.clone();
    tokio::spawn(async move {
        if let Err(e) = graph.run().await {
            error!(logger, "Failed to run graph: {}", e);
        }
    });

    Ok(StreamBody::new(rx))
}
