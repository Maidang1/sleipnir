//! Bounded JSON-lines I/O for the coordination socket.

use std::io::{self, Read};

/// Maximum request/response line length, excluding the trailing newline.
pub const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum BoundedRead {
    Eof,
    Line(String),
    Oversize,
}

/// Read one line, capped at `max` bytes before the newline.
pub fn read_bounded_line(reader: &mut impl Read, max: usize) -> io::Result<BoundedRead> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte)? {
            0 => {
                if buf.is_empty() {
                    return Ok(BoundedRead::Eof);
                }
                break;
            }
            _ if byte[0] == b'\n' => break,
            _ => {
                if buf.len() >= max {
                    return Ok(BoundedRead::Oversize);
                }
                buf.push(byte[0]);
            }
        }
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    match String::from_utf8(buf) {
        Ok(line) => Ok(BoundedRead::Line(line)),
        Err(err) => Err(io::Error::new(io::ErrorKind::InvalidData, err)),
    }
}

/// Consume the rest of a truncated line so the peer can finish writing.
pub fn drain_until_newline(reader: &mut impl Read) {
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte) {
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_a_line_without_the_newline() {
        let mut cur = Cursor::new(b"hello\nworld\n");
        assert_eq!(
            read_bounded_line(&mut cur, 16).unwrap(),
            BoundedRead::Line("hello".into())
        );
        assert_eq!(
            read_bounded_line(&mut cur, 16).unwrap(),
            BoundedRead::Line("world".into())
        );
        assert_eq!(read_bounded_line(&mut cur, 16).unwrap(), BoundedRead::Eof);
    }

    #[test]
    fn strips_crlf() {
        let mut cur = Cursor::new(b"hello\r\n");
        assert_eq!(
            read_bounded_line(&mut cur, 16).unwrap(),
            BoundedRead::Line("hello".into())
        );
    }

    #[test]
    fn exact_max_is_ok() {
        let mut body = vec![b'x'; 8];
        body.push(b'\n');
        let mut cur = Cursor::new(body);
        match read_bounded_line(&mut cur, 8).unwrap() {
            BoundedRead::Line(line) => assert_eq!(line.len(), 8),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn oversize_without_newline_is_rejected() {
        let mut cur = Cursor::new(vec![b'x'; 9]);
        assert_eq!(
            read_bounded_line(&mut cur, 8).unwrap(),
            BoundedRead::Oversize
        );
    }

    #[test]
    fn oversize_with_newline_is_rejected() {
        let mut body = vec![b'x'; 9];
        body.push(b'\n');
        let mut cur = Cursor::new(body);
        assert_eq!(
            read_bounded_line(&mut cur, 8).unwrap(),
            BoundedRead::Oversize
        );
    }
}
