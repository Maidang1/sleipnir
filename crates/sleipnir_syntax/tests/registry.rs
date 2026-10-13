//! What the registry answers with no provider installed.
//!
//! This crate links no grammar, so every seeded row is [`Known::Named`] until
//! something registers one. nextest runs each test in its own process, so a
//! registration in one is invisible to the rest.

use sleipnir_syntax::registry::{self, Known};
use std::path::Path;

/// Nothing paints until a provider registers.
#[test]
fn every_seeded_row_is_named_and_none_is_ready() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert!(registry::ready().is_empty());
    assert!(!registry::names().is_empty());
    for name in registry::names() {
        assert_eq!(
            registry::of_tag(name),
            Some(Known::Named(name)),
            "{name} claims a grammar this crate does not link"
        );
    }
}

/// A name nothing in the table claims stays nothing.
#[test]
fn an_unknown_tag_is_none() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert!(registry::of_tag("gleam").is_none());
    assert!(registry::of_path(Path::new("notes.gleam")).is_none());
}

/// Tags carry whatever the fence wrote after them.
#[test]
fn a_tag_is_trimmed_and_case_folded() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert_eq!(registry::of_tag("Rust").map(Known::name), Some("rust"));
    assert_eq!(
        registry::of_tag("rust {.numberLines}").map(Known::name),
        Some("rust")
    );
    assert_eq!(registry::of_tag("rust,foo").map(Known::name), Some("rust"));
}

/// Files resolve through the same table as fence tags.
#[test]
fn a_path_resolves_to_the_same_rows() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert_eq!(
        registry::of_path(Path::new("src/main.rs")).map(Known::name),
        Some("rust")
    );
    assert_eq!(
        registry::of_path(Path::new("routes/+page.svelte")).map(Known::name),
        Some("svelte")
    );
}

/// A whole name beats an extension, and the longer extension beats the shorter.
#[test]
fn a_whole_name_beats_an_extension() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert_eq!(
        registry::of_path(Path::new("Dockerfile")).map(Known::name),
        Some("dockerfile")
    );
    assert_eq!(
        registry::of_path(Path::new(".bashrc")).map(Known::name),
        Some("bash")
    );
    assert_eq!(
        registry::of_path(Path::new("App.tsx")).map(Known::name),
        Some("tsx")
    );
    assert_eq!(
        registry::of_path(Path::new("app.ts")).map(Known::name),
        Some("typescript")
    );
}

/// A name whose bytes are not ASCII must not be indexed mid-character.
///
/// The extension match used to slice `name[name.len() - candidate.len() - 1..]`,
/// which panics on any name where a multi-byte character straddles that offset.
/// Every row here is a name whose byte length differs from its char length by
/// exactly one, plus a whole-name match and an ASCII control.
#[test]
fn a_non_ascii_name_resolves_without_panicking() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    let names = [
        ("資料.rs", "rust"),         // CJK, 3-char prefix
        ("日本語.ts", "typescript"), // 3-char prefix
        ("日本語.tsx", "tsx"),       // 1-char prefix
        ("Привет.md", "markdown"),   // Cyrillic
        ("résumé.css", "css"),       // accented, 3-char prefix
        ("á.ts", "typescript"),      // accented, 1-char prefix
        ("Dörfer.go", "go"),         // accented, 2-char prefix
        ("世界.json", "json"),       // extension sits at a boundary
        ("némesis.rs", "rust"),      // longer accented prefix
    ];
    for (name, expected) in names {
        assert_eq!(
            registry::of_path(Path::new(name)).map(Known::name),
            Some(expected),
            "{name}"
        );
    }
}

/// A name that shares no extension with the table answers none, whatever its
/// script — including when a multi-byte character is the last thing in it.
#[test]
fn a_non_ascii_name_with_no_extension_is_none() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    for name in ["資料", "日本語", "Привет.gleam", "résumé.txt", "あ"] {
        assert!(
            registry::of_path(Path::new(name)).is_none(),
            "{name} claimed a row"
        );
    }
}

/// The extension match still prefers the longest candidate.
#[test]
fn the_longest_extension_wins_under_a_non_ascii_stem() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert_eq!(
        registry::of_path(Path::new("資料.rs")).map(Known::name),
        Some("rust")
    );
    // A dot inside the stem does not create a second extension match.
    assert_eq!(
        registry::of_path(Path::new("my.notes.md")).map(Known::name),
        Some("markdown")
    );
}
