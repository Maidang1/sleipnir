//! OSC 9 / OSC 777 desktop-notification detection.
//!
//! - `ESC ] 9 ; <message> ST` — the classic desktop-notification escape
//!   (OSC 9), used by `notify-send`-style hooks and iTerm2.
//! - `ESC ] 777 ; notify ; <message> ST` — kitty's notify escape.
//!
//! ST may be `BEL` (`\x07`) or `ESC \\`. Detection is detect-first: no shell
//! plugin is required, and an unrecognized OSC is silently ignored.

/// A desktop notification request extracted from the byte stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OscNotify {
    pub message: String,
}

/// Incremental scanner for OSC 9 / 777 notification sequences.
#[derive(Debug, Default)]
pub struct OscNotifyScanner {
    /// Partial match buffer for an in-progress OSC sequence.
    buf: Vec<u8>,
    /// True once we have seen `ESC ]`.
    in_osc: bool,
}

impl OscNotifyScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; return any complete notifications found (order preserved).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<OscNotify> {
        let mut out = Vec::new();
        for &b in bytes {
            if !self.in_osc {
                if b == 0x1b {
                    self.buf.clear();
                    self.buf.push(b);
                    self.in_osc = true;
                }
                continue;
            }
            self.buf.push(b);
            // Abort oversized garbage.
            if self.buf.len() > 64 {
                self.reset();
                continue;
            }
            // BEL terminator
            if b == 0x07 {
                if let Some(n) = parse_osc_notify_payload(&self.buf) {
                    out.push(n);
                }
                self.reset();
                continue;
            }
            // ESC \ terminator (ST)
            if self.buf.len() >= 2
                && self.buf[self.buf.len() - 2] == 0x1b
                && b == b'\\'
            {
                if let Some(n) = parse_osc_notify_payload(&self.buf[..self.buf.len() - 2]) {
                    out.push(n);
                }
                self.reset();
            }
        }
        out
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.in_osc = false;
    }
}

/// Parse a buffer that starts with `ESC ] ...` (without final ST/BEL).
fn parse_osc_notify_payload(buf: &[u8]) -> Option<OscNotify> {
    if buf.len() < 3 || buf[0] != 0x1b || buf[1] != b']' {
        return None;
    }
    let body = std::str::from_utf8(&buf[2..]).ok()?;
    let body = body.trim_end_matches('\u{07}');
    let (code, rest) = body.split_once(';')?;
    match code {
        "9" => Some(OscNotify {
            message: rest.to_string(),
        }),
        "777" => {
            let (sub, message) = rest.split_once(';')?;
            if sub == "notify" {
                Some(OscNotify {
                    message: message.to_string(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// One-shot parse of a complete slice (for tests / batch).
pub fn scan_osc_notify(bytes: &[u8]) -> Vec<OscNotify> {
    let mut s = OscNotifyScanner::new();
    s.push(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc9_with_bel() {
        let seq = b"\x1b]9;build done\x07";
        assert_eq!(
            scan_osc_notify(seq),
            vec![OscNotify {
                message: "build done".into()
            }]
        );
    }

    #[test]
    fn parses_osc9_with_st() {
        let seq = b"\x1b]9;hello\x1b\\";
        assert_eq!(
            scan_osc_notify(seq),
            vec![OscNotify {
                message: "hello".into()
            }]
        );
    }

    #[test]
    fn parses_osc777_notify() {
        let seq = b"\x1b]777;notify;deploy finished\x07";
        assert_eq!(
            scan_osc_notify(seq),
            vec![OscNotify {
                message: "deploy finished".into()
            }]
        );
    }

    #[test]
    fn message_may_contain_semicolons() {
        let seq = b"\x1b]9;a;b;c\x07";
        assert_eq!(
            scan_osc_notify(seq),
            vec![OscNotify {
                message: "a;b;c".into()
            }]
        );
    }

    #[test]
    fn ignores_other_osc_and_133() {
        let seq = b"\x1b]133;A\x07\x1b]0;title\x07\x1b]777;scroll;5\x07";
        assert!(scan_osc_notify(seq).is_empty());
    }

    #[test]
    fn incremental_across_chunks() {
        let mut s = OscNotifyScanner::new();
        assert!(s.push(b"\x1b]9;par").is_empty());
        assert_eq!(
            s.push(b"tial\x07"),
            vec![OscNotify {
                message: "partial".into()
            }]
        );
    }
}
