use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct TimeToSampleEntry {
    count: u32,
    delta: u32,
}

pub struct TimeToSampleBox {
    pub entries: Vec<TimeToSampleEntry>,
}

impl Mp4Box for TimeToSampleBox {
    const NAME: FourCC = FourCC(*b"stts");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64
            + (size_of::<u32>() as u64 + size_of::<u32>() as u64) * self.entries.len() as u64
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.write_u32::<BigEndian>(self.entries.len() as _)?;

        for entry in &self.entries {
            v.write_u32::<BigEndian>(entry.count)?;
            v.write_u32::<BigEndian>(entry.delta)?;
        }

        writer.put_slice(&v);

        Ok(())
    }
}
