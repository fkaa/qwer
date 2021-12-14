use av_mp4::boxes::{
    co64::ChunkLargeOffsetBox,
    codec::{
        avc1::AvcSampleEntryBox,
        avcc::{
            AvcConfigurationBox, AvcDecoderConfigurationRecord, PictureParameterSet,
            SequenceParameterSet,
        },
        esds::{DecoderConfigDescriptor, DecoderSpecificInfo, EsDescriptor, EsdBox},
        mp4a::Mpeg4AudioSampleEntryBox,
        stsd::{SampleDescriptionBox, SampleEntry},
    },
    dinf::DataInformationBox,
    dref::DataReferenceBox,
    ftyp::FileTypeBox,
    hdlr::HandlerBox,
    mdat::MediaDataBox,
    mdhd::MediaHeaderBox,
    mdia::MediaBox,
    mehd::MovieExtendsHeaderBox,
    mfhd::MovieFragmentHeaderBox,
    minf::{MediaHeader, MediaInformationBox},
    moof::MovieFragmentBox,
    moov::MovieBox,
    mvex::MovieExtendsBox,
    mvhd::MovieHeaderBox,
    smhd::SoundMediaHeaderBox,
    stbl::{ChunkOffsets, SampleTableBox},
    stsc::SampleToChunkBox,
    stsz::{SampleSizeBox, SampleSizes},
    stts::TimeToSampleBox,
    tfdt::TrackFragmentBaseMediaDecodeTimeBox,
    tfhd::TrackFragmentHeaderBox,
    tkhd::{TrackHeaderBox, TrackHeaderFlags},
    traf::TrackFragmentBox,
    trak::TrackBox,
    trex::TrackExtendsBox,
    trun::{TrackFragmentRunBox, TrackFragmentSample},
    url::DataEntryUrlBox,
    vmhd::VideoMediaHeaderBox,
};

use bytes::{BytesMut, BufMut};
use sh_media::{
    ByteWriteFilter2, CodecTypeInfo, Frame, FrameDependency, FrameWriteFilter, MediaTime, Stream,
    VideoCodecSpecificInfo,
};
use std::{borrow::Cow, io::Write};

use std::collections::HashMap;

use tracing::*;

pub fn single_frame_fmp4(frame: Frame) -> anyhow::Result<Vec<u8>> {
    let mut buffer = Vec::with_capacity(frame.buffer.len());

    write_preamble(&frame.stream, None, &mut buffer)?;

    let duration = 1800;

    let mut moof = MovieFragmentBox::new(
        MovieFragmentHeaderBox::new(1),
        TrackFragmentBox::new(
            TrackFragmentHeaderBox::new(1, None, None, None, None, None, false, true),
            vec![TrackFragmentRunBox::new(
                Some(0),
                None,
                vec![TrackFragmentSample {
                    duration: Some(duration as _),
                    size: Some(frame.buffer.len() as _),
                    flags: None,
                    composition_time_offset: None,
                }],
            )],
            Some(TrackFragmentBaseMediaDecodeTimeBox::new(
                0,
            )),
        ),
    );

    // patch data offset to point to contents of our 'mdat'
    moof.traf.track_runs[0].data_offset = Some(moof.total_size() as i32 + 8);

    let mdat = MediaDataBox::new(Cow::Borrowed(&frame.buffer));

    moof.write(&mut buffer)?;
    mdat.write(&mut buffer)?;

    Ok(buffer)
}

pub struct FragmentedMp4WriteFilter {
    target: Box<dyn ByteWriteFilter2 + Send + Unpin>,
    start_times: HashMap<u32, MediaTime>,
    prev_times: HashMap<u32, MediaTime>,
    sequence_id: u32,
}

