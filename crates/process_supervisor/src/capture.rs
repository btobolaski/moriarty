use std::{
    future::{pending, poll_fn},
    io,
    pin::Pin,
    task::Poll,
};

use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

use crate::result::Capture;

enum Lifecycle<R> {
    Uninitialized,
    Open(R),
    Eof,
    Failed,
}

pub(crate) struct Stream<R> {
    lifecycle: Lifecycle<R>,
    pub bytes: Vec<u8>,
    pub truncated: bool,
    limit: usize,
    buffer: [u8; 8192],
}

impl<R: AsyncRead + Unpin> Stream<R> {
    pub fn allocate(limit: usize) -> io::Result<Self> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(limit).map_err(io::Error::other)?;
        Ok(Self {
            lifecycle: Lifecycle::Uninitialized,
            bytes,
            truncated: false,
            limit,
            buffer: [0; 8192],
        })
    }

    pub fn register(&mut self, reader: R) {
        self.lifecycle = Lifecycle::Open(reader);
    }

    pub fn eof(&self) -> bool {
        matches!(self.lifecycle, Lifecycle::Eof)
    }

    pub fn settled(&self) -> bool {
        matches!(self.lifecycle, Lifecycle::Eof | Lifecycle::Failed)
    }

    pub fn close_failed(&mut self) {
        self.lifecycle = Lifecycle::Failed;
    }

    pub fn into_capture(self) -> Capture {
        Capture {
            eof: self.eof(),
            bytes: self.bytes,
            truncated: self.truncated,
        }
    }

    fn retain(&mut self, read: io::Result<usize>) -> io::Result<()> {
        match read {
            Ok(0) => self.lifecycle = Lifecycle::Eof,
            Ok(n) => {
                let keep = n.min(self.limit - self.bytes.len());
                self.bytes.extend_from_slice(&self.buffer[..keep]);
                self.truncated |= keep < n;
            }
            Err(error) => {
                self.close_failed();
                return Err(error);
            }
        }
        Ok(())
    }

    pub async fn read(&mut self) -> io::Result<()> {
        let Lifecycle::Open(reader) = &mut self.lifecycle else {
            // Inactive readers cannot win a select branch; uninitialized is not EOF.
            return pending().await;
        };
        let read = reader.read(&mut self.buffer).await;
        self.retain(read)
    }

    // A zero budget still gets one nonblocking readiness check, not a timeout future
    // which may expire before polling its inner operation.
    pub async fn probe(&mut self) -> io::Result<()> {
        let Lifecycle::Open(reader) = &mut self.lifecycle else {
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

    impl<R> Stream<R> {
        pub(crate) fn is_open(&self) -> bool {
            matches!(self.lifecycle, Lifecycle::Open(_))
        }
    }

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
        stream.register(Broken);
        let error = stream.read().await.unwrap_err();
        assert_eq!(error.to_string(), "injected read failure");
        assert!(stream.settled());
        assert!(!stream.is_open());
        assert!(!stream.into_capture().eof);
    }

    #[tokio::test]
    async fn absent_and_cancelled_reads_preserve_capture() {
        let mut stream = Stream::allocate(4).unwrap();
        let (reader, mut writer) = duplex(16);
        for reader in [None, Some(reader)] {
            let present = reader.is_some();
            if let Some(reader) = reader {
                stream.register(reader);
            }
            tokio::select! {
                biased;
                result = stream.read() => panic!("unready read completed: {result:?}"),
                _ = ready(()) => {},
            }
            assert!(!stream.settled() && stream.bytes.is_empty());
            assert_eq!(stream.is_open(), present);
        }
        writer.write_all(b"abc").await.unwrap();
        stream.read().await.unwrap();
        assert_eq!(stream.bytes, b"abc");
        drop(writer);
        stream.probe().await.unwrap();
        assert!(!stream.is_open());
        assert!(stream.into_capture().eof);
    }

    #[tokio::test]
    async fn byte_bounds_cover_zero_exact_and_discard_cases() {
        for (limit, input, truncated) in [
            (0, &b"\xff\0"[..], true),
            (2, &b"\xff\0"[..], false),
            (1, &b"\xff\0"[..], true),
        ] {
            let mut stream = Stream::allocate(limit).unwrap();
            stream.register(input);
            while stream.is_open() {
                stream.read().await.unwrap();
            }
            let capture = stream.into_capture();
            assert_eq!(capture.bytes, input[..limit.min(input.len())]);
            assert_eq!(capture.truncated, truncated);
            assert!(capture.eof);
        }
    }
}
