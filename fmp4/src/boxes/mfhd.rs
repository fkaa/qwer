use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

pub struct MovieFragmentHeaderBox {
    pub sequence_number: u32,
}

impl Mp4Box for MovieFragmentHeaderBox {
    const NAME: FourCC = FourCC(*b"mfhd");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        4
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 4];
        BigEndian::write_u32(&mut contents, self.sequence_number);

        writer.put_slice(&contents);

        Ok(())
    }
}
