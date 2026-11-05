# Privacy

First Aid Cat has no account, no telemetry, no analytics, no crash reporting and no auto-update. Nothing it finds leaves your computer. This page lists exactly what it reads, which servers it contacts, and what it stores.

## What it reads

To diagnose your system, the app reads the same things you could read yourself. It runs with your user rights, not as administrator.

- **System state:** network interfaces and routes, DNS settings, the system proxy, running processes, disk and memory usage, service status, package manager state, system logs about crashes and updates (Windows Event Log, macOS diagnostic reports), sound, printing and Bluetooth state.
- **Configuration files** where proxies and VPN leftovers hide: shell startup files (`.bashrc`, `.zshrc`, `.profile`, …), `.npmrc`, `.gitconfig`, pip and apt proxy settings, the Firefox `prefs.js`, browser extension lists.
- **AI agent sessions:** the session files of Claude Code, Codex, Gemini CLI, Qwen Code, Kimi Code, Cursor, OpenCode and Antigravity. They are read to show which sessions are alive and how to resume them. Session titles are shown with secrets masked.
- **Digital footprint check:** shell history, AI chat logs, `.env`-style files, VPN configs, SSH private keys and app token files are scanned **locally** for the *shapes* of secrets (API keys, private keys, tokens). The app shows only the **kind** of secret and the **file**. The secret itself is never displayed, logged, cached or sent anywhere. You can switch this check off in the **Metrics** tab.

## Which servers it contacts

Network checks make small requests. No personal data is sent in them: no file contents and no identifiers beyond what any HTTP request carries (your IP address and a generic user agent).

| Purpose | Address |
|---|---|
| Is there internet (connectivity checks) | `connectivitycheck.gstatic.com`, `www.msftconnecttest.com`, `captive.apple.com`, `cp.cloudflare.com`, `www.google.com/generate_204`, `neverssl.com` |
| Reachability without DNS | TCP to `1.1.1.1:443` and `77.88.8.8:443` |
| Does DNS resolve | DNS queries for `one.one.one.one`, `dns.google`, `ya.ru`, `example.com`, plus a random name under `example.com` (live DNS check) sent to your configured resolver and to `1.1.1.1` |
| Is the clock right | the `Date` header from the connectivity endpoints above and `ya.ru` |
| Which address and country the world sees (VPN check) | `www.cloudflare.com/cdn-cgi/trace`, `api.ipify.org`, `ipinfo.io`, `api.country.is` |
| Can AI services be reached | `api.anthropic.com`, `claude.ai`, `api.openai.com`, `chatgpt.com`, `auth.openai.com`, `generativelanguage.googleapis.com`, `cloudcode-pa.googleapis.com`, `accounts.google.com`, `dashscope-intl.aliyuncs.com`, `api.kimi.com`, `api.moonshot.ai`, `api2.cursor.sh`, `cursor.com`, `opencode.ai`, `models.dev`. Only connectivity is tested (an unauthenticated request); **your API keys are never sent** |
| Service status pages | `status.claude.com`, `status.openai.com`, `status.cursor.com` |
| Package repositories reachable | the apt sources configured on your system (Linux) |
| Speed test (only when you press the button) | `speed.cloudflare.com` |

Checks for AI tools you don't have installed are skipped. Every check can be switched off in the **Metrics** tab.

## What it stores

The app keeps no cache of measurements and no history. Measurement results can't end up on your disk, and nothing accumulates.

**In the settings folder** (permanent, owner-only: `0600` files in a `0700` folder, written atomically):

| File | What | Kept |
|---|---|---|
| `ui.json` | window layout, language, which checks are on, widget look | until you delete it |
| `snapshot.json` | your themes and extensions as you saved them (to notice when something breaks); written only when you ask | until you save again |
| `backups/` | a copy of a config file before the app edits it (for example before removing a proxy line), for undo. Lines that look like secrets are masked | 7 days, at most 10 copies |
| `widget.json` (macOS) | the few short rows the WidgetKit widget shows | overwritten each run |

**Measurement results live only in memory and in a temporary per-session folder** that the system clears (a `0700` folder owned by you, not shared with other users):

| What | Where | Gone |
|---|---|---|
| the latest check results, the "checker is alive" mark, the "check now" request (`checks.json`, `checks.alive`, `check_now`), shared between the window and the widget process | session folder | at logout or reboot; also when the app closes and no other copy is running |
| the latest speed test (`speed.json`, `speed.net`) | session folder | same; ignored after 6 hours |
| the last duplicate-file search (`dupes.json`: paths and sizes) | session folder | same |
| speed results for the "slower than usual for this network" hint (last 20, this session only) | memory | when the app closes |
| which files the secret scan already looked at (path, size, time, *kinds* found; no secret values) | memory | when the app closes |

There is no speed history and no scan cache on disk. Files in the session folder older than 12 hours are removed at start (Windows and macOS do not clear their temporary folders at logout). Older versions kept these files in the settings folder; the new version deletes them at start (`checks.json`, `checks.alive`, `check_now`, `speed.json`, `speed.net`, `speed_history.jsonl`, `footprint*.json`, `dupes.json`, and the old `~/.cache/aptechka/dupes.json`).

Locations:
- Linux: settings in `~/.config/aptechka/` (or `$XDG_CONFIG_HOME/aptechka/`); session folder `$XDG_RUNTIME_DIR/aptechka/` (memory-backed, cleared at logout), or `/tmp/aptechka-<uid>/` when that is not available.
- Windows: settings in `%APPDATA%\aptechka\`; session folder `%TEMP%\aptechka\`.
- macOS: settings in `~/Library/Application Support/aptechka/`, the widget container `~/Library/Containers/io.github.aptechka.widget/`; session folder `$TMPDIR/aptechka/`.

To remove all of it, delete the settings folder. The Linux installer's `--uninstall` removes the program, the menu entry and autostart.

## What it changes

Nothing changes until you press a fix button and confirm. Each fix says what it will do and shows the equivalent manual command. Fixes that edit a config file make a backup first.
