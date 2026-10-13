//! The stream scanner: what it takes off the pty stream, and what it leaves
//! the parser.

mod common;

use common::*;
use terminal::scanner::{Scanner, Segment};

fn commands(scanner: &mut Scanner, bytes: &[u8]) -> usize {
    scanner
        .feed(bytes)
        .iter()
        .filter(|segment| matches!(segment, Segment::Graphics(_)))
        .count()
}

// ---------------------------------------------------------------------------
// Kitty graphics
// ---------------------------------------------------------------------------

#[test]
fn text_either_side_of_a_command_passes_through() {
    let mut scanner = Scanner::new();
    let mut stream = b"before".to_vec();
    stream.extend(apc(&format!("a=t,f=32,s=1,v=1,i=1;{}", base64(&pixel()))));
    stream.extend_from_slice(b"after");
    assert_eq!(passed(&mut scanner, &stream), b"beforeafter");
}

#[test]
fn a_command_split_across_two_reads_is_one_command() {
    let whole = apc(&format!("a=t,f=32,s=1,v=1,i=7;{}", base64(&pixel())));
    // Every split, including the two that land between `ESC` and `_` and
    // between `ESC` and `\` — the boundaries a pty read is as likely to fall
    // on as any other.
    for at in 0..whole.len() {
        let mut scanner = Scanner::new();
        let first = commands(&mut scanner, &whole[..at]);
        let second = commands(&mut scanner, &whole[at..]);
        assert_eq!(
            first + second,
            1,
            "split at {at} produced {first} + {second} commands"
        );
    }
}

#[test]
fn an_escape_held_across_a_read_is_handed_back_whole() {
    let mut scanner = Scanner::new();
    // `ESC` alone, then the sequence it turned out to introduce: the parser
    // has to receive both bytes or the cursor never moves.
    assert_eq!(passed(&mut scanner, b"\x1b"), b"");
    assert_eq!(passed(&mut scanner, b"[H"), b"\x1b[H");
}

#[test]
fn another_apc_sequence_is_swallowed_rather_than_printed() {
    let mut scanner = Scanner::new();
    // Not ours — but it was never going to reach the screen either way, since
    // `vte` discards the run. What matters is that the payload does not.
    let stream = b"a\x1b_NOTGRAPHICS\x1b\\b";
    assert_eq!(passed(&mut scanner, stream), b"ab");
    assert_eq!(commands(&mut Scanner::new(), stream), 0);
}

#[test]
fn a_run_with_no_terminator_does_not_grow_forever() {
    let mut scanner = Scanner::new();
    let mut stream = b"\x1b_Ga=t;".to_vec();
    stream.extend(std::iter::repeat_n(b'A', 1 << 20));
    assert_eq!(commands(&mut scanner, &stream), 0);
    // And the stream recovers at the next terminator rather than eating the
    // rest of the session.
    assert_eq!(passed(&mut scanner, b"\x1b\\back"), b"back");
}

#[test]
fn a_bel_terminates_a_run_as_well_as_st() {
    let mut scanner = Scanner::new();
    let mut stream = format!("\x1b_Ga=t,f=32,s=1,v=1,i=3;{}\x07", base64(&pixel())).into_bytes();
    stream.extend_from_slice(b"tail");
    assert_eq!(commands(&mut Scanner::new(), &stream), 1);
    assert_eq!(passed(&mut scanner, &stream), b"tail");
}

// ---------------------------------------------------------------------------
// Synchronized output
// ---------------------------------------------------------------------------

#[test]
fn the_scanner_reports_mode_2026_and_still_passes_its_bytes_on() {
    let mut scanner = Scanner::new();
    let input = b"a\x1b[?2026hb\x1b[?2026lc";
    let segments = scanner.feed(input);

    let holds: Vec<bool> = segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Sync(hold) => Some(*hold),
            _ => None,
        })
        .collect();
    assert_eq!(holds, vec![true, false]);

    let text: Vec<u8> = segments
        .iter()
        .filter_map(|segment| match segment {
            Segment::Text(text) => Some(*text),
            _ => None,
        })
        .flatten()
        .copied()
        .collect();
    assert_eq!(text, input);
}

#[test]
fn a_mode_2026_run_split_across_reads_is_one_edge() {
    let mut scanner = Scanner::new();
    assert!(!scanner.feed(b"\x1b[?20").iter().any(is_sync));
    assert!(scanner.feed(b"26h").iter().any(is_sync));
}

