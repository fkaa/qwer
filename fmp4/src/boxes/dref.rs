use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4BoxError;
use crate::{FullBoxHeader, Mp4Box};

use std::mem::size_of;

use super::DataEntryUrlBox;

pub struct DataReferenceBox {
    pub entries: Vec<DataEntryUrlBox>,
}

impl Mp4Box for DataReferenceBox {
    const NAME: FourCC = FourCC(*b"dref");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        let mut size = size_of::<u32>() as u64; // entry_count

        for entry in &self.entries {
            size += entry.size();
        }

        size
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 4];

        BigEndian::write_u32(&mut contents[..], self.entries.len() as _);

        writer.put_slice(&contents);

        for entry in &self.entries {
            entry.write(writer)?;
        }

        Ok(())
    }
}
