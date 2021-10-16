use four_cc::FourCC;

use bytes::BytesMut;

use crate::Mp4Box;
use crate::Mp4BoxError;

use super::{MovieExtendsBox, MovieHeaderBox, TrackBox};

pub struct MovieBox {
    pub mvhd: MovieHeaderBox,
    pub mvex: Option<MovieExtendsBox>,
    pub tracks: Vec<TrackBox>,
}

impl Mp4Box for MovieBox {
    const NAME: FourCC = FourCC(*b"moov");

    fn content_size(&self) -> u64 {
        let mut size = self.mvhd.size();

        if let Some(mvex) = &self.mvex {
            size += mvex.size();
        }

        for track in &self.tracks {
            size += track.size();
        }

        size
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.mvhd.write(writer)?;

        if let Some(mvex) = &self.mvex {
            mvex.write(writer)?;
        }

        for track in &self.tracks {
            track.write(writer)?;
        }

        Ok(())
    }
}
