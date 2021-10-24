use crate::mp4;//::{FragmentedMp4WriteFilter, Mp4Metadata, Mp4Segment};
use crate::media::*;

use futures::{StreamExt, SinkExt, stream::SplitSink};
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension,
        Path,
    },
    response::{IntoResponse},
};

use bytes::{BufMut, BytesMut};

use slog::{debug, error};

use std::sync::Arc;

use crate::{AppData, ContextLogger};

use crate::media::{BitstreamFramerFilter, BitstreamFraming};

struct WebSocketWriteFilter {
    sink: SplitSink<WebSocket, Message>,
}

impl WebSocketWriteFilter {
    pub fn new(sink: SplitSink<WebSocket, Message>) -> Self {
        Self { sink }
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for WebSocketWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()> {
        self.sink.send(Message::Binary(bytes.to_vec())).await?;

        Ok(())
    }
}

fn get_codec_from_stream(stream: &Stream) -> anyhow::Result<(u8, u8, u8)> {
    if let Some(VideoCodecSpecificInfo::H264 {
        profile_indication,
        profile_compatibility,
        level_indication,
        ..
    }) = stream.codec.video().map(|v| &v.extra)
    {
        Ok((
            *profile_indication,
            *profile_compatibility,
            *level_indication,
        ))
    } else {
        todo!()
    }
}

pub async fn websocket_video(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    let logger = data.logger.scope();

    debug!(logger, "Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| {
        handle_websocket_video_response(logger, socket, stream, data.clone())
    })
}

async fn handle_websocket_video_response(
    logger: ContextLogger,
    socket: WebSocket,
    stream: String,
    data: Arc<AppData>)
{
    let queue_receiver = data.stream_repo.read().unwrap().streams.get(&stream).map(|s| s.get_receiver());

    if let Some(mut queue_receiver) = queue_receiver {
        debug!(logger, "Found a stream at {}", stream);


        if let Err(e) = start_websocket_filters(&logger, socket, &mut queue_receiver).await
        {
            error!(logger, "{}", e);
        }
    } else {
        debug!(logger, "Did not find a stream at {}", stream);
    }
}

pub async fn start_websocket_filters(
    logger: &ContextLogger,
    socket: WebSocket,
    read: &mut (dyn FrameReadFilter + Unpin + Send)) -> anyhow::Result<()>
{
    let streams = read.start().await?;
    let (profile, constraints, level) = get_codec_from_stream(streams.iter().find(|s| s.is_video()).unwrap())?;

    let mut framed = BytesMut::with_capacity(4);
    framed.put_u8(profile);
    framed.put_u8(constraints);
    framed.put_u8(level);

    let (mut sender, _receiver) = socket.split();
    sender.send(Message::Binary(framed.to_vec())).await?;

    // write
    let output_filter = WebSocketWriteFilter::new(sender);
    let fmp4_filter = Box::new(mp4::FragmentedMp4WriteFilter::new(Box::new(output_filter)));
    let write_analyzer = Box::new(FrameAnalyzerFilter::write(logger.clone(), fmp4_filter));
    let framer = Box::new(BitstreamFramerFilter::new(logger.clone(), BitstreamFraming::FourByteLength, write_analyzer));
    let mut write_filter = WaitForSyncFrameFilter::new(
        logger.clone(),
        framer,
    );

    write_filter.start(streams).await?;

    loop {
        let frame = read.read().await?;
        write_filter.write(frame).await?;
    }

    Ok(())
}
