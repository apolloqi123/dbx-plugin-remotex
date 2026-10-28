//! Shared SFTP download read helpers.

use tokio::io::{AsyncRead, AsyncReadExt};

/// Read until `buf` is full or EOF, returning the bytes actually read.
///
/// The SFTP client's `AsyncRead` impl yields one server DATA packet per poll
/// (commonly 32 KiB) while keeping its own window of requests in flight.
/// Download chunks are much larger than one packet, so a single `read()` would
/// return a short chunk and force the pump to seek again — dropping the client
/// pipeline. Filling the buffer here lets one chunk consume the whole
/// pipelined burst.
pub(crate) async fn read_filled<R: AsyncRead + Unpin>(
    source: &mut R,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let read = source.read(&mut buf[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::ReadBuf;

    /// Returns at most `max` bytes per poll, like an SFTP DATA packet.
    struct Trickle {
        data: Vec<u8>,
        pos: usize,
        max: usize,
        reads: usize,
    }

    impl AsyncRead for Trickle {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            let remaining = self.data.len() - self.pos;
            let len = remaining.min(self.max).min(buf.remaining());
            if len == 0 {
                return Poll::Ready(Ok(()));
            }
            let start = self.pos;
            let end = start + len;
            buf.put_slice(&self.data[start..end]);
            self.pos = end;
            self.reads += 1;
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn fills_the_buffer_across_short_packet_reads() {
        let mut source = Trickle { data: (0..40).collect(), pos: 0, max: 7, reads: 0 };
        let mut buf = [0_u8; 32];
        let read = read_filled(&mut source, &mut buf).await.unwrap();
        assert_eq!(read, 32);
        assert_eq!(&buf[..], &(0..32).collect::<Vec<u8>>()[..]);
        assert_eq!(source.reads, 5, "32 bytes / 7 bytes per poll");
    }

    #[tokio::test]
    async fn returns_a_short_tail_at_eof() {
        let mut source = Trickle { data: (0..10).collect(), pos: 0, max: 4, reads: 0 };
        let mut buf = [0_u8; 32];
        let read = read_filled(&mut source, &mut buf).await.unwrap();
        assert_eq!(read, 10);
        assert_eq!(&buf[..read], &(0..10).collect::<Vec<u8>>()[..]);
    }
}
