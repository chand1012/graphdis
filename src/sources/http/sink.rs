use super::{Error, SinkError};

/// Transactional destination for decoded body chunks; methods must remain bounded.
#[allow(async_fn_in_trait)]
pub trait BodySink {
    /// Start staging a response, clearing any previous incomplete transaction.
    fn begin(&mut self);
    /// Stage bytes with optional async backpressure, covered by the request deadline.
    async fn write(&mut self, bytes: &[u8]) -> Result<(), SinkError>;
    /// Synchronously validate/finalize and publish only if complete and valid.
    fn finish(&mut self) -> Result<(), SinkError>;
    /// Discard staging after any error or cancellation. Must not panic or block.
    fn abort(&mut self);
}

/// Fixed-capacity body destination, for small responses or host diagnostics.
/// Large metrics responses should use a streaming parser sink instead.
pub struct SliceSink<'a> {
    buffer: &'a mut [u8],
    len: usize,
    committed: bool,
}

impl<'a> SliceSink<'a> {
    /// Borrow a caller-owned buffer; never allocates or grows it.
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            len: 0,
            committed: false,
        }
    }
    /// Completed response only; empty while staging or after failure/cancellation.
    pub fn bytes(&self) -> &[u8] {
        if self.committed {
            &self.buffer[..self.len]
        } else {
            &[]
        }
    }
}

impl BodySink for SliceSink<'_> {
    fn begin(&mut self) {
        self.len = 0;
        self.committed = false;
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<(), SinkError> {
        if bytes.len() > self.buffer.len() - self.len {
            return Err(SinkError);
        }
        self.buffer[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(())
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        self.committed = true;
        Ok(())
    }
    fn abort(&mut self) {
        self.len = 0;
        self.committed = false;
    }
}

pub(super) struct Transaction<'a, S: BodySink> {
    pub sink: &'a mut S,
    committed: bool,
}
impl<'a, S: BodySink> Transaction<'a, S> {
    pub fn new(sink: &'a mut S) -> Self {
        sink.begin();
        Self {
            sink,
            committed: false,
        }
    }
    pub fn commit(&mut self) -> Result<(), Error> {
        self.sink.finish().map_err(|_| Error::Consumer)?;
        self.committed = true;
        Ok(())
    }
}
impl<S: BodySink> Drop for Transaction<'_, S> {
    fn drop(&mut self) {
        if !self.committed {
            self.sink.abort();
        }
    }
}
