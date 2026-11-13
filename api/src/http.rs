//! A small blocking HTTP/1.1 client: enough for the Docker Engine API over its Unix socket and for QTS's CGI
//! server on loopback. One request per connection (`Connection: close`).

use std::io::{self, BufRead, Read};

/// Status line and headers of an answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Head {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl Head {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn chunked(&self) -> bool {
        self.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    }

    fn content_length(&self) -> Option<u64> {
        self.header("content-length").and_then(|v| v.trim().parse().ok())
    }
}

const MAX_HEAD: usize = 64 * 1024;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Reads one line ending in `\n` (at most `budget` bytes), without the line end.
fn read_line<R: BufRead>(r: &mut R, budget: &mut usize) -> io::Result<String> {
    let mut line = Vec::new();
    let n = r.by_ref().take(*budget as u64 + 1).read_until(b'\n', &mut line)?;
    if n == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
    }
    if n > *budget {
        return Err(bad("header too long"));
    }
    *budget -= n;
    if line.last() != Some(&b'\n') {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed"));
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(String::from_utf8_lossy(&line).into_owned())
}

/// Reads the status line and headers. Interim `1xx` answers are skipped.
pub fn read_head<R: BufRead>(r: &mut R) -> io::Result<Head> {
    let mut budget = MAX_HEAD;
    loop {
        let status_line = read_line(r, &mut budget)?;
        let mut parts = status_line.split_whitespace();
        let proto = parts.next().unwrap_or("");
        if !proto.starts_with("HTTP/") {
            return Err(bad("not an HTTP answer"));
        }
        let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| bad("bad status line"))?;
        let mut headers = Vec::new();
        loop {
            let line = read_line(r, &mut budget)?;
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        if (100..200).contains(&status) {
            continue;
        }
        return Ok(Head { status, headers });
    }
}

/// The body of an answer, framed by `Transfer-Encoding: chunked`, `Content-Length` or the end of the connection.
pub enum Body<R> {
    Chunked(Chunked<R>),
    Fixed(io::Take<R>),
    Eof(R),
}

impl<R: BufRead> Body<R> {
    pub fn new(head: &Head, r: R) -> Self {
        if head.status == 204 || head.status == 304 {
            return Body::Fixed(r.take(0));
        }
        if head.chunked() {
            return Body::Chunked(Chunked::new(r));
        }
        match head.content_length() {
            Some(n) => Body::Fixed(r.take(n)),
            None => Body::Eof(r),
        }
    }

    /// Reads the whole body, failing when it is longer than `limit`.
    pub fn read_all(mut self, limit: usize) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        (&mut self).take(limit as u64 + 1).read_to_end(&mut out)?;
        if out.len() > limit {
            return Err(bad("answer too large"));
        }
        Ok(out)
    }
}

impl<R: BufRead> Read for Body<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Body::Chunked(c) => c.read(buf),
            Body::Fixed(t) => {
                let want = t.limit();
                let n = t.read(buf)?;
                if n == 0 && want > 0 && !buf.is_empty() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "answer cut short"));
                }
                Ok(n)
            }
            Body::Eof(r) => r.read(buf),
        }
    }
}

enum ChunkState {
    Size,
    Data(u64),
    DataEnd,
    Done,
}

/// Decodes `Transfer-Encoding: chunked` as it is read.
pub struct Chunked<R> {
    inner: R,
    state: ChunkState,
}

impl<R: BufRead> Chunked<R> {
    pub fn new(inner: R) -> Self {
        Chunked { inner, state: ChunkState::Size }
    }
}

impl<R: BufRead> Read for Chunked<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.state {
                ChunkState::Done => return Ok(0),
                ChunkState::Size => {
                    let mut budget = 1024;
                    let line = read_line(&mut self.inner, &mut budget)?;
                    let size = line.split(';').next().unwrap_or("").trim();
                    let n = u64::from_str_radix(size, 16).map_err(|_| bad("bad chunk size"))?;
                    if n == 0 {
                        // Trailers, up to the empty line.
                        let mut budget = MAX_HEAD;
                        while !read_line(&mut self.inner, &mut budget)?.is_empty() {}
                        self.state = ChunkState::Done;
                        return Ok(0);
                    }
                    self.state = ChunkState::Data(n);
                }
                ChunkState::Data(left) => {
                    if buf.is_empty() {
                        return Ok(0);
                    }
                    let want = buf.len().min(left.min(usize::MAX as u64) as usize);
                    let n = self.inner.read(&mut buf[..want])?;
                    if n == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "chunk cut short"));
                    }
                    self.state = if left == n as u64 { ChunkState::DataEnd } else { ChunkState::Data(left - n as u64) };
                    return Ok(n);
                }
                ChunkState::DataEnd => {
                    let mut budget = 2;
                    if !read_line(&mut self.inner, &mut budget)?.is_empty() {
                        return Err(bad("missing chunk end"));
                    }
                    self.state = ChunkState::Size;
                }
            }
        }
    }
}

