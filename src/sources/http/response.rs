use super::{BodySink, ContentKind, Error, ResponseBuffers, Summary};
use embedded_io::{Error as _, ErrorKind, ErrorType};
use embedded_io_async::Read;

#[derive(Clone, Copy)]
pub(super) enum Framing {
    Empty,
    Fixed(usize),
    Chunked,
    Close,
}
pub(super) struct Head {
    pub status: u16,
    pub content: ContentKind,
    pub framing: Framing,
    pub loaded: usize,
    pub body_start: usize,
}

pub(super) async fn headers<R: Read>(stream: &mut R, buffer: &mut [u8]) -> Result<Head, Error> {
    let mut loaded = 0;
    let mut informational = 0;
    loop {
        let mut slots = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Response::new(&mut slots);
        if let httparse::Status::Complete(body_start) =
            parsed
                .parse(&buffer[..loaded])
                .map_err(|error| match error {
                    httparse::Error::TooManyHeaders => Error::HeadersTooLarge,
                    _ => Error::Protocol,
                })?
        {
            if !matches!(parsed.version, Some(0 | 1)) {
                return Err(Error::Protocol);
            }
            let status = parsed.code.ok_or(Error::Protocol)?;
            if (100..200).contains(&status) {
                informational += 1;
                if status == 101
                    || informational > 4
                    || parsed.headers.iter().any(|h| {
                        h.name.eq_ignore_ascii_case("content-length")
                            || h.name.eq_ignore_ascii_case("transfer-encoding")
                    })
                {
                    return Err(Error::Protocol);
                }
                buffer.copy_within(body_start..loaded, 0);
                loaded -= body_start;
                continue;
            }
            if !(200..300).contains(&status) || status == 206 {
                return Err(Error::Status(status));
            }
            let mut length = None;
            let mut chunked = false;
            let mut content = ContentKind::Unknown;
            let mut encoding = false;
            for header in parsed.headers.iter() {
                if ![
                    "content-length",
                    "transfer-encoding",
                    "content-encoding",
                    "content-type",
                ]
                .iter()
                .any(|name| header.name.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                let value = core::str::from_utf8(header.value)
                    .map_err(|_| Error::Protocol)?
                    .trim_matches([' ', '\t']);
                if header.name.eq_ignore_ascii_case("content-length") {
                    if length.is_some()
                        || value.is_empty()
                        || !value.bytes().all(|b| b.is_ascii_digit())
                    {
                        return Err(Error::Protocol);
                    }
                    length = Some(value.parse::<usize>().map_err(|_| Error::Protocol)?);
                } else if header.name.eq_ignore_ascii_case("transfer-encoding") {
                    if chunked {
                        return Err(Error::Protocol);
                    }
                    if !value.eq_ignore_ascii_case("chunked") {
                        return Err(Error::UnsupportedEncoding);
                    }
                    chunked = true;
                } else if header.name.eq_ignore_ascii_case("content-encoding") {
                    if encoding {
                        return Err(Error::Protocol);
                    }
                    if !value.eq_ignore_ascii_case("identity") {
                        return Err(Error::UnsupportedEncoding);
                    }
                    encoding = true;
                } else if header.name.eq_ignore_ascii_case("content-type") {
                    let mime = value.split(';').next().unwrap_or("").trim();
                    content = if mime.eq_ignore_ascii_case("application/json") {
                        ContentKind::Json
                    } else if mime.eq_ignore_ascii_case("text/plain") {
                        ContentKind::Text
                    } else if mime.eq_ignore_ascii_case("application/openmetrics-text") {
                        ContentKind::OpenMetrics
                    } else {
                        ContentKind::Unknown
                    };
                }
            }
            if chunked && length.is_some() {
                return Err(Error::Protocol);
            }
            let framing = if matches!(status, 204 | 205) {
                if chunked || length.is_some_and(|n| n != 0) {
                    return Err(Error::Protocol);
                }
                Framing::Empty
            } else if let Some(length) = length {
                Framing::Fixed(length)
            } else if chunked {
                Framing::Chunked
            } else {
                Framing::Close
            };
            if let Framing::Fixed(length) = framing
                && loaded - body_start > length
            {
                return Err(Error::Protocol);
            }
            return Ok(Head {
                status,
                content,
                framing,
                loaded,
                body_start,
            });
        }
        if loaded == buffer.len() {
            return Err(Error::HeadersTooLarge);
        }
        let read = stream
            .read(&mut buffer[loaded..])
            .await
            .map_err(|e| Error::Transport(e.kind()))?;
        if read == 0 {
            return Err(Error::Truncated);
        }
        loaded += read;
    }
}

struct Buffered<'a, R> {
    stream: &'a mut R,
    buffer: &'a mut [u8],
    pos: usize,
    end: usize,
}
impl<R: Read> ErrorType for Buffered<'_, R> {
    type Error = ErrorKind;
}
impl<R: Read> Read for Buffered<'_, R> {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, ErrorKind> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.pos == self.end {
            self.end = self.stream.read(self.buffer).await.map_err(|e| e.kind())?;
            self.pos = 0;
        }
        let size = (self.end - self.pos).min(out.len());
        out[..size].copy_from_slice(&self.buffer[self.pos..self.pos + size]);
        self.pos += size;
        Ok(size)
    }
}

