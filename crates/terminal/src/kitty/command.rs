//! Commands: the parsed keys of one graphics escape, and the payload decoder.

/// What a command asks for — kitty's `a` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Action {
    /// `a=t`: hold the image under its id.
    #[default]
    Transmit,
    /// `a=T`: hold it and put it on the screen at the cursor.
    Display,
    /// `a=q`: answer whether this would have worked, storing nothing.
    Query,
    /// `a=p`: put an image already held on the screen at the cursor.
    Place,
    /// `a=d`: take placements off the screen, and with an upper-case `d`
    /// their images' data too.
    Delete,
    /// `a=f`: add a frame to an image, or draw over one it has.
    Frame,
    /// `a=a`: set an animation's state, current frame, loops or gaps.
    Animate,
    /// `a=c`: copy a rectangle of one frame onto another.
    Compose,
    /// Any other letter: parsed so the run is still consumed rather than
    /// printed, and answered as unsupported.
    Other(char),
}

/// How the payload is encoded — kitty's `f` key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `f=24`: raw RGB, `s` by `v` pixels.
    Rgb,
    /// `f=32`: raw RGBA, `s` by `v` pixels.
    Rgba,
    /// `f=100`: a PNG, which carries its own dimensions.
    Png,
}

/// Whether a display leaves the cursor where it found it — kitty's `C` key.
///
/// The ordering is load-bearing: [`Command::absorb`] merges a transmission
/// with `max`, and `None` — the key that silences the cursor — is the stronger
/// statement, so it is ordered above [`Self::After`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum CursorMovement {
    /// `C=0`: the cursor ends past the image, scrolling the screen when the
    /// image runs off the bottom.
    #[default]
    After,
    /// `C=1`: neither the cursor nor the screen moves.
    None,
}

/// The rectangle a display takes out of the image's pixels: `x`, `y`, `w`, `h`.
///
/// `x` and `y` double as the cell a delete by position names, and as the id
/// range of `d=r` — kitty reuses the keys, so every reader of them is here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// How an image is put on the screen: the cell extent, the source rectangle,
/// the nudge inside that extent, the stacking order, the cursor rule, and the
/// placement relationship.
///
/// A transmission states these keys once, on its first chunk, and a later
/// chunk states only what it changes — see [`Command::absorb`]. A display
/// echoes them, so this is what [`crate::kitty::Display`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Placement {
    /// `p`: the placement id. Zero is unset.
    pub id: u32,
    /// `c`, `r`: the cell extent. Zero is unset, which leaves the extent to
    /// the image's own pixels.
    pub columns: u32,
    pub rows: u32,
    /// `x`, `y`, `w`, `h`: the source rectangle in the image's pixels.
    pub source: SourceRect,
    /// `X`, `Y`: where in its top-left cell the image starts, in pixels.
    pub offset_x: u32,
    pub offset_y: u32,
    /// `z`: the placement's z-index.
    pub z: i32,
    /// `C`: whether the cursor moves past the image.
    pub cursor_movement: CursorMovement,
    /// `P`, `Q`: the image and placement a relative placement hangs off.
    /// A `P` of zero is no parent.
    pub parent_image: u32,
    pub parent_placement: u32,
    /// `H`, `V`: a relative placement's offset from its parent, in cells.
    pub parent_offset_x: i32,
    pub parent_offset_y: i32,
    /// `U=1`: the placement is virtual, shown only where the text holds
    /// [`PLACEHOLDER`] cells naming it.
    pub unicode: bool,
    /// `q`: 1 suppresses success replies, 2 suppresses failures too.
    pub quiet: u8,
}

/// A parsed graphics command: its keys, and the payload already un-base64'd.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Command {
    pub action: Action,
    pub format: Format,
    /// `i`: the id the client filed the image under. Zero means unset, which
    /// is how kitty spells "the terminal picks one".
    pub id: u32,
    /// `I`: an image number, which names the newest image transmitted under
    /// it. Zero is unset. A command carrying both `i` and `I` is refused.
    pub number: u32,
    /// How the image is put on the screen: the cell extent, the source
    /// rectangle, the stacking, and the placement relationship.
    pub placement: Placement,
    /// `m=1`: more chunks follow.
    pub more: bool,
    /// `s`, `v`: the pixel dimensions a raw payload cannot state for itself.
    pub width: u32,
    pub height: u32,
    /// `d`: what a delete is aimed at; an uppercase letter also frees the
    /// data of the images left with no placement.
    pub delete: char,
    /// `t`: where the bytes are. `d` carries them inline; `f`, `t` and `s`
    /// name a file, a temporary file or a shared memory object, read only
    /// once [`Store::set_local_media`] allows it.
    pub medium: char,
    /// `O`, `S`: where in a named medium the bytes start, and how many to
    /// read. A size of zero reads to the end.
    pub read_offset: u32,
    pub read_size: u32,
    /// `o`: payload compression, `None` when there is none. Only `z`, zlib,
    /// is carried out.
    pub compression: Option<char>,
    pub payload: Vec<u8>,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            action: Action::Transmit,
            format: Format::Rgba,
            id: 0,
            number: 0,
            more: false,
            width: 0,
            height: 0,
            placement: Placement::default(),
            delete: 'a',
            medium: 'd',
            read_offset: 0,
            read_size: 0,
            compression: None,
            payload: Vec::new(),
        }
    }
}

