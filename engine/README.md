# engine

The Rust crate that builds the `aptechka` binary (First Aid Cat): the main window, the desktop widget, the checks and the fixes.

```sh
cargo build --release            # target/release/aptechka
cargo test --release
cargo clippy --release --all-targets -- -D warnings
python3 ../tools/i18n.py check   # every user-visible string has an English translation
python3 ../tools/notices.py --check   # THIRD_PARTY_NOTICES matches Cargo.lock
```

How the code is organised: [ARCHITECTURE.md](../ARCHITECTURE.md). Rules for changes: [AGENTS.md](../AGENTS.md). Your own checks and fixes: [CUSTOM.md](../CUSTOM.md).

- `src/` — the code; `i18n/en/` — English for the Russian strings, `i18n/ignore/` — strings that are never shown.
- `assets/` — fonts and their licenses.
- `vendor/softbuffer/` — a patched copy of softbuffer (see `vendor/softbuffer/АПТЕЧКА.md`).
- `build.rs` — embeds the art from `../art/layers`, the license texts and the guide; turns on the optional `custom/` module.
