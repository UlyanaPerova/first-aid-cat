# Notes for AI coding agents

This file is for automated coding assistants (and humans) who change this repository.

## Ground rules

- **Never run fixes against the machine you are working on.** Fixes change system state: proxies, services, files. Use `DRY_RUN=1 aptechka --fix …`, `aptechka --diag`, or a disposable VM. Tests must use temporary `HOME`/`XDG_*` folders.
- **No secrets in the repository.** Keys go into an untracked `.env`. Never paste a key into code, tests or docs; build test keys at runtime from fragments.
- **No personal data:** no usernames, home paths, host names or IP addresses in code, tests, docs or screenshots.
- **One way in:**
  - external commands go through `cmd.rs`;
  - elevation through `elevate.rs`;
  - state files through `store.rs`;
  - fix IDs and arguments through `fixes::validate`.
  Don't add a second path.
- **Treat everything read from the system as hostile:** file contents, process names, session titles, Wi-Fi names, command output. Before display, use `diag::term_safe` and `footprint::mask_secrets` / `proxyaddr::redact`. Open files with the `store` readers (`O_NOFOLLOW`, regular files only, size caps).
- **A check must not lie.** If a probe failed or timed out, say so ("couldn't check"). Don't report OK.
- **Keep all three platforms building.** Gate platform code with `cfg`, run Clippy for the Windows target, and don't break the macOS paths you can't run.

## Strings and translation

The program works in Russian internally, and logic compares Russian strings. English is applied at display time (`engine/src/i18n.rs`).

- New user-visible text is written in Russian in the code.
- Add its English to `engine/i18n/en/<module>.txt` (`ru:` / `en:` pairs; `{}` for substituted values; `one|few|many` → `singular|plural` for numbers). Follow `engine/i18n/GLOSSARY.md`.
- Strings that are never shown (logic, layer names, debug logs) go to `engine/i18n/ignore/<module>.txt`.
- Translate parts **before** they are joined or cut. A joined or truncated phrase can't be found in the table.
- Checks and logic produce Russian text only; don't call `i18n::t` while building a check's value or hint. A phrase translated early stays in the old language after the user switches, and in the results file shared with the widget.
- `python3 tools/i18n.py check` (from the repository root) must print `0`, and `python3 tools/i18n_live.py` must report nothing left in Russian.

## Before you finish

```sh
cd engine
cargo fmt && cargo clippy --release && cargo clippy --release --target x86_64-pc-windows-gnu && cargo test --release
python3 ../tools/i18n.py check
```

Code style: `rustfmt.toml` (line width 150). Comments explain *why*, briefly.
