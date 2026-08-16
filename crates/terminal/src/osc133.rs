//! OSC 133 shell-integration markers (M14).
//!
//! Detect-first: we recognize Kitty/Ghostty-style sequences without injecting
//! a shell plugin. Sequence forms:
//!
//! - `ESC ] 133 ; A ST` — prompt start
//! - `ESC ] 133 ; B ST` — command start (input begins)
//! - `ESC ] 133 ; C ST` — command executed
//! - `ESC ] 133 ; D [; <status>] ST` — command finished
//!
//! ST may be `BEL` (`\x07`) or `ESC \\`.

/// Kind of shell-integration marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Osc133Kind {
    PromptStart,
    CommandStart,
    CommandExecuted,
    CommandFinished { status: Option<i32> },
}

impl Osc133Kind {
    /// Parse an alacritty event payload (`"A"`, `"B"`, `"C"`, `"D"`, `"D;0"`).
    pub fn from_payload(payload: &str) -> Option<Self> {
        let mut parts = payload.split(';');
        match parts.next()? {
            "A" => Some(Self::PromptStart),
            "B" => Some(Self::CommandStart),
            "C" => Some(Self::CommandExecuted),
            "D" => {
                let status = parts.next().and_then(|s| s.parse().ok());
                Some(Self::CommandFinished { status })
            }
            _ => None,
        }
    }
}

/// One marker with optional scrollback line (filled by the terminal when known).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Osc133Marker {
    pub kind: Osc133Kind,
    pub line: Option<i32>,
    /// Cursor column when the marker was recorded (input start for B).
    pub column: Option<usize>,
}

/// Overlay triangle on a command start or end line (does not occupy a grid cell).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterKind {
    Start,
    End,
}

/// One pane-gutter mark derived from OSC 133 C/D pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GutterMark {
    pub line: i32,
    pub kind: GutterKind,
    /// `None` while the command is still running or the finish had no status.
    pub status: Option<i32>,
}

/// Convert an absolute marker line into a viewport display line.
pub fn absolute_to_display_line(absolute: i32, history_size: i32, display_offset: usize) -> i32 {
    absolute - history_size + display_offset as i32
}

/// Pair each OSC 133 C with the following D. An unmatched C is a running start mark.
pub fn gutter_marks_from_markers(markers: &[Osc133Marker]) -> Vec<GutterMark> {
    let mut out = Vec::new();
    let mut open: Option<i32> = None;
    for marker in markers {
        match marker.kind {
            Osc133Kind::CommandExecuted => {
                if let Some(line) = marker.line {
                    open = Some(line);
                }
            }
            Osc133Kind::CommandFinished { status } => {
                if let Some(start) = open.take() {
                    out.push(GutterMark {
                        line: start,
                        kind: GutterKind::Start,
                        status,
                    });
                    if let Some(end) = marker.line {
                        out.push(GutterMark {
                            line: end,
                            kind: GutterKind::End,
                            status,
                        });
                    }
                }
            }
            Osc133Kind::PromptStart | Osc133Kind::CommandStart => {}
        }
    }
    if let Some(start) = open {
        out.push(GutterMark {
            line: start,
            kind: GutterKind::Start,
            status: None,
        });
    }
    out
}

/// Incremental scanner for OSC 133 sequences in a byte stream.
#[derive(Debug, Default)]
pub struct Osc133Scanner {
    /// Partial match buffer for an in-progress OSC sequence.
    buf: Vec<u8>,
    /// True once we have seen `ESC ]`.
    in_osc: bool,
}

