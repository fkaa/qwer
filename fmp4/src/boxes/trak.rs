use four_cc::FourCC;

use bytes::BytesMut;

use crate::Mp4Box;
use crate::Mp4BoxError;

use super::{MediaBox, TrackHeaderBox};

pub struct TrackBox {
    pub tkhd: TrackHeaderBox,
    pub mdia: MediaBox,
}

impl Mp4Box for TrackBox {
    const NAME: FourCC = FourCC(*b"trak");

    fn content_size(&self) -> u64 {
        self.tkhd.size() + self.mdia.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.tkhd.write(writer)?;
        self.mdia.write(writer)?;

        Ok(())
    }
}
