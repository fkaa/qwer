use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::{FullBoxHeader, Mp4Box, Mp4BoxError};

use std::mem::size_of;

pub struct TrackFragmentBaseMediaDecodeTimeBox {
    pub base_media_decode_time: u64,
}

impl Mp4Box for TrackFragmentBaseMediaDecodeTimeBox {
    const NAME: FourCC = FourCC(*b"tfdt");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(1, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u64>() as u64 // base_media_decode_time
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut content = [0u8; 8];
        BigEndian::write_u64(&mut content[..], self.base_media_decode_time);

        writer.put_slice(&content);

        Ok(())
    }
}
