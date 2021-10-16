use four_cc::FourCC;

use bytes::BytesMut;

use crate::{Mp4Box, Mp4BoxError};

use super::{TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentHeaderBox, TrackFragmentRunBox};

pub struct TrackFragmentBox {
    pub tfhd: TrackFragmentHeaderBox,
    pub track_runs: Vec<TrackFragmentRunBox>,
    pub base_media_decode_time: Option<TrackFragmentBaseMediaDecodeTimeBox>,
}

impl Mp4Box for TrackFragmentBox {
    const NAME: FourCC = FourCC(*b"traf");

    fn content_size(&self) -> u64 {
        let mut size = self.tfhd.size();

        for trun in &self.track_runs {
            size += trun.size();
        }

        if let Some(base_media_decode_time) = &self.base_media_decode_time {
            size += base_media_decode_time.size();
        }

        size
    }

    fn write_box_contents(&self, writer: &mut BytesMut) -> Result<(), Mp4BoxError> {
        self.tfhd.write(writer)?;

        if let Some(base_media_decode_time) = &self.base_media_decode_time {
            base_media_decode_time.write(writer)?;
        }

        for run in &self.track_runs {
            run.write(writer)?;
        }

        Ok(())
    }
}