fn write_preamble(
    video: &Stream,
    audio: Option<&Stream>,
    dest: &mut dyn Write,
) -> anyhow::Result<()> {
    let ftyp = FileTypeBox::new(*b"isom", 0, Cow::Owned(vec![*b"isom", *b"iso5", *b"dash"]));

    warn!("Timescale: {:?}", video.timebase);

    let mut tracks = vec![get_track_for_video_stream(video)];
    if let Some(audio) = audio {
        tracks.push(get_track_for_audio_stream(audio))
    }

    let moov = MovieBox::new(
        MovieHeaderBox::new(video.timebase.denominator, 0),
        Some(MovieExtendsBox::new(
            MovieExtendsHeaderBox::new(0),
            vec![
                // one per track
                TrackExtendsBox::new(1, 1, 0, 0, 0),
                TrackExtendsBox::new(2, 1, 0, 0, 0),
            ],
        )),
        tracks,
    );

    ftyp.write(dest)?;
    moov.write(dest)?;

    Ok(())
}

impl FragmentedMp4WriteFilter {
    pub fn new(target: Box<dyn ByteWriteFilter2 + Send + Unpin>) -> Self {
        FragmentedMp4WriteFilter {
            target,
            start_times: HashMap::new(),
            prev_times: HashMap::new(),
            sequence_id: 0,
        }
    }

    async fn write_preamble(
        &mut self,
        video: &Stream,
        audio: Option<&Stream>,
    ) -> anyhow::Result<()> {
        let bytes = BytesMut::with_capacity(1024);
        let mut writer = bytes.writer();
        write_preamble(video, audio, &mut writer)?;

        self.target.write(writer.into_inner().freeze()).await?;

        Ok(())
    }

    async fn write_fragment_for_frame(&mut self, frame: &Frame) -> anyhow::Result<()> {
        let prev_time = self
            .prev_times
            .entry(frame.stream.id)
            .or_insert(frame.time.clone());
        let start_time = self
            .start_times
            .entry(frame.stream.id)
            .or_insert(frame.time.clone());

        let media_duration = frame.time.clone() - prev_time.clone();
        let base_offset = prev_time.clone() - start_time.clone();

        let track_id = if frame.stream.is_video() { 1 } else { 2 };

        let duration = if media_duration.duration == 0 {
            1800
        } else {
            media_duration.duration
        };

        let mut moof = MovieFragmentBox::new(
            MovieFragmentHeaderBox::new(self.sequence_id),
            TrackFragmentBox::new(
                TrackFragmentHeaderBox::new(track_id, None, None, None, None, None, false, true),
                vec![TrackFragmentRunBox::new(
                    Some(0),
                    if frame.dependency != FrameDependency::None {
                        Some(0x10000)
                    } else {
                        None
                    },
                    vec![TrackFragmentSample {
                        duration: Some(duration as _),
                        size: Some(frame.buffer.len() as _),
                        flags: None,
                        composition_time_offset: None,
                    }],
                )],
                Some(TrackFragmentBaseMediaDecodeTimeBox::new(
                    base_offset.duration as u64,
                )),
            ),
        );

        // patch data offset to point to contents of our 'mdat'
        moof.traf.track_runs[0].data_offset = Some(moof.total_size() as i32 + 8);

        let mdat = MediaDataBox::new(Cow::Borrowed(&frame.buffer));

        let bytes = BytesMut::with_capacity(frame.buffer.len() + 1024);
        let mut writer = bytes.writer();

        moof.write(&mut writer)?;
        mdat.write(&mut writer)?;

        self.target.write(writer.into_inner().freeze()).await?;

        self.sequence_id += 1;

        Ok(())
    }
}

#[async_trait::async_trait]
impl FrameWriteFilter for FragmentedMp4WriteFilter {
    async fn start(&mut self, streams: Vec<Stream>) -> anyhow::Result<()> {
        self.target.start().await?;

        let video = streams.iter().find(|s| s.is_video()).unwrap();
        let audio = streams.iter().find(|s| s.is_audio());
        self.write_preamble(video, audio).await?;

        Ok(())
    }

    async fn write(&mut self, frame: Frame) -> anyhow::Result<()> {
        let start_time = self
            .start_times
            .entry(frame.stream.id)
            .or_insert(frame.time.clone());

        let _duration: chrono::Duration = frame.time.since(start_time).into();

        //log::debug!("{}", duration);

        self.write_fragment_for_frame(&frame).await?;

        self.prev_times.insert(frame.stream.id, frame.time.clone());

        Ok(())
    }
}

