//! Embedded interface catalogs.
//!
//! Add a locale by creating `../locales/<code>.json` and adding one entry to
//! `define_languages!` below. The settings UI and serialization use the
//! generated registry automatically.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

type Catalog = HashMap<String, String>;

fn parse_catalog(source: &'static str, code: &str) -> Catalog {
    serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("invalid embedded {code} locale catalog: {error}"))
}

macro_rules! define_languages {
    ($( $variant:ident => ($code:literal, $name:literal, $file:literal) ),+ $(,)?) => {
        #[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
        pub enum Language {
            $(
                #[serde(rename = $code)]
                $variant,
            )+
        }

        impl Language {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code),+
                }
            }

            pub fn display_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name),+
                }
            }

            fn catalog(self) -> &'static Catalog {
                match self {
                    $(
                        Self::$variant => {
                            static CATALOG: LazyLock<Catalog> =
                                LazyLock::new(|| parse_catalog(include_str!($file), $code));
                            &CATALOG
                        }
                    ),+
                }
            }

            /// Resolve a stable message key, falling back to English and then
            /// to the key itself so an incomplete locale never breaks the UI.
            pub fn text(self, key: &'static str) -> &'static str {
                self.catalog()
                    .get(key)
                    .or_else(|| Language::En.catalog().get(key))
                    .map(String::as_str)
                    .unwrap_or(key)
            }

            pub fn next(self) -> Self {
                let index = Self::ALL.iter().position(|item| *item == self).unwrap_or(0);
                Self::ALL[(index + 1) % Self::ALL.len()]
            }
        }
    };
}

// Adding a language requires one line here and one JSON catalog.
define_languages! {
    En => ("en", "English", "../locales/en.json"),
    ZhCn => ("zh_cn", "简体中文", "../locales/zh_cn.json"),
}

impl Default for Language {
    fn default() -> Self {
        Self::En
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_has_the_same_keys_as_english() {
        let english = Language::En.catalog();
        for language in Language::ALL {
            let catalog = language.catalog();
            let mut missing: Vec<_> = english
                .keys()
                .filter(|key| !catalog.contains_key(*key))
                .collect();
            let mut extra: Vec<_> = catalog
                .keys()
                .filter(|key| !english.contains_key(*key))
                .collect();
            missing.sort();
            extra.sort();
            assert!(
                missing.is_empty(),
                "{} is missing keys: {missing:?}",
                language.as_str()
            );
            assert!(
                extra.is_empty(),
                "{} has extra keys: {extra:?}",
                language.as_str()
            );
        }
    }

    #[test]
    fn missing_keys_fall_back_without_panicking() {
        assert_eq!(Language::ZhCn.text("missing.example"), "missing.example");
        assert!(Language::ALL.len() >= 2);
        assert_eq!(Language::ALL[0].next(), Language::ALL[1]);
        assert_eq!(Language::ALL.last().copied().unwrap().next(), Language::ALL[0]);
    }
}
