use std::{
    future::{pending, poll_fn},
    io,
    pin::Pin,
    task::Poll,
};

use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

use crate::result::Capture;

pub(crate) struct Stream<R> {
    pub reader: Option<R>,
    pub capture: Capture,
    failed: bool,
    limit: usize,
    buffer: [u8; 8192],
}

impl<R: AsyncRead + Unpin> Stream<R> {
    pub fn allocate(limit: usize) -> io::Result<Self> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(limit).map_err(io::Error::other)?;
        Ok(Self {
            reader: None,
            capture: Capture {
                bytes,
                ..Capture::default()
            },
            failed: false,
            limit,
            buffer: [0; 8192],
        })
    }

    pub fn settled(&self) -> bool {
        self.capture.eof || self.failed
    }

    pub fn close_failed(&mut self) {
        self.reader = None;
        self.failed = true;
    }

    fn retain(&mut self, read: io::Result<usize>) -> io::Result<()> {
        match read {
            Ok(0) => {
                self.capture.eof = true;
                self.reader = None;
            }
            Ok(n) => {
                let keep = n.min(self.limit - self.capture.bytes.len());
                self.capture.bytes.extend_from_slice(&self.buffer[..keep]);
                self.capture.truncated |= keep < n;
            }
            Err(error) => {
                self.close_failed();
                return Err(error);
            }
        }
        Ok(())
    }

    pub async fn read(&mut self) -> io::Result<()> {
        let Some(reader) = self.reader.as_mut() else {
            // Absent is neither EOF nor failure; it cannot win a select branch.
            return pending().await;
        };
        let read = reader.read(&mut self.buffer).await;
        self.retain(read)
    }

    // A zero budget still gets one nonblocking readiness check, not a timeout future
    // which may expire before polling its inner operation.
    pub async fn probe(&mut self) -> io::Result<()> {
        let Some(reader) = self.reader.as_mut() else {
            return Ok(());
        };
        let mut buffer = ReadBuf::new(&mut self.buffer);
        let result =
            poll_fn(|cx| Poll::Ready(Pin::new(&mut *reader).poll_read(cx, &mut buffer))).await;
        match result {
            Poll::Ready(result) => {
                let size = buffer.filled().len();
                self.retain(result.map(|()| size))
            }
            Poll::Pending => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{future::ready, task::Context};

    use tokio::io::{AsyncWriteExt, duplex};

    use super::*;

    struct Broken;
    impl AsyncRead for Broken {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::other("injected read failure")))
        }
    }

    #[tokio::test]
    async fn failed_read_disables_stream_without_inventing_eof() {
        let mut stream = Stream::allocate(0).unwrap();
        assert!(
            !stream.settled(),
            "an uninitialized reader is not successful capture"
        );
        stream.reader = Some(Broken);
        let error = stream.read().await.unwrap_err();
        assert_eq!(error.to_string(), "injected read failure");
        assert!(stream.settled());
        assert!(stream.reader.is_none());
        assert!(!stream.capture.eof);
    }

    #[tokio::test]
    async fn absent_and_cancelled_reads_preserve_capture() {
        let mut stream = Stream::allocate(4).unwrap();
        let (reader, mut writer) = duplex(16);
        for reader in [None, Some(reader)] {
            let present = reader.is_some();
            stream.reader = reader;
            tokio::select! {
                biased;
                result = stream.read() => panic!("unready read completed: {result:?}"),
                _ = ready(()) => {},
            }
            assert!(!stream.settled() && stream.capture.bytes.is_empty());
            assert_eq!(stream.reader.is_some(), present);
        }
        writer.write_all(b"abc").await.unwrap();
        stream.read().await.unwrap();
        assert_eq!(stream.capture.bytes, b"abc");
        drop(writer);
        stream.probe().await.unwrap();
        assert!(stream.capture.eof && stream.reader.is_none());
    }

    #[tokio::test]
    async fn byte_bounds_cover_zero_exact_and_discard_cases() {
        for (limit, input, truncated) in [
            (0, &b"\xff\0"[..], true),
            (2, &b"\xff\0"[..], false),
            (1, &b"\xff\0"[..], true),
        ] {
            let mut stream = Stream::allocate(limit).unwrap();
            stream.reader = Some(input);
            while stream.reader.is_some() {
                stream.read().await.unwrap();
            }
            assert_eq!(stream.capture.bytes, input[..limit.min(input.len())]);
            assert_eq!(stream.capture.truncated, truncated);
            assert!(stream.capture.eof);
        }
    }
}
