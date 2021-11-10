use super::{ByteWriteFilter, ByteWriteFilter2, Stream};
use tokio::fs::File;

pub struct FileWriteFilter {
    file: File,
}

impl FileWriteFilter {
    pub fn new(file: File) -> Self {
        Self { file }
    }
}

#[async_trait::async_trait]
impl ByteWriteFilter2 for FileWriteFilter {
    async fn start(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write(&mut self, bytes: bytes::Bytes) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        self.file.write_all(&bytes).await?;
        self.file.flush().await?;

        Ok(())
    }
}

#[async_trait::async_trait]
impl<T: Send + Unpin + 'static> ByteWriteFilter<T> for FileWriteFilter {
    async fn start(&mut self, _stream: Vec<Stream>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn write(&mut self, bytes: anyhow::Result<(bytes::Bytes, T)>) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        let bytes = bytes?.0;

        self.file.write_all(&bytes).await?;
        self.file.flush().await?;

        Ok(())
    }
}
