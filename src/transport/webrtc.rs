use webrtc::{
    api::{
        interceptor_registry::register_default_interceptors,
        media_engine::{
            MediaEngine,
            MIME_TYPE_H264,
            MIME_TYPE_OPUS,
        },
        APIBuilder,
    },
    peer::{
        configuration::RTCConfiguration,
        ice::{
            ice_connection_state::{RTCIceConnectionState},
            ice_candidate::{RTCIceCandidate, RTCIceCandidateInit},
            ice_server::RTCIceServer,
        },
        peer_connection::RTCPeerConnection,
        peer_connection_state::RTCPeerConnectionState,
        sdp::session_description::RTCSessionDescription,
    },
    media::{
        track::{
            track_local::{
                track_local_static_sample::{
                    TrackLocalStaticSample,
                },
                TrackLocal,
            }
        },
        rtp::{
            rtp_codec::{
                RTCRtpCodecCapability,
            }
        }
    }
};
use webrtc_media::{io::h264_reader::H264Reader, Sample};

use interceptor::registry::Registry;
use sdp::{
    description::{
        session::{
            Origin,
            SessionDescription,
        },
        media::{
            MediaDescription,
        }
    },
};

use axum::{
    extract::{
        ws::{WebSocket, Message, WebSocketUpgrade},
        Extension,
        Path,
    },
    response::{IntoResponse},
};
use std::sync::{Arc};
use std::time::Duration;

use slog::{error, info, debug};
use bytes::Bytes;

use serde::{Serialize, Deserialize};

use crate::{FilterGraph, ContextLogger, AppData, FrameReadFilter, FrameWriteFilter, Frame, media::{Stream, CodecTypeInfo}};
use crate::media::{WaitForSyncFrameFilter, MediaFrameQueue, BitstreamFramerFilter, BitstreamFraming, nut_header, parse_bitstream, is_video_nal_unit};

pub struct WebRtcTrack {
    stream: Stream,
    track: Arc<TrackLocalStaticSample>,
}

pub struct WebRtcWriteFilter {
    logger: ContextLogger,
    peer: Arc<RTCPeerConnection>,
    tracks: Vec<WebRtcTrack>,
}

impl WebRtcWriteFilter {
    pub fn new(logger: ContextLogger, tracks: Vec<WebRtcTrack>, peer: Arc<RTCPeerConnection>) -> Self {
        WebRtcWriteFilter {
            logger,
            peer,
            tracks,
        }
    }
}

fn parse_nals_webrtc_(bitstream: &Bytes) -> Vec<Bytes> {
    use std::io::{BufReader, Cursor};

    let mut cursor = Cursor::new(&bitstream[..]);
    let reader = BufReader::new(cursor);
    let mut h264 = H264Reader::new(reader);

    let mut nals = Vec::new();

    while let Ok(nal) = h264.next_nal() {
        nals.push(nal.data.freeze());
    }

    nals
}

#[async_trait::async_trait]
impl FrameWriteFilter for WebRtcWriteFilter {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()> {
        for stream in streams {
            if let Some(mut track) = self.tracks.iter_mut().find(|t| t.stream.id == stream.id) {
                debug!(self.logger, "{:?}", stream.bitstream_format());
                debug!(self.logger, "Updating stream {}", stream.id);
                track.stream = stream;
            }
        }
        for track in &self.tracks {
            for set in track.stream.parameter_sets() {
                let data = Bytes::from(set.to_vec());
                track.track.write_sample(&Sample {
                    data,
                    duration: Duration::from_millis(0),
                    ..Default::default()
                }).await?;
            }
        }