#[test]
fn a_mode_2026_query_is_not_an_edge() {
    let mut scanner = Scanner::new();
    assert!(!scanner.feed(b"\x1b[?2026$p").iter().any(is_sync));
}

#[test]
fn a_graphics_payload_cannot_finish_a_mode_2026_run() {
    let mut scanner = Scanner::new();
    // The `ESC` opening the APC would otherwise count as the first byte of a
    // BSU, leaving the rest to be completed by whatever follows the run.
    assert!(!scanner.feed(b"\x1b_Ga=q\x1b\\[?2026h").iter().any(is_sync));
}

/// The cell size query nothing answers: its bytes still reach the parser,
/// which is all the `CSI 16 t` handling ever amounted to here.
#[test]
fn a_cell_size_query_is_passed_through_as_text() {
    let mut scanner = Scanner::new();
    assert_eq!(passed(&mut scanner, b"a\x1b[16tb"), b"a\x1b[16tb");
}

#[test]
fn osc133_and_osc9_notify_pass_through() {
    let mut scanner = Scanner::new();
    let input = b"\x1b]133;A\x1b\\\x1b]9;notify\x07\x1b]777;notify;title;body\x1b\\";
    assert_eq!(passed(&mut scanner, input), input);
}

// ---------------------------------------------------------------------------
// Image payloads
// ---------------------------------------------------------------------------

/// A sixel image's data is taken off the stream; the DCS's own header and
/// terminator still reach the parser, which leaves the sequence in the state
/// it would have.
#[test]
fn a_sixel_images_data_is_swallowed_and_its_frame_is_not() {
    let mut scanner = Scanner::new();
    let stream = b"a\x1bP0;1q#0;2;100;100;0-bytes\x1b\\b";
    assert_eq!(passed(&mut scanner, stream), b"a\x1bP0;1q\x1b\\b");
}

/// A sixel is swallowed from a read boundary the same way: the bytes between
/// are held by nobody, and so cannot straddle a split.
#[test]
fn a_sixel_split_across_reads_is_still_swallowed() {
    let whole = b"a\x1bP0;1q#0;2;100;100;0-bytes\x1b\\b";
    for at in 0..whole.len() {
        let mut scanner = Scanner::new();
        let first = passed(&mut scanner, &whole[..at]);
        let second = passed(&mut scanner, &whole[at..]);
        assert_eq!(
            [first.as_slice(), second.as_slice()].concat(),
            b"a\x1bP0;1q\x1b\\b",
            "split at {at}"
        );
    }
}

/// An `OSC 1337` image payload is swallowed; the command's head and its
/// terminator still reach the parser.
#[test]
fn an_osc1337_image_payload_is_swallowed_and_its_head_is_not() {
    let mut scanner = Scanner::new();
    let stream = b"a\x1b]1337;File=inline=1:cGF5bG9hZA==\x1b\\b";
    assert_eq!(
        passed(&mut scanner, stream),
        b"a\x1b]1337;File=inline=1:\x1b\\b"
    );
}

/// `BEL` ends an image payload too, and the terminator is the parser's to
/// see.
#[test]
fn an_osc1337_image_payload_ends_on_bel() {
    let mut scanner = Scanner::new();
    let stream = b"\x1b]1337;FilePart=cGF5bG9hZA==\x07tail";
    assert_eq!(passed(&mut scanner, stream), b"\x1b]1337;FilePart=\x07tail");
}

/// A command with no payload of its own is not the scanner's: `MultipartFile`
/// states a file is coming, and the parser is handed it and its terminator.
#[test]
fn a_multipart_file_header_passes_through_whole() {
    let mut scanner = Scanner::new();
    let stream = b"\x1b]1337;MultipartFile=size=12\x1b\\tail";
    assert_eq!(
        passed(&mut scanner, stream),
        b"\x1b]1337;MultipartFile=size=12\x1b\\tail"
    );
}

/// A payload the terminal never sees the end of is swallowed to the end of
/// the session rather than buffered: the bytes in between cost nothing to
/// discard.
#[test]
fn an_osc1337_payload_with_no_terminator_is_discarded() {
    let mut scanner = Scanner::new();
    let mut stream = b"\x1b]1337;File=inline=1:".to_vec();
    stream.extend(std::iter::repeat_n(b'A', 1 << 20));
    assert_eq!(passed(&mut scanner, &stream), b"\x1b]1337;File=inline=1:");
}

fn is_sync(segment: &Segment<'_>) -> bool {
    matches!(segment, Segment::Sync(_))
}
