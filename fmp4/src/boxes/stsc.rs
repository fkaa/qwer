use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct SampleToChunkEntry {
    first_chunk: u32,
    samples_per_chunk: u32,
    sample_description_index: u32,
}

pub struct SampleToChunkBox {
    pub entries: Vec<SampleToChunkEntry>,
}

impl Mp4Box for SampleToChunkBox {
    const NAME: FourCC = FourCC(*b"stsc");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64 + (size_of::<u32>() as u64 * 3) * self.entries.len() as u64
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.write_u32::<BigEndian>(self.entries.len() as u32)?;

        for entry in &self.entries {
            v.write_u32::<BigEndian>(entry.first_chunk)?;
            v.write_u32::<BigEndian>(entry.samples_per_chunk)?;
            v.write_u32::<BigEndian>(entry.sample_description_index)?;
        }

        writer.put_slice(&v);

        Ok(())
    }
}
