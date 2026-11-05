<h1 align="center"><i>CUTE BUT USELESS</i></h1>

<p align="center">
  <img alt="Linux" src="https://img.shields.io/badge/Linux-supported-2e7d32?logo=linux&logoColor=white">
  <img alt="Windows" src="https://img.shields.io/badge/Windows-supported-1565c0?logo=windows&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-supported-424242?logo=apple&logoColor=white">
  <img alt="License: Apache-2.0" src="https://img.shields.io/badge/code-Apache--2.0-6a1b9a">
</p>

<p align="center">
  <img src="docs/screenshots/main.png" alt="First Aid Cat main window" width="820">
</p>

These days everyone checks their computer by asking a terminal AI agent, and so did I. Then one day the internet stopped working, and nothing I tried would bring it back. Once I'd fixed it, I wrote a small script that would rescue me next time and get me back to the quiet harbour of the all-knowing AI. The project grew into something much bigger, and now it runs on all the major platforms.

**First Aid Cat** (in Russian, *Аптечка*) is a desktop health checker with a doctor cat. It watches the things that usually break at the worst moment: the network, the VPN and its leftovers, proxies, the connection to AI assistants, disks, memory, updates, themes and drivers. It explains what is wrong in plain words, and it offers a fix only where a fix is safe.

It is a single native binary written in Rust. There is no Electron and no GTK. It works offline, and its interface is available in English and Russian.

## Contents

