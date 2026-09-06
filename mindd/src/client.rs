//! Async client for the mind socket.

use crate::proto::{Event, Request};
use anyhow::{Context, Result};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    pub async fn connect(path: &Path) -> Result<Client> {
        let stream = UnixStream::connect(path).await.with_context(|| format!("connecting to {} (is mindd running?)", path.display()))?;
        let (r, w) = stream.into_split();
        Ok(Client { reader: BufReader::new(r), writer: w })
    }

    pub async fn send(&mut self, req: &Request) -> Result<()> {
        let mut s = serde_json::to_string(req)?;
        s.push('\n');
        self.writer.write_all(s.as_bytes()).await?;
        Ok(())
    }

    /// Next event, or None when the daemon closed the connection.
    pub async fn next(&mut self) -> Result<Option<Event>> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self.reader.read_line(&mut line).await?;
            if n == 0 {
                return Ok(None);
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            return Ok(Some(serde_json::from_str(t).with_context(|| format!("bad event from daemon: {}", t))?));
        }
    }
}
