use four_cc::FourCC;

use bytes::BytesMut;

use crate::Mp4Box;
use crate::Mp4BoxError;

use super::{HandlerBox, MediaHeaderBox, MediaInformationBox};

pub struct MediaBox {
    pub mdhd: MediaHeaderBox,
    pub hdlr: HandlerBox,
    pub minf: MediaInformationBox,
}

impl Mp4Box for MediaBox {
    const NAME: FourCC = FourCC(*b"mdia");

    fn content_size(&self) -> u64 {
        self.mdhd.size() + self.hdlr.size() + self.minf.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.mdhd.write(writer)?;
        self.hdlr.write(writer)?;
        self.minf.write(writer)?;

        Ok(())
    }
}
