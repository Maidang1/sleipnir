//! Which languages this process can name, and which of those it can paint.
//!
//! [`LANGS`] seeds the registry; [`register`] adds to it at runtime. A name with
//! no grammar is still an entry — [`Known::Named`] — so a caller can tell a
//! language this build cannot paint from a file extension nobody has written a
//! grammar for.
//!
//! Registrations are leaked. A language lives for the process, and the strings
//! are a few hundred bytes against grammar bytes that are refcounted.

use crate::lang::Lang;
use std::{
    fmt,
    path::Path,
    sync::{OnceLock, RwLock},
};

/// What the registry knows about a name.
#[derive(Clone, Copy)]
pub enum Known {
    /// A grammar and a query are here.
    Ready(&'static Lang),
    /// Named, with nothing in this build to paint it.
    Named(&'static str),
}

/// Compared by name, on the same side of the [`Self::Ready`] /
/// [`Self::Named`] line.
///
/// Registering leaks the language for the process, so two rows naming one
/// language are one row — comparing names rests on that rather than on the
/// pointers the leak happens to produce, which is what a pointer comparison
/// would have rested on.
impl PartialEq for Known {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Ready(held), Self::Ready(other)) => held.name == other.name,
            (Self::Named(held), Self::Named(other)) => held == other,
            _ => false,
        }
    }
}

impl Eq for Known {}

impl fmt::Debug for Known {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ready(lang) => write!(f, "Ready({})", lang.name),
            Self::Named(name) => write!(f, "Named({name})"),
        }
    }
}

impl Known {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ready(lang) => lang.name,
            Self::Named(name) => name,
        }
    }

    pub fn lang(self) -> Option<&'static Lang> {
        match self {
            Self::Ready(lang) => Some(lang),
            Self::Named(_) => None,
        }
    }
}

/// One language the registry knows of.
pub struct Entry {
    pub name: &'static str,
    /// Fence tags this answers to, lowercase.
    pub aliases: &'static [&'static str],
    /// Whole file names and extensions — `Dockerfile` is a name, `rs` an
    /// extension. [`of_path`] tries whole names first.
    pub files: &'static [&'static str],
    /// `None` names a language this build cannot paint.
    pub lang: Option<&'static Lang>,
}

impl Entry {
    fn known(&self) -> Known {
        match self.lang {
            Some(lang) => Known::Ready(lang),
            None => Known::Named(self.name),
        }
    }
}

fn registry() -> &'static RwLock<Vec<Entry>> {
    static REGISTRY: OnceLock<RwLock<Vec<Entry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(seeded()))
}

/// The names alone. Grammars arrive through [`register`], from
/// [`crate::standard`] or from a provider of your own.
fn seeded() -> Vec<Entry> {
    NAMED
        .iter()
        .map(|(name, aliases, files)| Entry {
            name,
            aliases,
            files,
            lang: None,
        })
        .collect()
}

/// The registry, read. A poisoned lock recovers rather than emptying the
/// registry: the rows are leaked `&'static` data, so a panic while one held
/// the lock left them intact and leave them readable.
fn read() -> std::sync::RwLockReadGuard<'static, Vec<Entry>> {
    registry().read().unwrap_or_else(|err| err.into_inner())
}

/// The registry, written.
fn write() -> std::sync::RwLockWriteGuard<'static, Vec<Entry>> {
    registry().write().unwrap_or_else(|err| err.into_inner())
}

/// Put the registry back to the seeded name table.
///
/// Integration tests share one process under `cargo test` (nextest would give
/// each test its own). Registrations leak for the process, so a test that
/// needs the untouched table calls this while it holds [`crate::isolate_for_test`].
#[doc(hidden)]
pub fn reset() {
    *write() = seeded();
}

/// Add a language, or replace the entry of the same name. Leaks: see the module
/// note.
pub fn register(entry: Entry) {
    let mut entries = write();
    entries.retain(|held| held.name != entry.name);
    entries.push(entry);
}

/// The language a fence tag names. Tags are the raw first word of the fence
/// info string — `rust {.numberLines}`, `rust,foo`, `Rust` — so they are
/// trimmed at the first space or comma and case-folded before lookup.
pub fn of_tag(tag: &str) -> Option<Known> {
    let tag = tag
        .split([' ', ','])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let entries = read();
    entries
        .iter()
        .find(|entry| entry.aliases.contains(&tag.as_str()))
        .map(Entry::known)
}

