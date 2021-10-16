use four_cc::FourCC;

use bytes::BytesMut;

use crate::Mp4Box;
use crate::Mp4BoxError;

use super::DataReferenceBox;

pub struct DataInformationBox {
    pub dref: DataReferenceBox,
}

impl Mp4Box for DataInformationBox {
    const NAME: FourCC = FourCC(*b"dinf");

    fn content_size(&self) -> u64 {
        self.dref.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.dref.write(writer)?;

        Ok(())
    }
}