pub(super) async fn body<R: Read, S: BodySink>(
    stream: &mut R,
    buffers: &mut ResponseBuffers,
    head: Head,
    limit: usize,
    sink: &mut S,
) -> Result<Summary, Error> {
    let ResponseBuffers {
        headers: header,
        chunk,
        line,
    } = buffers;
    let mut reader = Buffered {
        stream,
        buffer: header,
        pos: head.body_start,
        end: head.loaded,
    };
    let mut delivered = 0;
    match head.framing {
        Framing::Empty => {
            if head.loaded != head.body_start {
                return Err(Error::Protocol);
            }
        }
        Framing::Fixed(size) => {
            if size > limit {
                return Err(Error::ResponseTooLarge);
            }
            exact(&mut reader, chunk, size, sink).await?;
            delivered = size;
        }
        Framing::Close => loop {
            let max = chunk.len().min(limit - delivered + 1);
            let size = reader
                .read(&mut chunk[..max])
                .await
                .map_err(Error::Transport)?;
            if size == 0 {
                break;
            }
            if size > limit - delivered {
                return Err(Error::ResponseTooLarge);
            }
            sink.write(&chunk[..size])
                .await
                .map_err(|_| Error::Consumer)?;
            delivered += size;
        },
        Framing::Chunked => loop {
            let size_line = read_line(&mut reader, line).await?;
            let (digits, extension) = match size_line.iter().position(|&b| b == b';') {
                Some(pos) => (&size_line[..pos], Some(&size_line[pos + 1..])),
                None => (size_line, None),
            };
            if digits.is_empty()
                || digits.len() > 8
                || !digits.iter().all(u8::is_ascii_hexdigit)
                || extension.is_some_and(|ext| {
                    !ext.iter()
                        .all(|&b| b == b'\t' || (0x20..=0x7e).contains(&b))
                })
            {
                return Err(Error::Protocol);
            }
            let digits = core::str::from_utf8(digits).map_err(|_| Error::Protocol)?;
            let size = u32::from_str_radix(digits, 16).map_err(|_| Error::Protocol)? as usize;
            if size == 0 {
                let mut trailer_bytes = 0;
                let mut trailers = 0;
                loop {
                    let trailer = read_line(&mut reader, line).await?;
                    trailer_bytes += trailer.len() + 2;
                    if trailer_bytes > 2048 {
                        return Err(Error::HeadersTooLarge);
                    }
                    if trailer.is_empty() {
                        break;
                    }
                    trailers += 1;
                    if trailers > 16 {
                        return Err(Error::HeadersTooLarge);
                    }
                    let Some(colon) = trailer.iter().position(|&b| b == b':') else {
                        return Err(Error::Protocol);
                    };
                    let name = &trailer[..colon];
                    let value = &trailer[colon + 1..];
                    if name.is_empty()
                        || !name
                            .iter()
                            .all(|&b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                        || !value
                            .iter()
                            .all(|&b| b == b'\t' || (0x20..=0x7e).contains(&b))
                    {
                        return Err(Error::Protocol);
                    }
                    if name.eq_ignore_ascii_case(b"content-length")
                        || name.eq_ignore_ascii_case(b"transfer-encoding")
                        || name.eq_ignore_ascii_case(b"content-encoding")
                    {
                        return Err(Error::Protocol);
                    }
                }
                break;
            }
            if size > limit - delivered {
                return Err(Error::ResponseTooLarge);
            }
            exact(&mut reader, chunk, size, sink).await?;
            delivered += size;
            if !read_line(&mut reader, line).await?.is_empty() {
                return Err(Error::Protocol);
            }
        },
    }
    Ok(Summary {
        status: head.status,
        content_type: head.content,
        body_bytes: delivered,
    })
}

async fn exact<R: Read, S: BodySink>(
    reader: &mut R,
    chunk: &mut [u8],
    mut remaining: usize,
    sink: &mut S,
) -> Result<(), Error> {
    while remaining > 0 {
        let max = remaining.min(chunk.len());
        let size = reader
            .read(&mut chunk[..max])
            .await
            .map_err(|e| Error::Transport(e.kind()))?;
        if size == 0 {
            return Err(Error::Truncated);
        }
        sink.write(&chunk[..size])
            .await
            .map_err(|_| Error::Consumer)?;
        remaining -= size;
    }
    Ok(())
}

async fn read_line<'a, R: Read>(reader: &mut R, line: &'a mut [u8]) -> Result<&'a [u8], Error> {
    let mut count = 0;
    loop {
        if count == line.len() {
            return Err(Error::HeadersTooLarge);
        }
        let size = reader
            .read(&mut line[count..count + 1])
            .await
            .map_err(|e| Error::Transport(e.kind()))?;
        if size == 0 {
            return Err(Error::Truncated);
        }
        count += 1;
        if line[count - 1] == b'\n' {
            if count < 2 || line[count - 2] != b'\r' {
                return Err(Error::Protocol);
            }
            return Ok(&line[..count - 2]);
        }
    }
}