impl Command {
    /// Fold one transmission chunk into the keys the first chunk stated.
    ///
    /// Kitty states a transmission's control keys on its first chunk, and the
    /// chunk that ends it carries `m=0` and little else. So `self` is the
    /// accumulator and `chunk` is what just arrived: the chunk wins a key only
    /// where it said something — `z` and the parent offsets are read as
    /// "not stated" when zero, and the numeric extents as "the larger of the
    /// two stated", which is what the accumulator already holds when the chunk
    /// is silent.
    pub(crate) fn absorb(&mut self, chunk: &Command) {
        // The action lives on the first chunk in kitty's own client, and on
        // the last in others. Either is a display, and the display keys ride
        // whichever chunk carries it.
        if chunk.action == Action::Display {
            self.action = Action::Display;
        }
        self.payload.extend_from_slice(&chunk.payload);
        let held = &mut self.placement;
        let chunk = &chunk.placement;
        held.columns = held.columns.max(chunk.columns);
        held.rows = held.rows.max(chunk.rows);
        held.id = held.id.max(chunk.id);
        held.source.x = held.source.x.max(chunk.source.x);
        held.source.y = held.source.y.max(chunk.source.y);
        held.source.width = held.source.width.max(chunk.source.width);
        held.source.height = held.source.height.max(chunk.source.height);
        held.offset_x = held.offset_x.max(chunk.offset_x);
        held.offset_y = held.offset_y.max(chunk.offset_y);
        if chunk.z != 0 {
            held.z = chunk.z;
        }
        held.cursor_movement = held.cursor_movement.max(chunk.cursor_movement);
        held.unicode |= chunk.unicode;
        held.parent_image = held.parent_image.max(chunk.parent_image);
        held.parent_placement = held.parent_placement.max(chunk.parent_placement);
        if chunk.parent_offset_x != 0 {
            held.parent_offset_x = chunk.parent_offset_x;
        }
        if chunk.parent_offset_y != 0 {
            held.parent_offset_y = chunk.parent_offset_y;
        }
        held.quiet = chunk.quiet;
    }

