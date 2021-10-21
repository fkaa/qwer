use fmp4::*;

use crate::media::{
    ByteWriteFilter2, CodecTypeInfo, Fraction, Frame, FrameDependency, FrameWriteFilter,
    MediaDuration, MediaTime, Stream, VideoCodecSpecificInfo,
};
use std::time::Instant;

use log::warn;

pub struct Mp4Segment {
    start: MediaDuration,
    duration: MediaDuration,
    is_keyframe: bool,
}

pub struct FragmentedMp4WriteFilter {
    target: Box<dyn ByteWriteFilter2 + Send + Unpin>,
    start_time: Option<MediaTime>,
    prev_time: Option<MediaTime>,
    time_scale: Option<Fraction>,
    sequence_id: u32,
}

impl FragmentedMp4WriteFilter {
    pub fn new(target: Box<dyn ByteWriteFilter2 + Send + Unpin>) -> Self {
        FragmentedMp4WriteFilter {
            target,
            start_time: None,
            prev_time: None,
            sequence_id: 0,
            time_scale: None,
        }
    }

    async fn write_preamble(&mut self, stream: &Stream) -> anyhow::Result<()> {
        let ftyp = FileTypeBox::new(
            FourCC(*b"isom"),
            0,
            vec![FourCC(*b"isom"), FourCC(*b"iso5"), FourCC(*b"dash")],
        );

        warn!("Timescale: {:?}", stream.timebase);

        let moov = MovieBox {
            mvhd: MovieHeaderBox {
                creation_time: 0,
                modification_time: 0,
                timescale: stream.timebase.denominator,
                duration: 0,
            },
            mvex: Some(MovieExtendsBox {
                mehd: MovieExtendsHeaderBox {
                    fragment_duration: 0,
                },
                trex: TrackExtendsBox {
                    track_id: 1,
                    default_sample_description_index: 1,
                    default_sample_duration: 0,
                    default_sample_size: 0,
                    default_sample_flags: 0,
                },
            }),
            tracks: vec![get_track_for_stream(stream)],
        };

        let mut bytes = bytes::BytesMut::with_capacity(1024);

        ftyp.write(&mut bytes)?;
        moov.write(&mut bytes)?;

        self.target.write(bytes.freeze()).await?;

        Ok(())
    }

    async fn write_fragment_for_frame(&mut self, frame: &Frame) -> anyhow::Result<()> {
        let media_duration = frame.time.clone() - self.prev_time.clone().unwrap();
        let base_offset = self.prev_time.clone().unwrap() - self.start_time.clone().unwrap();

        //println!("duration: {:?}", base_offset.duration);
        //dbg!(&duration.duration);

        let duration = if media_duration.duration == 0 {
            1800
        } else {
            media_duration.duration
        };

        let mut moof = MovieFragmentBox {
            mfhd: MovieFragmentHeaderBox {
                sequence_number: self.sequence_id,
            },
            traf: TrackFragmentBox {
                tfhd: TrackFragmentHeaderBox {
                    track_id: 1,
                    base_data_offset: None,
                    sample_description_index: None,
                    default_sample_duration: None,
                    default_sample_size: None,
                    default_sample_flags: None,
                    duration_is_empty: false,
                    default_base_is_moof: true,
                },
                track_runs: vec![TrackFragmentRunBox {
                    data_offset: Some(0),
                    first_sample_flags: if frame.dependency != FrameDependency::None {
                        Some(0x10000)
                    } else {
                        None
                    },
                    samples: vec![TrackFragmentSample {
                        duration: Some(duration as _),
                        size: Some(frame.buffer.len() as _),
                        flags: None,
                        composition_time_offset: None,
                    }],
                }],
                base_media_decode_time: Some(TrackFragmentBaseMediaDecodeTimeBox {
                    base_media_decode_time: base_offset.duration as _,
                }),
            },
        };

        // patch data offset to point to contents of our 'mdat'
        moof.traf.track_runs[0].data_offset = Some(moof.size() as i32 + 12);

        let mdat = MediaDataBox {
            data: &frame.buffer,
        };

        let mut bytes = bytes::BytesMut::with_capacity(1024);

        moof.write(&mut bytes)?;
        mdat.write(&mut bytes)?;

        let _segment = Mp4Segment {
            start: base_offset,
            duration: media_duration,
            is_keyframe: frame.dependency == FrameDependency::None,
        };

        self.target.write(bytes.freeze()).await?;
        let _now = Instant::now();
        //println!("frame delivery took: {:?}", now - frame.received);

        self.sequence_id += 1;

        Ok(())
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for FragmentedMp4WriteFilter {
    async fn start(&mut self, stream: Stream) -> anyhow::Result<()> {
        self.target.start().await?;

        self.write_preamble(&stream).await?;

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        if !frame.stream.is_video() {
            return Ok(());
        }

        if self.start_time.is_none() {
            self.start_time = Some(frame.time.clone());
            self.prev_time = Some(frame.time.clone());
        }

        let _duration: chrono::Duration =
            (frame.time.since(self.start_time.as_ref().unwrap())).into();

        //log::debug!("{}", duration);

        self.write_fragment_for_frame(&frame).await?;

        self.prev_time = Some(frame.time.clone());

        Ok(())
    }
}

fn get_sample_entry_for_codec_type(codec: &CodecTypeInfo) -> SampleEntry {
    match codec {
        CodecTypeInfo::Video(video) => {
            if let VideoCodecSpecificInfo::H264 {
                profile_indication,
                profile_compatibility,
                level_indication,
                ref sps,
                ref pps,
            } = video.extra
            {
                SampleEntry::Avc(AvcSampleEntryBox {
                    width: video.width as _,
                    height: video.height as _,
                    avcc: AvcConfigurationBox {
                        config: AvcDecoderConfigurationRecord {
                            profile_indication,
                            profile_compatibility,
                            level_indication,
                            sequence_parameter_set: sps.to_vec(),
                            picture_parameter_set: pps.to_vec(),
                        },
                    },
                })
            } else {
                todo!()
            }
        }
        _ => todo!(),
    }
}

fn get_track_for_stream(stream: &Stream) -> TrackBox {
    TrackBox {
        tkhd: TrackHeaderBox {
            creation_time: 0,
            modification_time: 0,
            track_id: 1,
            duration: 0,
        },
        mdia: MediaBox {
            mdhd: MediaHeaderBox {
                creation_time: 0,
                modification_time: 0,
                timescale: stream.timebase.denominator,
                duration: 0,
            },
            hdlr: HandlerBox {
                handler_type: 0x76696465,
                name: String::from("Video Handler"),
            },
            minf: MediaInformationBox {
                media_header: MediaHeader::Video(VideoMediaHeaderBox {}),
                dinf: DataInformationBox {
                    dref: DataReferenceBox {
                        entries: vec![DataEntryUrlBox {
                            location: String::from(""),
                        }],
                    },
                },
                stbl: SampleTableBox {
                    stsd: SampleDescriptionBox {
                        entries: vec![get_sample_entry_for_codec_type(&stream.codec.properties)],
                    },
                    stts: TimeToSampleBox {
                        entries: Vec::new(),
                    },
                    stsc: SampleToChunkBox {
                        entries: Vec::new(),
                    },
                    stsz: SampleSizeBox {
                        sample_sizes: Vec::new(),
                    },
                    co64: ChunkLargeOffsetBox {
                        chunk_offsets: Vec::new(),
                    },
                },
            },
        },
    }
}
