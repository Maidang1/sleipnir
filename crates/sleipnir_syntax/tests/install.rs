//! What `install` puts into the registry.

use sleipnir_syntax::registry::{self, Known};
use std::path::Path;

/// Before install the registry names these languages and paints none of them.
/// nextest runs each test in its own process, so this sees a fresh registry.
#[test]
fn nothing_paints_until_install() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    assert!(registry::ready().is_empty());
    assert_eq!(registry::of_tag("rs"), Some(Known::Named("rust")));

    sleipnir_syntax::standard::install();

    assert_eq!(registry::of_tag("rs").map(Known::name), Some("rust"));
    assert!(registry::of_tag("rs").and_then(Known::lang).is_some());
}

/// Every row this build carries paints, and answers to its files.
#[test]
fn every_carried_row_is_ready_and_paints() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    sleipnir_syntax::standard::install();

    for lang in sleipnir_syntax::standard::LANGS {
        let known = registry::of_tag(lang.name).expect("a carried row answers to its own name");
        assert!(
            known.lang().is_some(),
            "{} registered without a lang",
            lang.name
        );
    }
    assert_eq!(
        registry::ready().len(),
        sleipnir_syntax::standard::LANGS.len()
    );
    assert_eq!(
        registry::of_path(Path::new("src/main.rs")).map(Known::name),
        Some("rust")
    );
    assert!(
        sleipnir_syntax::highlight("fn main() {}", "rust").is_some_and(|spans| !spans.is_empty())
    );
}

/// Installing twice replaces the same rows rather than doubling them.
#[test]
fn installing_twice_is_the_same_registry() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    sleipnir_syntax::standard::install();
    let once = registry::names().len();
    sleipnir_syntax::standard::install();
    assert_eq!(registry::names().len(), once);
}

/// A language no provider here carries stays named and unpainted.
#[test]
fn install_leaves_the_rest_named() {
    let _isolate = sleipnir_syntax::isolate_for_test();
    sleipnir_syntax::standard::install();

    assert_eq!(registry::of_tag("svelte"), Some(Known::Named("svelte")));
    assert_eq!(
        registry::of_path(Path::new("routes/+page.svelte")),
        Some(Known::Named("svelte"))
    );
}