    /// Parse one APC run's body. `None` for anything that is not a graphics
    /// command, which is every other APC sequence on the wire.
    pub(crate) fn parse(run: &[u8]) -> Option<Self> {
        let (&b'G', rest) = run.split_first()? else {
            return None;
        };
        let (keys, payload) = match rest.iter().position(|&byte| byte == b';') {
            Some(at) => (&rest[..at], &rest[at + 1..]),
            None => (rest, &[][..]),
        };
        let mut command = Command::default();
        for pair in keys.split(|&byte| byte == b',') {
            let Some(at) = pair.iter().position(|&byte| byte == b'=') else {
                continue;
            };
            let (key, value) = (&pair[..at], &pair[at + 1..]);
            let [key] = key else { continue };
            let number = || {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|value| value.parse::<u32>().ok())
            };
            let letter = || value.first().map(|&byte| byte as char);
            let signed = || {
                std::str::from_utf8(value)
                    .ok()
                    .and_then(|value| value.parse::<i32>().ok())
            };
            match key {
                b'a' => {
                    command.action = match letter() {
                        Some('t') => Action::Transmit,
                        Some('T') => Action::Display,
                        Some('q') => Action::Query,
                        Some('p') => Action::Place,
                        Some('d') => Action::Delete,
                        Some('f') => Action::Frame,
                        Some('a') => Action::Animate,
                        Some('c') => Action::Compose,
                        Some(other) => Action::Other(other),
                        None => continue,
                    }
                }
                b'f' => {
                    command.format = match number() {
                        Some(24) => Format::Rgb,
                        Some(32) => Format::Rgba,
                        Some(100) => Format::Png,
                        _ => continue,
                    }
                }
                b'i' => command.id = number().unwrap_or(0),
                b'I' => command.number = number().unwrap_or(0),
                b'p' => command.placement.id = number().unwrap_or(0),
                b'x' => command.placement.source.x = number().unwrap_or(0),
                b'y' => command.placement.source.y = number().unwrap_or(0),
                b'w' => command.placement.source.width = number().unwrap_or(0),
                b'h' => command.placement.source.height = number().unwrap_or(0),
                b'X' => command.placement.offset_x = number().unwrap_or(0),
                b'Y' => command.placement.offset_y = number().unwrap_or(0),
                b'z' => command.placement.z = signed().unwrap_or(0),
                b'm' => command.more = number() == Some(1),
                b's' => command.width = number().unwrap_or(0),
                b'v' => command.height = number().unwrap_or(0),
                b'c' => command.placement.columns = number().unwrap_or(0),
                b'r' => command.placement.rows = number().unwrap_or(0),
                b'C' => {
                    command.placement.cursor_movement = match number() {
                        Some(1) => CursorMovement::None,
                        _ => CursorMovement::After,
                    }
                }
                b'U' => command.placement.unicode = number() == Some(1),
                b'P' => command.placement.parent_image = number().unwrap_or(0),
                b'Q' => command.placement.parent_placement = number().unwrap_or(0),
                b'H' => command.placement.parent_offset_x = signed().unwrap_or(0),
                b'V' => command.placement.parent_offset_y = signed().unwrap_or(0),
                b'q' => command.placement.quiet = number().unwrap_or(0).min(u8::MAX as u32) as u8,
                b'd' => command.delete = letter().unwrap_or('a'),
                b't' => command.medium = letter().unwrap_or('d'),
                b'O' => command.read_offset = number().unwrap_or(0),
                b'S' => command.read_size = number().unwrap_or(0),
                b'o' => command.compression = letter(),
                _ => {}
            }
        }
        command.payload = decode(payload);
        Some(command)
    }
}

/// Base64, the subset a graphics payload is: standard alphabet, padding
/// optional, anything else skipped. Written out rather than taken as a
/// dependency — the decoder is the size of the code that would configure one.
pub(crate) fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut bits: u32 = 0;
    let mut held = 0;
    for &byte in bytes {
        let six = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            // Padding ends a group: a new one starts on a byte boundary,
            // which is where a second base64 string run on after it begins.
            b'=' => {
                held = 0;
                continue;
            }
            _ => continue,
        };
        bits = (bits << 6) | six as u32;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push((bits >> held) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kitty states a transmission's control keys on its first chunk, so a
    /// closing chunk that says nothing must not erase what the first stated.
    #[test]
    fn a_chunk_that_states_nothing_leaves_the_first_chunks_keys_alone() {
        let mut held = Command {
            placement: Placement {
                columns: 4,
                rows: 2,
                z: 7,
                cursor_movement: CursorMovement::None,
                parent_offset_x: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        held.absorb(&Command::default());
        assert_eq!(held.placement.columns, 4);
        assert_eq!(held.placement.rows, 2);
        assert_eq!(held.placement.z, 7);
        assert_eq!(held.placement.cursor_movement, CursorMovement::None);
        assert_eq!(held.placement.parent_offset_x, 3);
    }

    /// The chunk that states a key wins it — including the action, which kitty
    /// itself puts on the first chunk and other clients on the last.
    #[test]
    fn a_chunk_that_states_a_key_wins_it() {
        let mut held = Command {
            placement: Placement {
                columns: 4,
                ..Default::default()
            },
            ..Default::default()
        };
        let chunk = Command {
            action: Action::Display,
            placement: Placement {
                id: 6,
                z: 5,
                cursor_movement: CursorMovement::None,
                parent_offset_x: 9,
                ..Default::default()
            },
            ..Default::default()
        };
        held.absorb(&chunk);
        assert_eq!(held.action, Action::Display);
        assert_eq!(held.placement.id, 6);
        assert_eq!(held.placement.z, 5);
        assert_eq!(held.placement.cursor_movement, CursorMovement::None);
        assert_eq!(held.placement.parent_offset_x, 9);
        assert_eq!(held.placement.columns, 4);
    }

    /// Payload bytes accumulate across chunks, in arrival order.
    #[test]
    fn payload_bytes_accumulate_across_chunks() {
        let mut held = Command {
            payload: b"one".to_vec(),
            ..Default::default()
        };
        held.absorb(&Command {
            payload: b"two".to_vec(),
            ..Default::default()
        });
        assert_eq!(held.payload, b"onetwo");
    }
}
