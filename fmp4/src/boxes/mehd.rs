use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct MovieExtendsHeaderBox {
    pub fragment_duration: u64,
}

impl Mp4Box for MovieExtendsHeaderBox {
    const NAME: FourCC = FourCC(*b"mehd");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(1, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u64>() as u64 // fragment_duration
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 8];

        BigEndian::write_u64(&mut contents[..], self.fragment_duration);

        writer.put_slice(&contents);

        Ok(())
    }
}
