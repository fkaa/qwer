use four_cc::FourCC;

use bytes::BytesMut;

use crate::{Mp4Box, Mp4BoxError};

use super::{MovieExtendsHeaderBox, TrackExtendsBox};

pub struct MovieExtendsBox {
    pub mehd: MovieExtendsHeaderBox,
    pub trex: TrackExtendsBox,
}

impl Mp4Box for MovieExtendsBox {
    const NAME: FourCC = FourCC(*b"mvex");

    fn content_size(&self) -> u64 {
        self.mehd.size() + self.trex.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.mehd.write(writer)?;
        self.trex.write(writer)?;

        Ok(())
    }
}
