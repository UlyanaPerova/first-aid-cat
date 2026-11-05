# Testing

## Automated checks

GitHub Actions runs them on every push and pull request ([ci.yml](.github/workflows/ci.yml)):
- Linux: formatting, Clippy for Linux and Windows, tests, translation coverage;
- Windows and macOS: tests;
- macOS: a build of the app with its WidgetKit widget;
- weekly: `cargo audit` for new advisories on dependencies, with the exceptions and their reasons in `engine/.cargo/audit.toml`.

Run the same checks locally in `engine/`:

```sh
cargo fmt --check
cargo clippy --release                                   # 0 warnings
cargo clippy --release --target x86_64-pc-windows-gnu    # Windows code paths
cargo test --release
python3 ../tools/i18n.py check                           # 0 strings without translation
```

The unit tests cover the parts where a mistake is dangerous or silent:

- **Fix gate:** unknown IDs and malformed arguments are refused, and every action runs only on its own operating system.
- **Elevation:** only root-owned system binaries are run, and multicall binaries keep their name.
- **Processes:** a recycled PID or a different start time is never killed.
- **Files:** atomic private writes, refusal to follow symlinks or read FIFOs, size caps, UTF-16 and BOM settings files, lock files.
- **Secrets:** detection of real key shapes without false alarms on examples, masking in URLs, headers, `curl -u` and cookies, no slowdown on pathological input, masking in backups.
- **Shell configs:** multi-line quotes and comments are not taken for settings; history editing works for bash continuation lines, fish, symlinks and non-UTF-8 files.
- **Duplicates:** nothing is trashed without a byte-by-byte match.
- **Translation:** templates, plural forms, and consistency of the tables.
- **Command runner:** output is kept when a background child holds the pipe, and the environment is allowlisted.

Tests never touch real user files. They use temporary `HOME` and `XDG_*` folders.

## Real systems

Fixes change real system state, so they are never tested on a developer's own machine. Every fix and every check that can break something is exercised in **disposable virtual machines**:

- **Linux:** Ubuntu 24.04 in a throwaway VM (Multipass);
- **Windows:** Windows 11 in a throwaway QEMU/KVM VM with Secure Boot and TPM;
- **macOS:** a real Mac, with fixes run as `DRY_RUN=1` unless the fix is harmless.

Each scenario breaks the system first: set a dead proxy, leave a VPN leftover, pause a printer, plant a symlink, start a process with a reused name, and so on. Then it checks that the right row turns red, that the fix repairs it, and that a second run is a no-op.

On the host machine only read-only modes are used: `aptechka --diag`, and `DRY_RUN=1 aptechka --fix …`.

## UI

The window and every widget look are checked at **×1 and ×2 display scale**, in both languages. Translated text has to fit, and nothing may be clipped or blank. Screenshots are taken from the VMs. The accessibility tree is checked with a screen-reader client (AT-SPI on Linux).
