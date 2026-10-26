//! Chunk coalescing for streamed command output: `docker logs -f` emits
//! arbitrary-size fragments, and forwarding every fragment as its own sidecar
//! event would flood the host bridge. Batches flush on a byte ceiling or an
//! elapsed interval, whichever comes first.

use std::time::{Duration, Instant};

/// Flush trigger: the batch is emitted once `pending` reaches this size, but a
/// single larger incoming fragment can push a batch past it (the fragment is
/// never split).
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
            .then(|| self.take(now))
    }

    pub fn flush(&mut self) -> Option<Vec<u8>> {
        (!self.pending.is_empty()).then(|| self.take(Instant::now()))
    }

    fn take(&mut self, now: Instant) -> Vec<u8> {
        self.last_flush = now;
        std::mem::take(&mut self.pending)
    }
}

impl Default for ChunkBatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Incremental UTF-8 decoding for streamed output. Decoding each pipe read or
/// batch on its own with `from_utf8_lossy` would turn a multibyte character
/// split across the boundary into U+FFFD, so an incomplete trailing sequence
/// is carried into the next [`push`](Self::push).
#[derive(Default)]
pub struct StreamText {
    pending: Vec<u8>,
}

impl StreamText {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes the next fragment and returns the text that is complete now.
    /// It is empty when the fragment ends inside a character.
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        match std::str::from_utf8(&self.pending) {
            Ok(text) => {
                let text = text.to_string();
                self.pending.clear();
                text
            }
            // Genuinely invalid bytes: degrade exactly like `from_utf8_lossy`.
            Err(error) if error.error_len().is_some() => {
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending.clear();
                text
            }
            // Truncated at the end: hold the suffix for the next push.
            Err(error) => {
                let suffix = self.pending.split_off(error.valid_up_to());
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                self.pending = suffix;
                text
            }
        }
    }

    /// Flushes a stream that ended inside a character, lossily. `None` when
    /// nothing is pending.
    pub fn finish(&mut self) -> Option<String> {
        (!self.pending.is_empty())
            .then(|| String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned())
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
        let batch = batcher.push(b"b", now).unwrap();
        assert_eq!(batch.len(), MAX_BATCH_BYTES);
        assert_eq!(&batch[..MAX_BATCH_BYTES - 1], vec![b'a'; MAX_BATCH_BYTES - 1].as_slice());
        assert_eq!(batch[MAX_BATCH_BYTES - 1], b'b');
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

    #[test]
    fn stream_text_keeps_a_split_multibyte_character() {
        let mut text = StreamText::new();
        // "é" is C3 A9: the batch boundary falls between its two bytes.
        assert_eq!(text.push(b"caf\xC3"), "caf");
        assert_eq!(text.push(b"\xA9!"), "é!");
        assert_eq!(text.finish(), None);
    }

    #[test]
    fn stream_text_keeps_a_split_cjk_character() {
        let mut text = StreamText::new();
        // "日" (U+65E5) is E6 97 A5, split one byte at a time.
        assert_eq!(text.push(b"\xE6"), "");
        assert_eq!(text.push(b"\x97"), "");
        assert_eq!(text.push(b"\xA5\n"), "日\n");
        assert_eq!(text.finish(), None);
    }

    #[test]
    fn stream_text_replaces_invalid_bytes_and_flushes_a_dangling_tail() {
        let mut text = StreamText::new();
        assert_eq!(text.push(b"a\xFFb"), "a\u{FFFD}b");
        assert_eq!(text.push(b"\xC3"), "");
        assert_eq!(text.finish().as_deref(), Some("\u{FFFD}"));
        assert_eq!(text.finish(), None);
    }
}