            // tokio::time::sleep(Duration::from_millis(5000)).await;

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        if let Some(WebRtcTrack { track, .. }) = self.tracks.iter().find(|t| t.stream.id == frame.stream.id) {
            // debug!(self.logger, "Writing track sample!");

            let source = frame.stream.bitstream_format().unwrap();

            //info!(self.logger, "format={:?}", source);

            // let their_nals = parse_nals_webrtc_(&frame.buffer);
            let nals = parse_bitstream(frame.buffer, source);

            //info!(self.logger, "theirs: {}", their_nals.iter().map(|n| format!("{:?}({})", nut_header(n), n.len())).collect::<Vec<_>>().join(","));
            //info!(self.logger, "ours: {}", nals.iter().map(|n| format!("{:?}({})", nut_header(n), n.len())).collect::<Vec<_>>().join(","));

            for nal in nals.into_iter().filter(|n| is_video_nal_unit(n)) {
                track.write_sample(&Sample {
                    data: nal,
                    duration: Duration::from_millis(1),
                    ..Default::default()
                }).await?;
            }

        }

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum SignalingMessage {
    #[serde(rename = "offer")]
    Offer(RTCSessionDescription),

    #[serde(rename = "answer")]
    Answer(RTCSessionDescription),

    #[serde(rename = "new-ice-candidate")]
    NewIceCandidate(RTCIceCandidateInit),
}

fn get_codec_capabilities_from_stream(stream: Stream) -> TrackLocalStaticSample {
    match &stream.codec.properties {
        CodecTypeInfo::Video(video) => {
            TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_string(),
                    clock_rate: stream.timebase.decimal() as u32,
                    ..Default::default()
                },
                String::from("video"),
                String::from("streamhead"),
            )
        }
        CodecTypeInfo::Audio(audio) => {
            TrackLocalStaticSample::new(
                RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_OPUS.to_string(),
                    clock_rate: stream.timebase.decimal() as u32,
                    ..Default::default()
                },
                String::from("audio"),
                String::from("streamhead"),
            )
        }
    }
}

fn get_tracks_from_streams(streams: Vec<Stream>) -> Vec<WebRtcTrack> {
    let mut tracks = Vec::new();

    for stream in streams {
        tracks.push(WebRtcTrack {
            stream: stream.clone(),
            track: Arc::new(get_codec_capabilities_from_stream(stream))
        });
    }

    tracks
}

pub async fn websocket_webrtc_signalling(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    let logger = data.logger.scope();

    debug!(logger, "Received WebRTC signaling request for '{}'", stream);

    ws.on_upgrade(move |socket| {
        handle_websocket_video_response(logger, socket, stream, data.clone())
    })
}

