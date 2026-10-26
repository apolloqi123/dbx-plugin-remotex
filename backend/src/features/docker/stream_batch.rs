//! Chunk coalescing for streamed command output: `docker logs -f` emits
//! arbitrary-size fragments, and forwarding every fragment as its own sidecar
//! event would flood the host bridge. Batches flush on a byte ceiling or an
//! elapsed interval, whichever comes first.

// The docker log stream registry (next task) is the first consumer.
#![allow(dead_code)]

use std::time::{Duration, Instant};

pub const MAX_BATCH_BYTES: usize = 8 * 1024;
pub const BATCH_INTERVAL: Duration = Duration::from_millis(100);

pub struct ChunkBatcher {
    pending: Vec<u8>,
    last_flush: Instant,
}

impl ChunkBatcher {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            last_flush: Instant::now(),
        }
    }

    pub fn push(&mut self, bytes: &[u8], now: Instant) -> Option<Vec<u8>> {
        self.pending.extend_from_slice(bytes);
        (self.pending.len() >= MAX_BATCH_BYTES
            || now.duration_since(self.last_flush) >= BATCH_INTERVAL)
            .then(|| self.take())
    }

    pub fn flush(&mut self) -> Option<Vec<u8>> {
        (!self.pending.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> Vec<u8> {
        self.last_flush = Instant::now();
        std::mem::take(&mut self.pending)
    }
}

impl Default for ChunkBatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn flushes_at_byte_limit() {
        let mut batcher = ChunkBatcher::new();
        let now = Instant::now();
        assert!(batcher.push(&vec![b'a'; MAX_BATCH_BYTES - 1], now).is_none());
        assert_eq!(batcher.push(b"b", now).unwrap().len(), MAX_BATCH_BYTES);
    }

    #[test]
    fn flushes_after_interval() {
        let mut batcher = ChunkBatcher::new();
        let now = Instant::now();
        assert!(batcher.push(b"a", now).is_none());
        assert!(batcher.push(b"b", now + BATCH_INTERVAL - Duration::from_millis(1)).is_none());
        assert_eq!(batcher.push(b"c", now + BATCH_INTERVAL).unwrap(), b"abc");
    }

    #[test]
    fn flush_returns_none_when_empty() {
        assert!(ChunkBatcher::new().flush().is_none());
    }
}
