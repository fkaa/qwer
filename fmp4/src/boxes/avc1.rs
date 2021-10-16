use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4Box;
use crate::Mp4BoxError;

use super::AvcConfigurationBox;

use std::mem::size_of;

pub struct AvcSampleEntryBox {
    pub width: u16,
    pub height: u16,
    pub avcc: AvcConfigurationBox,
}

impl Mp4Box for AvcSampleEntryBox {
    const NAME: FourCC = FourCC(*b"avc1");

    fn content_size(&self) -> u64 {
        size_of::<u8>() as u64 * 6
            + size_of::<u16>() as u64
            + size_of::<u8>() as u64 * 16
            + size_of::<u16>() as u64
            + size_of::<u16>() as u64
            + size_of::<u32>() as u64
            + size_of::<u32>() as u64
            + size_of::<u8>() as u64 * 4
            + size_of::<u16>() as u64
            + size_of::<u8>() as u64 * 32
            + size_of::<u16>() as u64
            + size_of::<i16>() as u64
            + self.avcc.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.extend(&[0u8; 6]);
        v.write_u16::<BigEndian>(1)?;

        v.extend(&[0u8; 16]);

        v.write_u16::<BigEndian>(self.width)?;
        v.write_u16::<BigEndian>(self.height)?;
        v.write_u32::<BigEndian>(0x0048_0000)?;
        v.write_u32::<BigEndian>(0x0048_0000)?;
        v.extend(&[0u8; 4]);
        v.write_u16::<BigEndian>(1)?;
        v.extend(&[0u8; 32]);
        v.write_u16::<BigEndian>(0x0018)?;
        v.write_i16::<BigEndian>(-1)?;

        writer.put_slice(&v);

        self.avcc.write(writer)?;

        Ok(())
    }
}