impl Osc133Scanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; return any complete markers found (order preserved).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Osc133Kind> {
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
                if let Some(k) = parse_osc133_payload(&self.buf) {
                    out.push(k);
                }
                self.reset();
                continue;
            }
            // ESC \ terminator (ST)
            if self.buf.len() >= 2 && self.buf[self.buf.len() - 2] == 0x1b && b == b'\\' {
                if let Some(k) = parse_osc133_payload(&self.buf[..self.buf.len() - 2]) {
                    out.push(k);
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

/// Parse a buffer that starts with `ESC ] ...` (without final ST/BEL if already stripped).
fn parse_osc133_payload(buf: &[u8]) -> Option<Osc133Kind> {
    // Expect ESC ]
    if buf.len() < 3 || buf[0] != 0x1b || buf[1] != b']' {
        return None;
    }
    let body = std::str::from_utf8(&buf[2..]).ok()?;
    // Strip trailing BEL if still present
    let body = body.trim_end_matches('\u{07}');
    // Form: 133;A or 133;D;0
    let mut parts = body.split(';');
    let code = parts.next()?;
    if code != "133" {
        return None;
    }
    let kind = parts.next()?;
    match kind {
        "A" => Some(Osc133Kind::PromptStart),
        "B" => Some(Osc133Kind::CommandStart),
        "C" => Some(Osc133Kind::CommandExecuted),
        "D" => {
            let status = parts.next().and_then(|s| s.parse().ok());
            Some(Osc133Kind::CommandFinished { status })
        }
        _ => None,
    }
}

/// One-shot parse of a complete slice (for tests / batch).
pub fn scan_osc133(bytes: &[u8]) -> Vec<Osc133Kind> {
    let mut s = Osc133Scanner::new();
    s.push(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prompt_and_finish_bel() {
        let seq = b"\x1b]133;A\x07hello\x1b]133;D;0\x07";
        let kinds = scan_osc133(seq);
        assert_eq!(
            kinds,
            vec![
                Osc133Kind::PromptStart,
                Osc133Kind::CommandFinished { status: Some(0) },
            ]
        );
    }

    #[test]
    fn parses_st_terminator() {
        let seq = b"\x1b]133;B\x1b\\";
        assert_eq!(scan_osc133(seq), vec![Osc133Kind::CommandStart]);
    }

    #[test]
    fn incremental_across_chunks() {
        let mut s = Osc133Scanner::new();
        assert!(s.push(b"\x1b]13").is_empty());
        assert!(s.push(b"3;C").is_empty());
        assert_eq!(s.push(b"\x07"), vec![Osc133Kind::CommandExecuted]);
    }

    #[test]
    fn ignores_other_osc() {
        let seq = b"\x1b]0;title\x07\x1b]133;A\x07";
        assert_eq!(scan_osc133(seq), vec![Osc133Kind::PromptStart]);
    }

    #[test]
    fn from_payload_parses_alacritty_event_kinds() {
        assert_eq!(Osc133Kind::from_payload("A"), Some(Osc133Kind::PromptStart));
        assert_eq!(
            Osc133Kind::from_payload("B"),
            Some(Osc133Kind::CommandStart)
        );
        assert_eq!(
            Osc133Kind::from_payload("C"),
            Some(Osc133Kind::CommandExecuted)
        );
        assert_eq!(
            Osc133Kind::from_payload("D"),
            Some(Osc133Kind::CommandFinished { status: None })
        );
        assert_eq!(
            Osc133Kind::from_payload("D;0"),
            Some(Osc133Kind::CommandFinished { status: Some(0) })
        );
        assert_eq!(
            Osc133Kind::from_payload("D;1"),
            Some(Osc133Kind::CommandFinished { status: Some(1) })
        );
        assert_eq!(Osc133Kind::from_payload("Z"), None);
        assert_eq!(Osc133Kind::from_payload(""), None);
    }

    #[test]
    fn gutter_pairs_c_with_following_d() {
        let markers = [
            Osc133Marker {
                kind: Osc133Kind::CommandExecuted,
                line: Some(10),
                column: Some(0),
            },
            Osc133Marker {
                kind: Osc133Kind::CommandFinished { status: Some(1) },
                line: Some(18),
                column: Some(0),
            },
        ];
        assert_eq!(
            gutter_marks_from_markers(&markers),
            vec![
                GutterMark {
                    line: 10,
                    kind: GutterKind::Start,
                    status: Some(1),
                },
                GutterMark {
                    line: 18,
                    kind: GutterKind::End,
                    status: Some(1),
                },
            ]
        );
    }

    #[test]
    fn unmatched_c_is_a_running_start_mark() {
        let markers = [Osc133Marker {
            kind: Osc133Kind::CommandExecuted,
            line: Some(4),
            column: Some(2),
        }];
        assert_eq!(
            gutter_marks_from_markers(&markers),
            vec![GutterMark {
                line: 4,
                kind: GutterKind::Start,
                status: None,
            }]
        );
    }

    #[test]
    fn absolute_display_line_uses_viewport_top() {
        assert_eq!(absolute_to_display_line(100, 80, 10), 30);
        assert_eq!(absolute_to_display_line(70, 80, 0), -10);
    }
}
