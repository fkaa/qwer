use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct SampleSizeBox {
    pub sample_sizes: Vec<u32>,
}

impl Mp4Box for SampleSizeBox {
    const NAME: FourCC = FourCC(*b"stsz");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64
            + size_of::<u32>() as u64
            + size_of::<u32>() as u64 * self.sample_sizes.len() as u64
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.write_u32::<BigEndian>(0)?;
        v.write_u32::<BigEndian>(self.sample_sizes.len() as u32)?;

        for &size in &self.sample_sizes {
            v.write_u32::<BigEndian>(size)?;
        }

        writer.put_slice(&v);

        Ok(())
    }
}
