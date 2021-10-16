use byteorder::{BigEndian, ByteOrder};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::FullBoxHeader;
use crate::Mp4Box;
use crate::Mp4BoxError;

use super::AvcSampleEntryBox;

use std::mem::size_of;

pub enum SampleEntry {
    Avc(AvcSampleEntryBox),
}

impl SampleEntry {
    fn size(&self) -> u64 {
        match self {
            SampleEntry::Avc(avc) => avc.size(),
        }
    }
}

pub struct SampleDescriptionBox {
    pub entries: Vec<SampleEntry>,
}

impl Mp4Box for SampleDescriptionBox {
    const NAME: FourCC = FourCC(*b"stsd");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        let mut size = size_of::<u32>() as u64;

        for entry in &self.entries {
            size += entry.size();
        }

        size
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut contents = [0u8; 4];
        BigEndian::write_u32(&mut contents, self.entries.len() as _);

        writer.put_slice(&contents);

        for entry in &self.entries {
            match entry {
                SampleEntry::Avc(avc) => avc.write(writer)?,
            }
        }

        Ok(())
    }
}
