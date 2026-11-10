# Testing

## Automated checks

GitHub Actions runs them on every push and pull request ([ci.yml](.github/workflows/ci.yml)):
- Linux: formatting, Clippy for Linux and Windows, tests, translation coverage;
- Windows and macOS: tests. The Windows runner tests with its default MSVC toolchain, while releases are built with `x86_64-pc-windows-gnu`; no code depends on that difference, and the GNU build is linted on Linux;
- macOS: a build of the app with its WidgetKit widget;
- weekly: `cargo audit` for new advisories on dependencies, with the exceptions and their reasons in `engine/.cargo/audit.toml`.

Run the same checks locally in `engine/`:

```sh
cargo fmt --check
cargo clippy --release --all-targets                                   # 0 warnings
cargo clippy --release --all-targets --target x86_64-pc-windows-gnu    # Windows code paths
cargo test --release
python3 ../tools/i18n.py check                           # 0 strings without translation
python3 ../tools/i18n.py unused                          # translations whose Russian text is gone from the code
```

The unit tests cover the parts where a mistake is dangerous or silent:

- **Fix gate:** unknown IDs and malformed arguments are refused, and every action runs only on its own operating system.
- **Elevation:** only root-owned system binaries are run, and multicall binaries keep their name.
- **Processes:** a recycled PID or a different start time is never killed.
- **Files:** atomic private writes, refusal to read FIFOs and other non-regular files (and to follow symlinks where asked), size caps, UTF-16 and BOM settings files, lock files.
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

## Developer environment variables

These are for development and for producing screenshots. Normal use needs none of them. They are read in release builds too, so the same binary that ships can be checked.

| Variable | What it does |
|---|---|
| `DRY_RUN=1` | Fixes only log the commands they would run |
| `UI_K=2` | Interface scale (default: the monitor's scale) |
| `UI_SHOT=file.png` | Draw the whole window into a PNG without opening it. With `UI_H` (window height), `T` (moment of the animation, seconds), `UI_FAKE=ok\|fix` (sample data instead of real checks), `UI_DO="fix0;speed;menu2;sessions;heal;about;tab1"` (open things before the shot) |
| `WIDGET_SHOT=file.png` | The same for the widget. `WIDGET_HOVER=1` draws it as if the mouse were over it |
| `TRAY_SHOT=file.png` | Draw the AI limits popup into a PNG with the real numbers, without the panel |
| `WIDGET_MAC=dir` | Write the `widget.json` the macOS widget reads, on any system |
| `WIDGET_CONCEPT=1\|warn\|bad` | Fixed sample data for comparing the widget with its concept art |
| `WIDGET_ANIM_LOG=1` | The widget logs which animation frames it draws |
| `FRAMES=dir N=200` | Record N frames as PNG without a window. `CLICK=1.5:84,146;4:62,243` pokes the cat (second:cell x,y) |
| `ART=dir` | Load art layers from a folder instead of the built-in ones. For the artist: a broken file stops the program |
| `SCALE`, `FPS`, `SEED` | Cell size, frames per second, random seed (repeatable runs) |
| `TROUBLE=1` | The cat is worried all the time |
| `APP_ANIM=0` | No window animation |
| `SCHED=0`, `CORES=8-15` | Linux: skip the lower priority and timer slack; pin the window to these CPU cores |
| `NODIAG=1`, `DIAG_WAIT=s` | Skip the checks; wait for the checks before recording frames |