/// The language `path` is written in.
///
/// A whole name beats an extension — `Dockerfile` is not a `.file` — and
/// between extensions the longest match wins, so `.d.ts` would beat `.ts`
/// rather than racing it.
pub fn of_path(path: &Path) -> Option<Known> {
    let name = path.file_name()?.to_str()?;
    let entries = read();
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .files
                .iter()
                .filter(|candidate| matches(name, candidate))
                .map(|candidate| (candidate.len(), entry))
                .max_by_key(|(len, _)| *len)
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, entry)| entry.known())
}

fn matches(name: &str, candidate: &str) -> bool {
    if name.eq_ignore_ascii_case(candidate) {
        return true;
    }
    // `name` ends with `.{candidate}`, answered by byte arithmetic instead of
    // string slicing: `candidate` and the dot are ASCII, so `start` lands on a
    // boundary whenever it lands on a dot, and a non-ASCII file name such as
    // `資料.rs` never indexes into the middle of a character.
    match name.len().checked_sub(candidate.len() + 1) {
        Some(start) if name.is_char_boundary(start) => {
            name.as_bytes()[start] == b'.' && name[start + 1..].eq_ignore_ascii_case(candidate)
        }
        _ => false,
    }
}

/// The file names and extensions the registry's name table holds for `name`.
///
/// Only seeded rows are answerable — a [`register`]ed entry may carry its own
/// list, which this does not see. Standard's rows mirror this table, so they
/// ask here instead of keeping a second copy of it.
pub fn files_of(name: &str) -> &'static [&'static str] {
    NAMED
        .iter()
        .find(|(held, _, _)| *held == name)
        .map(|(_, _, files)| *files)
        .unwrap_or(&[])
}

/// Every name the registry holds, painted or not.
pub fn names() -> Vec<&'static str> {
    let Ok(entries) = registry().read() else {
        return Vec::new();
    };
    entries.iter().map(|entry| entry.name).collect()
}

/// Every name this build can paint.
pub fn ready() -> Vec<&'static str> {
    let Ok(entries) = registry().read() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter(|entry| entry.lang.is_some())
        .map(|entry| entry.name)
        .collect()
}

/// Every language this crate can name: name, fence tags, then file names and
/// extensions. None carries a grammar — that is a provider's job, and until one
/// registers, every one of these resolves to [`Known::Named`].
#[rustfmt::skip]
const NAMED: &[(&str, &[&str], &[&str])] = &[
    ("bash", &["bash", "sh", "shell", "zsh", "console"], &[".bash_profile", ".bashrc", ".profile", ".zshrc", "bash", "sh", "zsh"]),
    ("c", &["c"], &["c", "h"]),
    ("cpp", &["cpp", "c++"], &["cc", "cpp", "cxx", "hpp"]),
    ("csharp", &["csharp", "cs"], &["cs"]),
    ("css", &["css", "scss"], &["css", "scss"]),
    ("dockerfile", &["dockerfile"], &["Containerfile", "Dockerfile"]),
    ("elixir", &["elixir", "ex"], &["ex", "exs"]),
    ("go", &["go", "golang"], &["go"]),
    ("graphql", &["graphql", "gql"], &["gql", "graphql"]),
    ("haskell", &["haskell", "hs"], &["hs"]),
    ("html", &["html"], &["htm", "html"]),
    ("java", &["java"], &["java"]),
    ("json", &["json", "jsonc"], &["json", "jsonc"]),
    ("kotlin", &["kotlin", "kt"], &["kt", "kts"]),
    ("lua", &["lua"], &["lua"]),
    ("make", &["make", "makefile"], &["Makefile", "mk"]),
    ("markdown", &["markdown", "md"], &["markdown", "md"]),
    ("nix", &["nix"], &["nix"]),
    ("php", &["php"], &["php"]),
    ("proto", &["proto", "protobuf"], &["proto"]),
    ("python", &["python", "py"], &["py", "pyi"]),
    ("ruby", &["ruby", "rb"], &["Gemfile", "Rakefile", "erb", "rb"]),
    ("rust", &["rust", "rs"], &["rs"]),
    ("scala", &["scala"], &["sbt", "scala"]),
    ("sql", &["sql"], &["sql"]),
    ("svelte", &["svelte"], &["svelte"]),
    ("swift", &["swift"], &["swift"]),
    ("toml", &["toml"], &["toml"]),
    ("tsx", &["tsx", "jsx", "javascript", "js"], &["cjs", "jsx", "mjs", "tsx"]),
    ("typescript", &["typescript", "ts"], &["cts", "mts", "ts"]),
    ("vue", &["vue"], &["vue"]),
    ("xml", &["xml"], &["xml"]),
    ("yaml", &["yaml", "yml"], &["yaml", "yml"]),
    ("zig", &["zig"], &["zig"]),
];
