use byteorder::{BigEndian, WriteBytesExt};
use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use std::mem::size_of;

use crate::{FullBoxHeader, Mp4Box, Mp4BoxError};

bitflags::bitflags! {
    pub struct TrackFragmentRunFlags: u32 {
        const DATA_OFFSET_PRESENT = 0x00000001;
        const FIRST_SAMPLE_FLAGS_PRESENT = 0x00000004;
        const SAMPLE_DURATION_PRESENT = 0x00000100;
        const SAMPLE_SIZE_PRESENT = 0x00000200;
        const SAMPLE_FLAGS_PRESENT = 0x00000400;
        const SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT = 0x00000800;
    }
}

pub struct TrackFragmentSample {
    pub duration: Option<u32>,
    pub size: Option<u32>,
    pub flags: Option<u32>,
    pub composition_time_offset: Option<i32>,
}

pub struct TrackFragmentRunBox {
    pub data_offset: Option<i32>,
    pub first_sample_flags: Option<u32>,
    pub samples: Vec<TrackFragmentSample>,
}

impl TrackFragmentRunBox {
    fn sample_size(&self, flags: TrackFragmentRunFlags) -> u64 {
        let mut sample_size = 0;

        if flags.contains(TrackFragmentRunFlags::SAMPLE_DURATION_PRESENT) {
            sample_size += 4; // sample_duration
        }

        if flags.contains(TrackFragmentRunFlags::SAMPLE_SIZE_PRESENT) {
            sample_size += 4; // sample_size
        }

        if flags.contains(TrackFragmentRunFlags::SAMPLE_FLAGS_PRESENT) {
            sample_size += 4; // sample_flags
        }

        if flags.contains(TrackFragmentRunFlags::SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT) {
            sample_size += 4; // sample_composition_time_offset
        }

        sample_size
    }

    fn flags_from_fields(&self) -> TrackFragmentRunFlags {
        let mut flags = TrackFragmentRunFlags::empty();

        if self.data_offset.is_some() {
            flags.insert(TrackFragmentRunFlags::DATA_OFFSET_PRESENT);
        }

        if self.first_sample_flags.is_some() {
            flags.insert(TrackFragmentRunFlags::FIRST_SAMPLE_FLAGS_PRESENT);
        }

        if let Some(sample) = self.samples.get(0) {
            if sample.duration.is_some() {
                flags.insert(TrackFragmentRunFlags::SAMPLE_DURATION_PRESENT);
            }

            if sample.size.is_some() {
                flags.insert(TrackFragmentRunFlags::SAMPLE_SIZE_PRESENT);
            }

            if sample.flags.is_some() {
                flags.insert(TrackFragmentRunFlags::SAMPLE_FLAGS_PRESENT);
            }

            if sample.composition_time_offset.is_some() {
                flags.insert(TrackFragmentRunFlags::SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT);
            }
        }

        flags
    }
}

impl Mp4Box for TrackFragmentRunBox {
    const NAME: FourCC = FourCC(*b"trun");

    fn get_full_box_header(&self) -> Option<FullBoxHeader> {
        Some(FullBoxHeader::new(0, self.flags_from_fields().bits()))
    }

    fn content_size(&self) -> u64 {
        let flags = self.flags_from_fields();

        let mut size = 0;

        size += size_of::<u32>() as u64; // sample_count

        if flags.contains(TrackFragmentRunFlags::DATA_OFFSET_PRESENT) {
            size += size_of::<i32>() as u64; // data_offset
        }

        if flags.contains(TrackFragmentRunFlags::FIRST_SAMPLE_FLAGS_PRESENT) {
            size += size_of::<u32>() as u64; // first_sample_flags
        }

        size += self.sample_size(flags) * self.samples.len() as u64;

        size
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.write_u32::<BigEndian>(self.samples.len() as u32)?; // TODO: check for truncation

        if let Some(data_offset) = self.data_offset {
            v.write_i32::<BigEndian>(data_offset)?;
        }

        if let Some(first_sample_flags) = self.first_sample_flags {
            v.write_u32::<BigEndian>(first_sample_flags)?;
        }

        let flags = self.flags_from_fields();
        for sample in &self.samples {
            ensure_sample_fields_present(&sample, flags);

            if let Some(duration) = sample.duration {
                v.write_u32::<BigEndian>(duration)?;
            }

            if let Some(size) = sample.size {
                v.write_u32::<BigEndian>(size)?;
            }

            if let Some(flags) = sample.flags {
                v.write_u32::<BigEndian>(flags)?;
            }

            if let Some(composition_time_offset) = sample.composition_time_offset {
                v.write_i32::<BigEndian>(composition_time_offset)?;
            }
        }

        assert_eq!(v.len() as u64, self.content_size());

        writer.put_slice(&v);

        Ok(())
    }
}

fn ensure_sample_fields_present(sample: &TrackFragmentSample, flags: TrackFragmentRunFlags) {
    let duration_should_be_present = flags.contains(TrackFragmentRunFlags::SAMPLE_DURATION_PRESENT);
    let size_should_be_present = flags.contains(TrackFragmentRunFlags::SAMPLE_SIZE_PRESENT);
    let flags_should_be_present = flags.contains(TrackFragmentRunFlags::SAMPLE_FLAGS_PRESENT);
    let composition_time_offset_should_be_present =
        flags.contains(TrackFragmentRunFlags::SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT);

    let duration_is_present = sample.duration.is_some();
    let size_is_present = sample.size.is_some();
    let flags_is_present = sample.flags.is_some();
    let composition_time_offset_is_present = sample.composition_time_offset.is_some();

    // TODO: return error
    assert_eq!(duration_should_be_present, duration_is_present);
    assert_eq!(size_should_be_present, size_is_present);
    assert_eq!(flags_should_be_present, flags_is_present);
    assert_eq!(
        composition_time_offset_should_be_present,
        composition_time_offset_is_present
    );
}
