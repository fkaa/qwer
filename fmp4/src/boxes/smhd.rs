use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct SoundMediaHeaderBox {}

impl Mp4Box for SoundMediaHeaderBox {
    const NAME: FourCC = FourCC(*b"smhd");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u16>() as u64 + // balance
        size_of::<u16>() as u64 // reserved
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let contents = [0u8; 4];

        writer.put_slice(&contents);

        Ok(())
    }
}