/// An HTTP request. `http10` sends HTTP/1.0 (QTS's `_thttpd_` answers that best).
pub fn request(method: &str, path: &str, host: &str, headers: &[(&str, &str)], body: Option<&[u8]>, http10: bool) -> Vec<u8> {
    let proto = if http10 { "HTTP/1.0" } else { "HTTP/1.1" };
    let mut req = format!("{method} {path} {proto}\r\nHost: {host}\r\nUser-Agent: hbmgr/{}\r\n", crate::VERSION);
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    } else if method == "POST" || method == "PUT" {
        req.push_str("Content-Length: 0\r\n");
    }
    req.push_str("Connection: close\r\n\r\n");
    let mut out = req.into_bytes();
    if let Some(b) = body {
        out.extend_from_slice(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    fn parse(raw: &[u8]) -> (Head, Vec<u8>) {
        let mut r = BufReader::new(raw);
        let head = read_head(&mut r).unwrap();
        let body = Body::new(&head, r).read_all(1 << 20).unwrap();
        (head, body)
    }

    #[test]
    fn reads_a_fixed_length_answer() {
        let (head, body) = parse(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}trailing");
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-type"), Some("application/json"));
        assert_eq!(body, b"{}");
    }

    #[test]
    fn decodes_chunks_with_extensions_and_trailers() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5;x=y\r\nhello\r\n7\r\n, world\r\n0\r\nX-Trailer: 1\r\n\r\n";
        let (_, body) = parse(raw);
        assert_eq!(body, b"hello, world");
    }

    #[test]
    fn chunked_reads_in_small_pieces() {
        let raw: &[u8] = b"3\r\nabc\r\n10\r\n0123456789abcdef\r\n0\r\n\r\n";
        let mut c = Chunked::new(BufReader::new(raw));
        let mut out = Vec::new();
        let mut buf = [0u8; 2];
        loop {
            let n = c.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        assert_eq!(out, b"abc0123456789abcdef");
    }

    #[test]
    fn broken_chunks_are_errors() {
        let mut c = Chunked::new(BufReader::new(&b"zz\r\nabc"[..]));
        assert!(c.read_to_end(&mut Vec::new()).is_err());
        let mut c = Chunked::new(BufReader::new(&b"5\r\nab"[..]));
        assert!(c.read_to_end(&mut Vec::new()).is_err());
    }

    #[test]
    fn skips_interim_answers_and_reads_to_the_end() {
        let (head, body) = parse(b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.0 200 OK\r\nServer: x\r\n\r\n<xml/>");
        assert_eq!(head.status, 200);
        assert_eq!(body, b"<xml/>");
    }

    #[test]
    fn no_content_has_no_body() {
        let (head, body) = parse(b"HTTP/1.1 204 No Content\r\nDate: x\r\n\r\n");
        assert_eq!(head.status, 204);
        assert!(body.is_empty());
    }

    #[test]
    fn a_short_body_is_an_error() {
        let mut r = BufReader::new(&b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc"[..]);
        let head = read_head(&mut r).unwrap();
        assert!(Body::new(&head, r).read_all(100).is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(read_head(&mut BufReader::new(&b"SSH-2.0-OpenSSH\r\n\r\n"[..])).is_err());
        assert!(read_head(&mut BufReader::new(&b""[..])).is_err());
    }

    #[test]
    fn builds_requests() {
        let req = request("POST", "/v1.44/containers/x/start", "docker", &[("Content-Type", "application/json")], Some(b"{}"), false);
        let text = String::from_utf8(req).unwrap();
        assert!(text.starts_with("POST /v1.44/containers/x/start HTTP/1.1\r\nHost: docker\r\n"));
        assert!(text.contains("Content-Length: 2\r\n"));
        assert!(text.ends_with("Connection: close\r\n\r\n{}"));
        let get = String::from_utf8(request("GET", "/x", "h", &[], None, true)).unwrap();
        assert!(get.starts_with("GET /x HTTP/1.0\r\n") && !get.contains("Content-Length"));
    }
}
