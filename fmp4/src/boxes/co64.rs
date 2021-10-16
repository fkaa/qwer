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

pub struct ChunkLargeOffsetBox {
    pub chunk_offsets: Vec<u64>,
}

impl Mp4Box for ChunkLargeOffsetBox {
    const NAME: FourCC = FourCC(*b"co64");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64 + (size_of::<u64>() as u64) * self.chunk_offsets.len() as u64
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.write_u32::<BigEndian>(self.chunk_offsets.len() as u32)?;

        for &chunk_offset in &self.chunk_offsets {
            v.write_u64::<BigEndian>(chunk_offset)?;
        }

        writer.put_slice(&v);

        Ok(())
    }
}