- [What it checks](#what-it-checks)
- [Fixes](#fixes)
- [Widgets](#widgets)
- [Install](#install)
- [Command line](#command-line)
- [Languages](#languages)
- [Privacy](#privacy)
- [Architecture](#architecture)
- [Building from source](#building-from-source)
- [Documentation](#documentation)
- [License](#license)

## What it checks

The checks are grouped into tabs. Each one can be switched off in the **Metrics** tab.

| Tab | What it looks at |
|---|---|
| **Network** | Internet and DNS, including a live DNS query; a system proxy left behind by a VPN that is off; proxies in browsers, extensions, the terminal, git, npm, pip and apt; VPN tunnels and the address the world sees; network latency and jitter; captive portals (cafe and hotel sign-in pages); Wi-Fi signal |
| **AI** | Whether Claude, Codex, Gemini CLI, Qwen Code, Kimi Code, Cursor, OpenCode and Antigravity can reach their servers; the status pages of those services; which agent sessions are alive in your terminals, and the command to resume one |
| **Graphics** | What draws the screen, the NVIDIA driver and Secure Boot, GPU temperature and load |
| **Devices** | Sound and microphone, Bluetooth, printing (paused printers, stuck queues), keyboard layouts, battery wear, disk health (S.M.A.R.T.) |
| **Themes** | Broken themes and icons, GNOME extensions, a dock that vanished after a full-screen window |
| **System** | Clock accuracy and time sync, free space and inodes, memory pressure, failed services, interrupted package installs, Windows Update errors, crashes and freezes, resource-hungry apps, duplicate files |
| **Digital footprint** | What a trojan running as you could take: API keys in shell history, in AI chat logs and in config files; VPN credentials; SSH keys without a passphrase; app login tokens; whether the disk is encrypted. Only the kind of secret and the file are shown, never the secret itself |

A speed test (download, upload, ping) runs only when you ask for it.

## Fixes

Rows with a known remedy have a button. Before anything runs, every action goes through a single validation gate. Actions that need administrator rights ask the system for them (pkexec, UAC or the macOS password dialog) and run in a restricted sandbox.

Examples of fixes:

- turn off a proxy left behind by a VPN;
- restart a VPN client that hangs;
- flush the DNS cache;
- re-enable a paused printer and clear a stuck queue;
- restore the dock;
- reset the Windows Update components;
- empty the trash and thumbnail cache;
- move exact duplicate files to the trash (only after a byte-by-byte comparison);
- lock credential files so other users can't read them;
- remove lines with keys from your shell history.

Every fix also shows the equivalent manual command. You can try any fix without changing anything by setting `DRY_RUN=1`.

## Widgets

A small desktop widget shows the essentials: network, VPN, AI, CPU and disk. Click it to open the main window. Pick its look and size in the **Widget** menu.

| Look | Medium | Small |
|---|---|---|
| **Window** | <img src="docs/screenshots/widget_window.png" alt="Window widget, medium" height="170"> | <img src="docs/screenshots/widget_window_small.png" alt="Window widget, small" height="170"> |
| **Monitor** | <img src="docs/screenshots/widget_monitor.png" alt="Monitor widget, medium" height="190"> | <img src="docs/screenshots/widget_monitor_small.png" alt="Monitor widget, small" height="190"> |
| **Glass** | <img src="docs/screenshots/widget_glass.png" alt="Glass widget, medium" height="170"> | <img src="docs/screenshots/widget_glass_small.png" alt="Glass widget, small" height="190"> |

On **macOS** the widget is a native WidgetKit widget (small and medium). Add it through *Edit Widgets* on the desktop.

<p align="center">
  <img src="docs/screenshots/widget_macos.png" alt="macOS WidgetKit widget" width="420">
</p>

<p align="center">
  <img src="docs/screenshots/medical_note.png" alt="Medical note from Dr. Meow" width="620">
</p>

**Help → Medical Note from Dr. Meow** turns the current state into a (joking) clinic certificate that you can save as an image.

## Install

Download the archive for your system from [Releases](../../releases).

**Linux** (x86-64; glibc 2.35 or newer: Ubuntu 22.04+, Debian 12+, Fedora 36+; X11 or XWayland):

```sh
tar xf first-aid-cat-0.9.0-linux-x86_64.tar.gz
cd first-aid-cat-0.9.0-linux-x86_64
./install.sh              # installs to ~/.local/bin, adds a menu entry and starts the widget at login
./install.sh --no-widget  # same, without the widget at login
./install.sh --uninstall
```

**Windows** (10 and 11, x86-64): unpack the zip and run `install.cmd`, or just start `aptechka.exe`. The binary is not signed yet, so SmartScreen may warn about an unknown publisher.

**macOS** (14 Sonoma or later, Apple silicon): unpack the zip and move the app to *Applications*. The app has an ad-hoc signature, not a Developer ID, so on first launch you need to right-click it and choose *Open*.

## Command line

```text
aptechka                 open the window
aptechka --diag          all checks as a table in the terminal
aptechka --sessions      AI agent sessions and how to resume them
aptechka --speed         speed test in the terminal
aptechka --widget        only the desktop widget (Linux, Windows)
aptechka --fix ID [ARG]  run one fix; DRY_RUN=1 shows what would happen
```

## Languages

The interface follows the system language: Russian on a Russian-language system, English otherwise. You can switch it in **View → Language**. The environment variable `APTECHKA_LANG=en|ru` overrides both.

## Privacy

First Aid Cat has no accounts, no telemetry and no analytics. Everything it finds stays on your computer. To check the network it contacts a few well-known public endpoints (connectivity checks, DNS, the status pages of AI services, and IP lookup). [PRIVACY.md](PRIVACY.md) lists every address and every file the app writes.

## Architecture

The program is a single Rust binary:

- winit window, software rendering through softbuffer;
- its own small widget toolkit and text renderer;
- a background scheduler that runs checks in parallel with timeouts and caches the results for the widget;
- a fix engine with one validation gate and a sandboxed elevation path.

[ARCHITECTURE.md](ARCHITECTURE.md) has the module map.

## Building from source

You need Rust 1.85 or newer.

```sh
cd engine
cargo build --release              # Linux or native
cargo build --release --target x86_64-pc-windows-gnu   # Windows from Linux (needs mingw-w64)
macos/build_app.sh                 # macOS app + WidgetKit widget (needs Xcode)
cargo test --release
python3 tools/i18n.py check        # every user-visible string has a translation
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — how the code is organised
- [SECURITY.md](SECURITY.md) — security model and how to report a vulnerability
- [PRIVACY.md](PRIVACY.md) — what is read, what is sent, what is stored
- [TESTING.md](TESTING.md) — how changes are tested
- [AGENTS.md](AGENTS.md) — notes for AI coding agents working on this repository

## License

The **source code** is licensed under the [Apache License 2.0](LICENSE).

The **images** are not covered by that licence. This includes the cat character, the background art, the widget art, the icons and every other picture in the program and in this repository. Using them outside this program is **not permitted**. See [LICENSE-ART.md](LICENSE-ART.md), [NOTICE](NOTICE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
