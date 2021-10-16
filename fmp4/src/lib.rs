#![allow(dead_code)]

use std::io;

use byteorder::{BigEndian, ByteOrder};
use bytes::{BufMut, BytesMut};
pub use four_cc::FourCC;

mod boxes;

pub use boxes::*;

fn get_total_box_size<B: Mp4Box + ?Sized>(boks: &B) -> u64 {
    let size = boks.content_size();

    if boks.get_full_box_header().is_some() {
        size + FullBoxHeader::SIZE + 8
    } else {
        size + 8
    }
}

fn write_box_header<B: Mp4Box + ?Sized>(header: &mut [u8], size: u64) -> usize {
    if size > u32::MAX as _ {
        BigEndian::write_u32(&mut header[..], 1);
        header[4..8].copy_from_slice(&B::NAME.0);
        BigEndian::write_u64(&mut header[8..], size);

        16
    } else {
        BigEndian::write_u32(&mut header[..], size as u32);
        header[4..8].copy_from_slice(&B::NAME.0);

        8
    }
}

fn write_full_box_header(header: &mut [u8], box_header: FullBoxHeader) -> usize {
    header[0] = box_header.version;
    BigEndian::write_u24(&mut header[1..], box_header.flags);

    4
}

#[derive(Copy, Clone)]
pub struct FullBoxHeader {
    version: u8,
    flags: u32,
}

impl FullBoxHeader {
    pub const SIZE: u64 = 4;

    pub fn new(version: u8, flags: u32) -> Self {
        FullBoxHeader { version, flags }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Mp4BoxError {
    #[error("Failed to write box: {0}")]
    IoError(#[from] io::Error),
}

/// A trait interface for a MP4 box.
pub trait Mp4Box {
    const NAME: FourCC;

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        None
    }

    fn flags(&self) -> Option<u32> {
        self.get_full_box_header().map(|h| h.flags)
    }

    /// The size of the contents of the box.
    fn content_size(&self) -> u64;

    fn size(&self) -> u64 {
        get_total_box_size::<Self>(&self)
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError>;

    fn write(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut header = [0u8; 20];

        /* println!(
            "{}: {}/{}",
            std::any::type_name::<Self>(),
            self.size(),
            self.content_size()
        ); */

        let mut size = write_box_header::<Self>(&mut header, self.size());
        if let Some(box_header) = self.get_full_box_header() {
            size += write_full_box_header(&mut header[size..], box_header);
        }

        writer.put_slice(&header[..size]);

        self.write_box_contents(writer)?;

        Ok(())
    }
}

pub struct MediaDataBox<'a> {
    pub data: &'a [u8],
}

impl<'a> Mp4Box for MediaDataBox<'a> {
    const NAME: FourCC = FourCC(*b"mdat");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, 0))
    }

    fn content_size(&self) -> u64 {
        self.data.len() as _
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        writer.put_slice(&self.data);

        Ok(())
    }
}
