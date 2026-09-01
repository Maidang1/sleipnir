/// Byte-level state machine that extracts Kitty graphics APC sequences from a
/// raw PTY byte stream. Kitty graphics uses APC: `ESC _ G <control>;<payload> ESC \`
///
/// The scanner watches for the two-byte intro `ESC _ G`, buffers everything
/// until the two-byte terminator `ESC \` (ST), and yields each complete
/// graphics payload (the bytes between `G` and `ESC \`).
///
/// Non-APC bytes pass through unmodified, returned in `filtered_output`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScanState {
    Ground,
    Escape,
    ApcStart,
    ApcBody,
    ApcEscape,
}

pub struct ApcScanner {
    state: ScanState,
    buf: Vec<u8>,
}

pub struct ScanResult {
    pub filtered: Vec<u8>,
    pub commands: Vec<Vec<u8>>,
}

impl ApcScanner {
    pub fn new() -> Self {
        Self {
            state: ScanState::Ground,
            buf: Vec::with_capacity(4096),
        }
    }

    pub fn feed(&mut self, data: &[u8]) -> ScanResult {
        let mut filtered = Vec::with_capacity(data.len());
        let mut commands = Vec::new();

        for &byte in data {
            match self.state {
                ScanState::Ground => {
                    if byte == 0x1B {
                        self.state = ScanState::Escape;
                    } else {
                        filtered.push(byte);
                    }
                }
                ScanState::Escape => {
                    if byte == b'_' {
                        self.state = ScanState::ApcStart;
                        self.buf.clear();
                    } else {
                        filtered.push(0x1B);
                        filtered.push(byte);
                        self.state = ScanState::Ground;
                    }
                }
                ScanState::ApcStart => {
                    if byte == b'G' {
                        self.state = ScanState::ApcBody;
                        self.buf.clear();
                    } else {
                        // Not a graphics APC — pass the intro bytes through
                        // and continue discarding in APC body mode since we
                        // still need to consume through ST.
                        self.state = ScanState::ApcBody;
                        self.buf.clear();
                        // Mark as non-graphics so we discard on completion
                        self.buf.push(0); // sentinel: first byte 0 = non-graphics
                        self.buf.push(byte);
                    }
                }
                ScanState::ApcBody => {
                    if byte == 0x1B {
                        self.state = ScanState::ApcEscape;
                    } else {
                        if self.buf.len() < 64 * 1024 * 1024 {
                            self.buf.push(byte);
                        }
                    }
                }
                ScanState::ApcEscape => {
                    if byte == b'\\' {
                        // End of APC — check if this was a graphics sequence
                        if self.buf.first() != Some(&0) {
                            commands.push(std::mem::take(&mut self.buf));
                        } else {
                            self.buf.clear();
                        }
                        self.state = ScanState::Ground;
                    } else {
                        // ESC inside APC that wasn't followed by \ — keep collecting
                        self.buf.push(0x1B);
                        if self.buf.len() < 64 * 1024 * 1024 {
                            self.buf.push(byte);
                        }
                        self.state = ScanState::ApcBody;
                    }
                }
            }
        }

        ScanResult { filtered, commands }
    }

    pub fn reset(&mut self) {
        self.state = ScanState::Ground;
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_apc() {
        let mut scanner = ApcScanner::new();
        // ESC _ G a=q,i=1; ESC \
        let input = b"\x1b_Ga=q,i=1;\x1b\\";
        let result = scanner.feed(input);
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0], b"a=q,i=1;");
        assert!(result.filtered.is_empty());
    }

    #[test]
    fn passes_through_normal_text() {
        let mut scanner = ApcScanner::new();
        let input = b"hello world";
        let result = scanner.feed(input);
        assert!(result.commands.is_empty());
        assert_eq!(result.filtered, b"hello world");
    }

    #[test]
    fn mixed_content() {
        let mut scanner = ApcScanner::new();
        let input = b"before\x1b_Ga=T,i=1;data\x1b\\after";
        let result = scanner.feed(input);
        assert_eq!(result.commands.len(), 1);
        assert_eq!(result.commands[0], b"a=T,i=1;data");
        assert_eq!(result.filtered, b"beforeafter");
    }

    #[test]
    fn non_graphics_apc_discarded() {
        let mut scanner = ApcScanner::new();
        // APC with non-G start byte
        let input = b"\x1b_Xsome data\x1b\\visible";
        let result = scanner.feed(input);
        assert!(result.commands.is_empty());
        assert_eq!(result.filtered, b"visible");
    }

    #[test]
    fn split_across_feeds() {
        let mut scanner = ApcScanner::new();
        let r1 = scanner.feed(b"\x1b_Ga=q,");
        assert!(r1.commands.is_empty());
        let r2 = scanner.feed(b"i=1;\x1b\\");
        assert_eq!(r2.commands.len(), 1);
        assert_eq!(r2.commands[0], b"a=q,i=1;");
    }

    #[test]
    fn escape_sequence_passthrough() {
        let mut scanner = ApcScanner::new();
        // Normal CSI sequence should pass through
        let input = b"\x1b[1mhello\x1b[0m";
        let result = scanner.feed(input);
        assert!(result.commands.is_empty());
        assert_eq!(result.filtered, input.to_vec());
    }

    #[test]
    fn multiple_commands_in_one_feed() {
        let mut scanner = ApcScanner::new();
        let input = b"\x1b_Ga=q,i=1;\x1b\\\x1b_Ga=q,i=2;\x1b\\";
        let result = scanner.feed(input);
        assert_eq!(result.commands.len(), 2);
        assert_eq!(result.commands[0], b"a=q,i=1;");
        assert_eq!(result.commands[1], b"a=q,i=2;");
    }
}
