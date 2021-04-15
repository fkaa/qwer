use std::cell::RefCell;

use std::collections::{HashMap, VecDeque};

use std::sync::Arc;
use std::time::Instant;

use byteorder::{BigEndian, ByteOrder};
use bytes::{BytesMut, Bytes};
use futures_util::TryStreamExt;

use mpeg2ts_reader::{demultiplex, packet, packet_filter_switch, pes, psi, StreamType};

use async_channel::{Receiver, Sender};

use srt_tokio::SrtSocket;

use crate::media::*;
use crate::mpegts::*;

#[derive(thiserror::Error, Debug)]
pub enum SrtError {
    #[error("Encountered end of stream")]
    Eos,
}

pub struct SrtReadFilter {
    socket: SrtSocket,
}

/*async fn read_srt(mut socket: SrtSocket, sender: Sender<MpegTsPacket>) {
    let mut ctx = DumpDemuxContext::new(sender);
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);
    use std::time::Instant;

    let start = Instant::now();
    loop {
        if let Some((instant, bytes)) = socket.try_next().await.unwrap() {
            let now = Instant::now();
            let _diff = now - instant;
            let _duration = now - start;
            // println!("SRT received. diff: {:?}, duration: {:?}", diff, duration);
            ctx.instant = now;
            demux.push(&mut ctx, &bytes);
        }
    }
}*/

impl SrtReadFilter {
    pub fn new(socket: SrtSocket) -> Self {
        Self {
            socket,
        }
    }
}

#[async_trait::async_trait]
impl ByteReadFilter for SrtReadFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn read(&mut self) -> anyhow::Result<Bytes> {
        let (instant, bytes) = self.socket.try_next().await?.ok_or(SrtError::Eos)?;

        Ok(bytes)
    }
}

