use byteorder::{BigEndian, WriteBytesExt, ReadBytesExt};

use four_cc::FourCC;

use bytes::{BufMut, BytesMut};

use crate::Mp4Box;
use crate::Mp4BoxError;

use std::mem::size_of;
use std::io::Read;

pub struct AvcConfigurationBox {
    pub config: AvcDecoderConfigurationRecord,
}

impl Mp4Box for AvcConfigurationBox {
    const NAME: FourCC = FourCC(*b"avcC");

    fn content_size(&self) -> u64 {
        self.config.size()
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.config.write(writer)?;

        Ok(())
    }
}

pub struct AvcDecoderConfigurationRecord {
    pub profile_indication: u8,
    pub profile_compatibility: u8,
    pub level_indication: u8,
    pub sequence_parameter_set: Vec<u8>,
    pub picture_parameter_set: Vec<u8>,
}

impl AvcDecoderConfigurationRecord {
    pub fn read(buf: &mut dyn Read) -> Result<Self, Mp4BoxError> {
        let mut header = [0u8; 6];
        buf.read_exact(&mut header)?;

        let _length_minus_one = header[4];
        let sps_count = header[5] & 0b0001_1111;

        let mut sequence_parameter_sets = Vec::new();
        let mut picture_parameter_sets = Vec::new();

        for _i in 0..sps_count {
            let sps_len = buf.read_u16::<BigEndian>()?;
            let mut sps = vec![0u8; sps_len as usize];

            buf.read_exact(&mut sps)?;

            sequence_parameter_sets.push(sps);
        }

        let pps_count = buf.read_u8()?;

        for _i in 0..pps_count {
            let pps_len = buf.read_u16::<BigEndian>()?;
            let mut pps = vec![0u8; pps_len as usize];

            buf.read_exact(&mut pps)?;

            picture_parameter_sets.push(pps);
        }

        Ok(AvcDecoderConfigurationRecord {
            profile_indication: header[1],
            profile_compatibility: header[2],
            level_indication: header[3],
            sequence_parameter_set: sequence_parameter_sets.into_iter().next().unwrap(),
            picture_parameter_set: picture_parameter_sets.into_iter().next().unwrap(),
        })
    }

    fn size(&self) -> u64 {
        size_of::<u8>() as u64 // configurationVersion
            + size_of::<u8>() as u64 // AVCProfileIndication
            + size_of::<u8>() as u64 // profile_compatibility
            + size_of::<u8>() as u64 // AVCLevelIndication
            + size_of::<u8>() as u64 // lengthSizeMinusOne
            + size_of::<u8>() as u64 // numOfSequenceParameterSets
            + size_of::<u16>() as u64 // sequenceParameterSetLength
            + self.sequence_parameter_set.len() as u64
            + size_of::<u8>() as u64 // numOfPictureParameterSets
            + size_of::<u16>() as u64 // pictureParameterSetLength
            + self.picture_parameter_set.len() as u64
    }

    fn write(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        let mut v = Vec::new();

        v.push(1);
        v.push(self.profile_indication);
        v.push(self.profile_compatibility);
        v.push(self.level_indication);
        v.push(0b1111_1100 | 3);

        v.push(0b1110_0000 | 1);
        v.write_u16::<BigEndian>(self.sequence_parameter_set.len() as u16)?;
        v.extend(&self.sequence_parameter_set);

        v.push(1);
        v.write_u16::<BigEndian>(self.picture_parameter_set.len() as u16)?;
        v.extend(&self.picture_parameter_set);

        assert_eq!(self.size(), v.len() as u64);

        writer.put_slice(&v);

        Ok(())
    }
}
