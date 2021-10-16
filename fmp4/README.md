# fmp4

## fragmented MP4 / fantastic MP4 / fabulous MP4

Crate for writing primarily fragmented MP4 files according to [ISO BMFF](https://w3c.github.io/mse-byte-stream-format-isobmff/).

# Features

[x] H.264/AVC Video
[ ] H.265/HEVC Video
[ ] AAC Audio

# Usage

```rust
use fmp4::*;

let ftyp = FileTypeBox::new(
    FourCC(*b"isom"),
    0,
    vec![FourCC(*b"isom"), FourCC(*b"iso5"), FourCC(*b"dash")],
);

let moov = MovieBox {
    mvhd: MovieHeaderBox {
        creation_time: 0,
        modification_time: 0,
        timescale: 90000,
        duration: 0,
    },
    mvex: Some(MovieExtendsBox {
        mehd: MovieExtendsHeaderBox {
            fragment_duration: 0,
        },
        trex: TrackExtendsBox {
            track_id: 1,
            default_sample_description_index: 0,
            default_sample_duration: 0,
            default_sample_size: 0,
            default_sample_flags: 0,
        },
    }),
    tracks: vec![
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
                    timescale: 90000,
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
                            entries: Vec::new(),
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
    ],
};

let mut bytes = bytes::BytesMut::with_capacity(1024);

ftyp.write(&mut bytes)?;
moov.write(&mut bytes)?;
```

# License

fmp4 is licensed under the [Mozilla Public License](LICENSE-MPL).
