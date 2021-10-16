use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

bitflags::bitflags! {
    pub struct TrackHeaderFlags: u32 {
        const ENABLED = 0x000001;
        const IN_MOVIE = 0x000002;
        const IN_PREVIEW = 0x000004;
        const SIZE_IS_ASPECT_RATIO = 0x000008;
    }
}

pub struct TrackHeaderBox {
    pub creation_time: u64,
    pub modification_time: u64,
    pub track_id: u32,
    pub duration: u64,
}

impl Mp4Box for TrackHeaderBox {
    const NAME: FourCC = FourCC(*b"tkhd");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        let flags = TrackHeaderFlags::ENABLED | TrackHeaderFlags::IN_MOVIE;
        Some(FullBoxHeader::new(1, flags.bits())) // TODO: flags matter
    }

    fn content_size(&self) -> u64 {
        size_of::<u64>() as u64 + // creation_time
        size_of::<u64>() as u64 + // modification_time
        size_of::<u32>() as u64 + // track_ID
        size_of::<u32>() as u64 + // reserved
        size_of::<u64>() as u64 + // duration
        size_of::<u32>() as u64 * 2 + // reserved
        size_of::<u16>() as u64 + // layer
        size_of::<u16>() as u64 + // alternate_group
        size_of::<u16>() as u64 + // volume
        size_of::<u16>() as u64 + // reserved
        size_of::<i32>() as u64 * 9 + // matrix
        size_of::<u32>() as u64 + // width
        size_of::<u32>() as u64 // height
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 92];

        BigEndian::write_u64(&mut contents[..], self.creation_time);
        BigEndian::write_u64(&mut contents[8..], self.modification_time);
        BigEndian::write_u32(&mut contents[16..], self.track_id);
        BigEndian::write_u64(&mut contents[24..], self.duration);

        BigEndian::write_i32(&mut contents[44..], 0); // volume

        BigEndian::write_i32(&mut contents[48..], 0x00010000);
        BigEndian::write_i32(&mut contents[64..], 0x00010000);
        BigEndian::write_i32(&mut contents[80..], 0x40000000);

        BigEndian::write_i32(&mut contents[84..], 1920 << 16); // width
        BigEndian::write_i32(&mut contents[88..], 1080 << 16); // height

        writer.put_slice(&contents);

        Ok(())
    }
}
