//! Tree-sitter syntax classification.
//!
//! Spans come back in document order, and anything they do not cover is
//! unhighlighted text. No color and no rendering here — map [`HighlightKind`]
//! onto a palette at the paint site.
//!
//! [`standard::LANGS`] is one row per cargo feature, fixed at build time; it
//! seeds nothing by itself. Call [`standard::install`] to put those rows into
//! [`registry`]. [`registry`] also holds the languages this build can name and
//! not paint, and [`registry::register`] adds to it at runtime.
//!
//! A language may carry an injections query marking regions written in another
//! language — `<script>` and `<style>` in html. The injected name is resolved
//! through [`registry`], so it must be one this build can paint.
//!
//! Every language is configured with [`lang::NAMES`]. A `Highlight` index means
//! whatever the layer that produced it was configured with, and an injected
//! parse returns indices of its own.
//!
//! No locals support: that query is passed empty in [`Lang::compile`](lang::Lang).

use std::ops::Range;
use std::sync::{Mutex, MutexGuard};

pub mod document;
mod kind;
pub mod lang;
pub mod registry;
pub mod session;
pub mod standard;

pub use kind::HighlightKind;

/// The exact tree-sitter these grammars were built against. Reach for a
/// `LanguageFn` through here rather than declaring your own tree-sitter, or
/// [`Lang::new`](lang::Lang::new) will not accept it — two versions in the
/// graph are two unrelated types with one name.
pub use tree_sitter;
pub use tree_sitter_language;

/// The wasm engine, which this crate borrows and never builds.
///
/// `tree_sitter::wasmtime` is the only handle that works: wasmtime arrives
/// through tree-sitter, and a second one in the graph is a second `Engine` type.
#[cfg(feature = "wasm")]
pub use tree_sitter::wasmtime;

#[cfg(feature = "wasm")]
static ENGINE: Mutex<Option<wasmtime::Engine>> = Mutex::new(None);

/// Hand this crate the engine wasm grammars are instantiated in. `false` if one
/// was already set, which leaves the first in place.
///
/// The app owns the engine; this keeps a handle so other consumers share it.
/// Until one is set, a [`Grammar::Wasm`](lang::Grammar::Wasm) cannot load.
#[cfg(feature = "wasm")]
pub fn set_engine(engine: wasmtime::Engine) -> bool {
    let mut slot = ENGINE.lock().unwrap_or_else(|err| err.into_inner());
    if slot.is_some() {
        return false;
    }
    *slot = Some(engine);
    true
}

#[cfg(feature = "wasm")]
#[doc(hidden)]
pub fn clear_engine_for_test() {
    *ENGINE.lock().unwrap_or_else(|err| err.into_inner()) = None;
}

#[cfg(feature = "wasm")]
pub(crate) fn engine() -> Option<wasmtime::Engine> {
    ENGINE.lock().unwrap_or_else(|err| err.into_inner()).clone()
}

/// Highlight `source` as `language` (a fence tag — `rs`, `py`, `tsx`, …).
/// `None` when the tag names no language.
pub fn highlight(source: &str, language: &str) -> Option<Vec<(Range<usize>, HighlightKind)>> {
    lang::resolve(language)?.highlight(source)
}

static TEST_ISOLATION: Mutex<()> = Mutex::new(());

/// Serialize and reset process-global registry state for one test.
///
/// Hold the guard for the whole test. `cargo test` runs the tests in one
/// integration-test binary in parallel threads; the registry is a process
/// global, so they would otherwise see each other's registrations.
#[doc(hidden)]
pub fn isolate_for_test() -> MutexGuard<'static, ()> {
    let guard = TEST_ISOLATION.lock().unwrap_or_else(|err| err.into_inner());
    registry::reset();
    #[cfg(feature = "wasm")]
    clear_engine_for_test();
    guard
}
