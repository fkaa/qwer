use sh_media::*;
use sh_media::{BitstreamFramerFilter, BitstreamFraming};
use sh_fmp4::FragmentedMp4WriteFilter;

use axum::extract::ws::{Message, WebSocket};
use futures::{stream::SplitSink, SinkExt, StreamExt};
use bytes::{BufMut, BytesMut};

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
        self.sink.flush().await?;

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

pub async fn start_websocket_filters(
    socket: WebSocket,
    read: &mut (dyn FrameReadFilter + Unpin + Send),
) -> anyhow::Result<()> {
    let streams = read.start().await?;
    let (profile, constraints, level) =
        get_codec_from_stream(streams.iter().find(|s| s.is_video()).unwrap())?;

    let mut framed = BytesMut::with_capacity(4);
    framed.put_u8(profile);
    framed.put_u8(constraints);
    framed.put_u8(level);

    let (mut sender, mut receiver) = socket.split();
    sender.send(Message::Binary(framed.to_vec())).await?;

    // write
    let output_filter = WebSocketWriteFilter::new(sender);
    let fmp4_filter = Box::new(FragmentedMp4WriteFilter::new(Box::new(output_filter)));
    let write_analyzer = Box::new(FrameAnalyzerFilter::write(fmp4_filter));
    let mut write = Box::new(BitstreamFramerFilter::new(
        BitstreamFraming::FourByteLength,
        write_analyzer,
    ));

    let first_frame = wait_for_sync_frame(read).await?;
    write.start(streams).await?;
    write.write(first_frame).await?;

    tokio::select! {
        res = async {
            loop {
                let frame = read.read().await?;
                write.write(frame).await?;
            }
        } => res,
        res = async {
            loop {
                if let Some(Ok(Message::Close(close))) = receiver.next().await {
                    break Err(anyhow::anyhow!("WebSocket closed: {:?}", close));
                }
            }
        } => res
    }
}
