# Interface locale catalogs

Each JSON file is an embedded interface catalog keyed by stable message IDs.

To add a language:

1. Copy `en.json` to `<locale>.json` and translate values without changing keys.
2. Add one entry to `define_languages!` in `src/i18n.rs`.
3. Run the settings and UI tests. Catalog tests reject missing or extra keys.

English is the fallback catalog. Missing runtime keys display the stable key instead
of crashing the application. The language selector automatically cycles through
every entry registered in `Language::ALL`.
