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

fn get_codec_from_stream(stream: &Stream) -> anyhow::Result<rfc6381_codec::Codec> {
    use mpeg4_audio_const::AudioObjectType;
    use rfc6381_codec::{Codec, Mp4a};

    if let Some(VideoCodecSpecificInfo::H264 {
        profile_indication,
        profile_compatibility,
        level_indication,
        ..
    }) = stream.codec.video().map(|v| &v.extra)
    {
        Ok(Codec::avc1(
            *profile_indication,
            *profile_compatibility,
            *level_indication,
        ))
    } else if let Some(audio_specific) = stream
        .codec
        .audio()
        .and_then(|v| v.extra.decoder_specific_data())
    {
        let audio_object_type = audio_specific[0] >> 3;
        Ok(Codec::Mp4a(Mp4a::Mpeg4Audio {
            audio_object_type: Some(AudioObjectType::try_from(audio_object_type).unwrap()),
        }))
    } else {
        unimplemented!()
    }
}

pub async fn start_websocket_filters(
    socket: WebSocket,
    read: &mut (dyn FrameReadFilter + Unpin + Send),
) -> anyhow::Result<()> {
    let streams = read.start().await?;
    let video_codec =
        get_codec_from_stream(streams.iter().find(|s| s.is_video()).unwrap())?;

    let audio_codec =
        get_codec_from_stream(streams.iter().find(|s| s.is_audio()).unwrap())?;

    let (mut sender, mut receiver) = socket.split();
    sender.send(Message::Text(format!("{},{}", video_codec, audio_codec))).await?;

    // write
    let output_filter = WebSocketWriteFilter::new(sender);
    let fmp4_filter = Box::new(FragmentedMp4WriteFilter::new(Box::new(output_filter)));
    let write_analyzer = Box::new(FrameAnalyzerFilter::write(fmp4_filter));
    //let output_filter = FileWriteFilter::new(tokio::fs::File::create("output.mp4").await.unwrap());
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
