# Security

## Reporting a vulnerability

Please report security problems privately through GitHub: go to **Security → Report a vulnerability** in this repository. Do not open a public issue for them. Include the operating system, the version, and steps to reproduce. You should get an answer within a week.

## Security model

First Aid Cat reads a lot of sensitive local state, and some of its fixes need administrator rights. That makes it a tempting target, so it is built to never become a new source of risk.

### Running as the user

- The window, the checks and the scheduler run with normal user rights. Checks only read.
- Child processes get an **allowlisted** environment. Variables like `GH_TOKEN`, `DATABASE_URL` or `*_PASSWORD` are not passed to the tools the app runs.
- Text that comes from other programs or files (session titles, file names, Wi-Fi names, process names) is stripped of terminal control sequences before it is printed. Secrets are masked before display or logging: keys, tokens, `user:password@` in URLs, `Authorization` headers, cookies, `curl -u`.

### Fixes

- Every fix ID and argument goes through **one validation gate** (`fixes::validate`). Unknown IDs, unexpected arguments and arguments with control characters are refused, whether they come from the UI, the command line or the cache.
- A fix that needs administrator rights runs **only** an absolute path from a system directory. The binary and every directory above it must be owned by root and not writable by group or others. The fix runs under its own name, so multicall binaries behave correctly.
- **Linux:** elevation is `pkexec systemd-run --pipe --wait` with a sandbox profile: `NoNewPrivileges`, `PrivateTmp`, `ProtectHome=read-only`, `RestrictSUIDSGID`, `ProtectKernelModules` and more. The package-manager profile is looser only where apt and dnf need it. On a system without `systemd-run` (or where it isn't owned by root) such fixes are refused instead of running without the sandbox; the manual command is still shown.
- **Windows:** elevated PowerShell is passed as `-EncodedCommand`, so there is no quoting to break. Processes are killed through a handle after checking the executable path.
- **macOS:** `osascript … with administrator privileges` gets the command as a separate argument, never spliced into AppleScript source.
- **Processes** are stopped by identity, never by a bare PID. Linux uses `pidfd` plus a start-time check; Windows uses a process handle plus the image path; macOS checks the start time. A recycled PID is never killed.
- **Duplicate files** are moved to the trash only after a byte-by-byte comparison. MD5 is used only to group candidates.
- `DRY_RUN=1` runs any fix in "show only" mode.

### Files

- State files are written atomically (a temporary file with an unpredictable name, `fsync`, then rename), with mode `0600` in a folder created `0700`. The app refuses a state folder owned by someone else.
- Measurement results (check results, speed test, duplicate search) are not kept in the state folder. They live in memory and in a per-session runtime folder (`$XDG_RUNTIME_DIR/aptechka`, `$TMPDIR/aptechka`, `%TEMP%\\aptechka`) that is `0700`, owned by the user and not a symlink. It is in memory on usual Linux desktops (cleared at logout) but on disk in the Linux `/tmp` fallback, on Windows and on macOS; the app empties it when the last copy exits, and the next start removes leftovers older than 12 hours (see PRIVACY.md). The state folder keeps only settings, the saved theme baseline and edit backups.
- Files from the user's home are opened non-blocking, must be regular files and are size-capped, so a planted FIFO, device or huge file can't hang a read or exhaust memory. Symlinks to regular files are followed on purpose (dotfiles are often symlinks); a few sensitive reads use `O_NOFOLLOW` (`store::read_capped`). Windows has no equivalent of `O_NOFOLLOW`, so links and junctions are followed there.
- Cross-process locks keep two copies of the app from running the same fix, or two scans, at once.

### Robustness

- Every check runs with a timeout in a panic-isolated worker. A hung or crashing check marks its row and does not freeze the window.
- External command output is capped (4 MB), and timed-out commands are killed together with their children (Windows uses a job object).
- Parsers for untrusted input were fuzzed during the audits: proxy addresses, secret detection, shell config parsing. They are covered by regression tests.

## Known limitations

- **The binaries are not code-signed.** The macOS app has an ad-hoc signature, and the Windows executable has no Authenticode signature. Your system will warn about an unknown publisher. Build from source if you prefer.
- Checks are not sandboxed separately from the UI. They run with your user rights, as any program you start does.
- State files (and the temporary session files) are protected by file permissions, not encrypted. Anything running as your user can read them, just as it can read the original files the checks look at.
- Network checks go directly, not through your proxy. They are meant to find out whether the network itself works.
- On Linux the window uses X11. Wayland sessions run it through XWayland.

## Audits

Before release, the code went through several rounds of security review with proof-of-concept tests. The reviews focused on TOCTOU races, symlink attacks, path traversal, privilege escalation, command injection, unsafe deserialization and denial of service. Each finding was fixed together with a regression test.
