use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct MediaHeaderBox {
    pub creation_time: u64,
    pub modification_time: u64,
    pub timescale: u32,
    pub duration: u64,
}

impl Mp4Box for MediaHeaderBox {
    const NAME: FourCC = FourCC(*b"mdhd");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(1, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u64>() as u64 + // creation_time
        size_of::<u64>() as u64 + // modification_time
        size_of::<u32>() as u64 + // timescale
        size_of::<u64>() as u64 + // duration
        size_of::<u16>() as u64 + // language
        size_of::<u16>() as u64 // pre_defined
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 32];

        BigEndian::write_u64(&mut contents[..], self.creation_time);
        BigEndian::write_u64(&mut contents[8..], self.modification_time);
        BigEndian::write_u32(&mut contents[16..], self.timescale);
        BigEndian::write_u64(&mut contents[20..], self.duration);

        BigEndian::write_u16(&mut contents[28..], 0); // language

        writer.put_slice(&contents);

        Ok(())
    }
}
