use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct TrackExtendsBox {
    pub track_id: u32,
    pub default_sample_description_index: u32,
    pub default_sample_duration: u32,
    pub default_sample_size: u32,
    pub default_sample_flags: u32,
}

impl Mp4Box for TrackExtendsBox {
    const NAME: FourCC = FourCC(*b"trex");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64 + // track_ID
        size_of::<u32>() as u64 + // default_sample_description_index
        size_of::<u32>() as u64 + // default_sample_duration
        size_of::<u32>() as u64 + // default_sample_size
        size_of::<u32>() as u64 // default_sample_flags
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 20];

        BigEndian::write_u32(&mut contents[..], self.track_id);
        BigEndian::write_u32(&mut contents[4..], self.default_sample_description_index);
        BigEndian::write_u32(&mut contents[8..], self.default_sample_duration);
        BigEndian::write_u32(&mut contents[12..], self.default_sample_size);
        BigEndian::write_u32(&mut contents[16..], self.default_sample_flags);

        writer.put_slice(&contents);

        Ok(())
    }
}
