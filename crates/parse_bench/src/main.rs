//! Headless emulator-parse throughput benchmark.
//!
//! Measures how fast the `alacritty_terminal` backend — the exact parser
//! Sleipnir uses — ingests a byte stream into its grid, with rendering
//! suppressed. This is the analogue of kitty's `kitten __benchmark__`
//! (parser-only) methodology, and is **not** the end-to-end `cat` throughput
//! measured inside the GUI (see `scripts/bench/README.md`).
//!
//! Usage:
//!   cargo run --release -p parse_bench -- <file> [chunk_bytes] [cols] [rows]
//!
//! Defaults: chunk = 65536 bytes (matches the event-loop read granularity),
//! grid = 120 cols x 40 rows, scrollback = 10000 (Config::default, same as
//! Sleipnir's `max_scroll_history_lines` default).

use std::env;
use std::fs;
use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let path = args.get(1).map(String::as_str).unwrap_or("bench-ascii.txt");
    let chunk = args
        .get(2)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(65536);
    let cols = args
        .get(3)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(120);
    let rows = args
        .get(4)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(40);

    let bytes = fs::read(path).expect("failed to read corpus file");

    let mut term = Term::new(Config::default(), &Size { cols, rows }, VoidListener);
    let mut parser: Processor<StdSyncHandler> = Processor::new();

    // Warm up the parser (first call allocates / initializes internal state).
    parser.advance(&mut term, b"warmup\n");

    let t = Instant::now();
    let chunk = chunk.clamp(1, bytes.len());
    for b in bytes.chunks(chunk) {
        parser.advance(&mut term, b);
    }
    let dur = t.elapsed().as_secs_f64();

    let mb = bytes.len() as f64 / 1_000_000.0;
    let mib = bytes.len() as f64 / (1024.0 * 1024.0);
    println!("file        {}", path);
    println!(
        "bytes       {} ({:.2} MB / {:.2} MiB)",
        bytes.len(),
        mb,
        mib
    );
    println!(
        "grid        {} cols x {} rows, scrollback = 10000",
        cols, rows
    );
    println!("chunk       {} bytes", chunk);
    println!("elapsed     {:.4} s", dur);
    println!("throughput  {:.2} MB/s  ({:.2} MiB/s)", mb / dur, mib / dur);
}