fn get_sample_entry_for_codec_type(codec: &CodecTypeInfo) -> SampleEntry {
    match codec {
        CodecTypeInfo::Video(video) => {
            let VideoCodecSpecificInfo::H264 {
                bitstream_format: _,
                profile_indication,
                profile_compatibility,
                level_indication,
                ref sps,
                ref pps,
            } = video.extra;

            SampleEntry::Avc(AvcSampleEntryBox::new(
                video.width as u16,
                video.height as u16,
                AvcConfigurationBox::new(AvcDecoderConfigurationRecord {
                    profile_indication,
                    profile_compatibility,
                    level_indication,
                    sequence_parameter_sets: vec![SequenceParameterSet(sps.to_vec())],
                    picture_parameter_sets: vec![PictureParameterSet(pps.to_vec())],
                }),
            ))
        }
        CodecTypeInfo::Audio(audio) => SampleEntry::Mp4a(Mpeg4AudioSampleEntryBox::new(
            2,
            16,
            48000,
            EsdBox::new(EsDescriptor::new(
                2,
                DecoderConfigDescriptor::new(
                    0x40,
                    audio
                        .extra
                        .decoder_specific_data()
                        .map(DecoderSpecificInfo::new),
                ),
            )),
        )),
    }
}

fn get_track_for_video_stream(stream: &Stream) -> TrackBox {
    let (width, height) = stream
        .codec
        .video()
        .map(|v| (v.width, v.height))
        .unwrap_or((1920, 1080));
    TrackBox::new(
        TrackHeaderBox::new(
            TrackHeaderFlags::ENABLED | TrackHeaderFlags::IN_MOVIE,
            1, // track_id
            0,
            width.into(),
            height.into(),
        ),
        MediaBox::new(
            MediaHeaderBox::new(stream.timebase.denominator, 0),
            HandlerBox::new(*b"vide", String::from("Video Handler")),
            MediaInformationBox::new(
                MediaHeader::Video(VideoMediaHeaderBox::new()),
                DataInformationBox::new(DataReferenceBox::new(vec![DataEntryUrlBox::new(
                    String::new(),
                )])),
                SampleTableBox::new(
                    SampleDescriptionBox::new(vec![get_sample_entry_for_codec_type(
                        &stream.codec.properties,
                    )]),
                    TimeToSampleBox::new(Vec::new()),
                    SampleToChunkBox::new(Vec::new()),
                    SampleSizeBox::new(SampleSizes::Variable(Vec::new())),
                    ChunkOffsets::Co64(ChunkLargeOffsetBox::new(Vec::new())),
                    None,
                ),
            ),
        ),
    )
}

fn get_track_for_audio_stream(stream: &Stream) -> TrackBox {
    TrackBox::new(
        TrackHeaderBox::new(
            TrackHeaderFlags::ENABLED | TrackHeaderFlags::IN_MOVIE,
            2, // track_id
            0,
            0.into(),
            0.into(),
        ),
        MediaBox::new(
            MediaHeaderBox::new(stream.timebase.denominator, 0),
            HandlerBox::new(*b"soun", String::from("Audio Handler")),
            MediaInformationBox::new(
                MediaHeader::Sound(SoundMediaHeaderBox::new()),
                DataInformationBox::new(DataReferenceBox::new(vec![DataEntryUrlBox::new(
                    String::new(),
                )])),
                SampleTableBox::new(
                    SampleDescriptionBox::new(vec![get_sample_entry_for_codec_type(
                        &stream.codec.properties,
                    )]),
                    TimeToSampleBox::new(Vec::new()),
                    SampleToChunkBox::new(Vec::new()),
                    SampleSizeBox::new(SampleSizes::Variable(Vec::new())),
                    ChunkOffsets::Co64(ChunkLargeOffsetBox::new(Vec::new())),
                    None,
                ),
            ),
        ),
    )
}
