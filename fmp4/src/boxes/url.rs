use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

pub struct DataEntryUrlBox {
    pub location: String,
}

impl Mp4Box for DataEntryUrlBox {
    const NAME: FourCC = FourCC(*b"url ");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0x000001))
    }

    fn content_size(&self) -> u64 {
        self.location.as_bytes().len() as u64 + 1
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.extend(self.location.as_bytes());
        v.push(0);

        assert_eq!(v.len() as u64, self.content_size());

        writer.put_slice(&v);

        Ok(())
    }
}
