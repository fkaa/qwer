use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use std::io::Write;
use std::mem::size_of;

use crate::{Mp4Box, Mp4BoxError};

pub struct FileTypeBox {
    major_brand: FourCC,
    minor_version: u32,
    compatible_brands: Vec<FourCC>,
}

impl FileTypeBox {
    pub fn new(major_brand: FourCC, minor_version: u32, compatible_brands: Vec<FourCC>) -> Self {
        FileTypeBox {
            major_brand,
            minor_version,
            compatible_brands,
        }
    }
}

impl Mp4Box for FileTypeBox {
    const NAME: FourCC = FourCC(*b"ftyp");

    fn content_size(&self) -> u64 {
        size_of::<u32>() as u64 + // major_brand
        size_of::<u32>() as u64 + // minor_version
        size_of::<u32>() as u64 * self.compatible_brands.len() as u64 // compatible_brands
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        Write::write_all(&mut v, &self.major_brand.0)?;
        v.write_u32::<BigEndian>(self.minor_version)?;

        for brand in &self.compatible_brands {
            Write::write_all(&mut v, &brand.0)?;
        }

        assert_eq!(self.content_size() as usize, v.len());

        writer.put_slice(&v);

        Ok(())
    }
}
