use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

pub struct HandlerBox {
    pub handler_type: u32,
    pub name: String,
}

impl Mp4Box for HandlerBox {
    const NAME: FourCC = FourCC(*b"hdlr");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64 + // pre_defined
        size_of::<u32>() as u64 + // handler_type
        size_of::<u32>() as u64 * 3 + // reserved
        self.name.as_bytes().len() as u64 + // name
        1
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.write_u32::<BigEndian>(0)?;
        v.write_u32::<BigEndian>(self.handler_type)?;
        v.write_u32::<BigEndian>(0)?;
        v.write_u32::<BigEndian>(0)?;
        v.write_u32::<BigEndian>(0)?;
        v.extend(self.name.as_bytes());
        v.push(0);

        assert_eq!(v.len() as u64, self.content_size());

        writer.put_slice(&v);

        Ok(())
    }
}
