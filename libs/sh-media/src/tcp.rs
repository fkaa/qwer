use crate::{ByteReadFilter, ByteWriteFilter2};

use bytes::Bytes;
use std::io;
use tokio::net::{tcp, TcpStream};

pub fn split_tcp_filters(socket: TcpStream, buffer: usize) -> (TcpReadFilter, TcpWriteFilter) {
    let (read, write) = socket.into_split();

    (TcpReadFilter::new(read, buffer), TcpWriteFilter::new(write))
}

pub struct TcpWriteFilter {
    write: tcp::OwnedWriteHalf,
}

impl TcpWriteFilter {
    pub fn new(write: tcp::OwnedWriteHalf) -> Self {
        Self { write }
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for TcpWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        let _ = self.write.write(&bytes).await?;

        Ok(())
    }
}

pub struct TcpReadFilter {
    socket: tcp::OwnedReadHalf,
    size: usize,
    buf: Vec<u8>,
}

impl TcpReadFilter {
    pub fn new(socket: tcp::OwnedReadHalf, size: usize) -> Self {
        Self {
            socket,
            size,
            buf: vec![0; size],
        }
    }
}

#[async_trait::async_trait]
impl ByteReadFilter for TcpReadFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn read(&mut self) -> anyhow::Result<Bytes> {
        use tokio::io::AsyncReadExt;

        loop {
            match self.socket.read(&mut self.buf).await {
                Ok(n) => {
                    if n == 0 {
                        return Err(anyhow::anyhow!("EOS!"));
                    }

                    return Ok(Bytes::copy_from_slice(&self.buf[..n]));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                Err(e) => {
                    return Err(e.into());
                }
            }
        }
    }
}