async fn handle_websocket_video_response(
    logger: ContextLogger,
    mut socket: WebSocket,
    stream: String,
    data: Arc<AppData>)
    -> anyhow::Result<()>
{
    let frame_queue = {
        let repo = data.stream_repo.read().unwrap();
        repo.streams.get(&stream).cloned()
    };

    if let Some(mut frame_queue) = frame_queue {
        let config = RTCConfiguration {
            ice_servers: vec![RTCIceServer {
                urls: vec!["stun:stun.l.google.com:19302".to_owned()],
                ..Default::default()
            }],
            ..Default::default()
        };

        // Create a MediaEngine object to configure the supported codec
        let mut m = MediaEngine::default();
        m.register_default_codecs()?;

        let mut registry = Registry::new();

        // Use the default set of Interceptors
        registry = register_default_interceptors(registry, &mut m)?;

        // Create the API object with the MediaEngine
        let api = APIBuilder::new()
            .with_media_engine(m)
            .with_interceptor_registry(registry)
            .build();

        let peer_connection = Arc::new(api.new_peer_connection(config).await?);

        let streams = frame_queue.get_streams();
        let tracks = get_tracks_from_streams(streams.into_iter().filter(|s| s.is_video()).collect::<Vec<_>>());

        for track in &tracks {
            debug!(logger, "Adding WebRTC track for mime type '{}'", track.track.codec().mime_type);
            let rtp_sender = peer_connection.add_track(Arc::clone(&track.track) as _).await?;

            let rtcp_logger = logger.scope();
            tokio::spawn(async move {
                debug!(rtcp_logger, "Starting RTCP task");
                let mut rtcp_buf = vec![0u8; 1500];
                while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
                debug!(rtcp_logger, "Finished RTCP task");
            });
        }


        let (ice_tx, ice_rx) = async_channel::unbounded();
        let (start_tx, start_rx) = tokio::sync::oneshot::channel();

        let ice_tx = Arc::new(ice_tx);
        let blogger = Arc::new(logger.clone());
        peer_connection.on_ice_candidate(Box::new(move |candidate| {
            let ice_tx = ice_tx.clone();
            let logger = blogger.clone();

            Box::pin(async move {
                ice_tx.send(candidate).await.unwrap();
            })
        })).await;

        let mut start_tx = Some(start_tx);
        let blogger = Arc::new(logger.clone());
        peer_connection.on_ice_connection_state_change(Box::new(move |connection_state| {
            // let start_tx = start_tx.clone();
            let logger = blogger.clone();

            debug!(logger, "ICE connection state changed: {}", connection_state);

            if connection_state == RTCIceConnectionState::Connected {
                if let Some(start_tx) = start_tx.take() {
                    start_tx.send(());
                }
            }

            Box::pin(async move {})
        }))
        .await;

        let blogger = Arc::new(logger.clone());
        peer_connection.on_peer_connection_state_change(Box::new(move |state| {
            let logger = blogger.clone();

            debug!(logger, "Peer connection state has changed: {}", state);

            Box::pin(async {})
        }))
        .await;

        let offer = peer_connection.create_offer(None).await?;
        if let Err(e) = peer_connection
            .set_local_description(offer.clone())
            .await
        {
            error!(logger, "Failed to set local SDP description: {}", e);
        }

        debug!(logger, "Sending offer to remote peer");
        socket.send(Message::Text(serde_json::to_string(&SignalingMessage::Offer(offer)).unwrap())).await.unwrap();


        let blogger = logger.scope();
        let peer = peer_connection.clone();
        tokio::spawn(async move {
            start_rx.await;

            let write_filter = Box::new(WebRtcWriteFilter::new(blogger.clone(), tracks, peer));
            let write_filter = Box::new(BitstreamFramerFilter::new(blogger.clone(), BitstreamFraming::FourByteStartCode, write_filter));
            //let framer = Box::new(BitstreamFramerFilter::new(logger.clone(), BitstreamFraming::TwoByteLength, write_filter));
            let mut write_filter = Box::new(WaitForSyncFrameFilter::new(
                blogger.clone(),
                write_filter,
            ));

            let queue_receiver = frame_queue.get_receiver();
            let mut graph = FilterGraph::new(Box::new(queue_receiver), write_filter);

            debug!(blogger, "Starting WebRTC filter graph");
            if let Err(e) = graph.run().await {
                error!(blogger, "Failed to run graph: {}", e);
            }
        });

        loop {
        tokio::select! {
            Ok(Some(ice_candidate)) = ice_rx.recv() => {
                let candidate = ice_candidate.to_json().await.unwrap();
                debug!(logger, "Sending new ICE candidate to remote peer");
                socket.send(Message::Text(serde_json::to_string(&SignalingMessage::NewIceCandidate(candidate)).unwrap())).await.unwrap();
            }
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Text(data) = msg {
                    let msg: Result<SignalingMessage, _> = serde_json::from_str(&data);

                    match msg {
                        Ok(SignalingMessage::Answer(answer)) => {
                            debug!(logger, "Got answer");

                            if let Err(e) = peer_connection
                                .set_remote_description(answer)
                                .await
                            {
                                error!(logger, "Failed to set remote SDP description: {}", e);
                            }

                            debug!(logger, "Accepted remote peer answer");

                            //let answer = peer_connection.create_answer(None).await;

                            // debug!(logger, "Created answer: {:?}", answer);

                            /*match answer {
                                Ok(answer) => {
                                    debug!(logger, "Sending answer to remote peer");
                                    socket.send(Message::Text(serde_json::to_string(&SignalingMessage::Answer(answer)).unwrap())).await.unwrap();
                                },
                                Err(e) => {
                                    error!(logger, "Failed to create an answer for remote peer: {}", e);
                                }
                            }*/
                        },

                        Ok(SignalingMessage::Offer(offer)) => {
                            debug!(logger, "Got offer");

                            if let Err(e) = peer_connection
                                .set_remote_description(offer)
                                .await
                            {
                                error!(logger, "Failed to set remote SDP description: {}", e);
                            }

                            debug!(logger, "Accepted remote peer offer");

                            let answer = peer_connection.create_answer(None).await;

                            // debug!(logger, "Created answer: {:?}", answer);

                            match answer {
                                Ok(answer) => {
                                    debug!(logger, "Sending answer to remote peer");
                                    socket.send(Message::Text(serde_json::to_string(&SignalingMessage::Answer(answer)).unwrap())).await.unwrap();
                                },
                                Err(e) => {
                                    error!(logger, "Failed to create an answer for remote peer: {}", e);
                                }
                            }
                        },

                        Ok(SignalingMessage::NewIceCandidate(candidate)) => {
                            debug!(logger, "Got ICE candidate");

                            if let Err(e) = peer_connection
                                .add_ice_candidate(candidate)
                                .await
                            {
                                error!(logger, "Failed to add ICE candidate: {}", e);
                            }
                        },

                        Ok(msg) => {
                            debug!(logger, "Got signaling message: {:?}", msg);
                        },
                        Err(e) => {
                            error!(logger, "Failed to parse signaling message: {}", e);
                        }
                    }
                }
            }
            else => break
        }
        }
    } else {
        debug!(logger, "Did not find a stream at {}", stream);

    }

    debug!(logger, "Finished WebRTC signaling");

    Ok(())
}
