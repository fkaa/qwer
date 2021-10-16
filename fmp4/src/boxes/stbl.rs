use four_cc::FourCC;

use bytes::BytesMut;

use crate::Mp4Box;
use crate::Mp4BoxError;

use super::{
    ChunkLargeOffsetBox, SampleDescriptionBox, SampleSizeBox, SampleToChunkBox, TimeToSampleBox,
};

pub struct SampleTableBox {
    pub stsd: SampleDescriptionBox,
    pub stts: TimeToSampleBox,
    pub stsc: SampleToChunkBox,
    pub stsz: SampleSizeBox,
    pub co64: ChunkLargeOffsetBox,
}

impl Mp4Box for SampleTableBox {
    const NAME: FourCC = FourCC(*b"stbl");

    fn content_size(&self) -> u64 {
        self.stsd.size() + self.stts.size() + self.stsc.size() + self.stsz.size() + self.co64.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.stsd.write(writer)?;
        self.stts.write(writer)?;
        self.stsc.write(writer)?;
        self.stsz.write(writer)?;
        self.co64.write(writer)?;

        Ok(())
    }
}
