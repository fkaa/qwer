use four_cc::FourCC;

use bytes::BytesMut;

use crate::{Mp4Box, Mp4BoxError};

use super::{MovieFragmentHeaderBox, TrackFragmentBox};

pub struct MovieFragmentBox {
    pub mfhd: MovieFragmentHeaderBox,
    pub traf: TrackFragmentBox,
}

impl Mp4Box for MovieFragmentBox {
    const NAME: FourCC = FourCC(*b"moof");

    fn content_size(&self) -> u64 {
        self.mfhd.size() + self.traf.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.mfhd.write(writer)?;
        self.traf.write(writer)?;

        Ok(())
    }
}
