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

Everything stays in your user profile. Files are created with owner-only permissions (`0600`, in a `0700` folder) and written atomically.

| File | What | Kept |
|---|---|---|
| `ui.json` | window layout, language, which checks are on, widget look | until you delete it |
| `checks.json`, `checks.alive` | the last check results, for the widget | overwritten each run |
| `speed.json`, `speed.net`, `speed_history.jsonl` | speed test results, so it can compare with your usual speed | 6 hours (latest result); limited history |
| `footprint-v4.json` | which files were already scanned (path, size, time, *kinds* of secrets found; no secret values) | 30 days |
| `snapshot.json` | your themes and extensions as you saved them (to notice when something breaks) | until you save again |
| `dupes.json` | the last duplicate-file search (paths and sizes) | until the next search |
| `backups/` | a copy of a config file before the app edits it (for example before removing a proxy line). Lines that look like secrets are masked | 7 days, at most 10 copies |
| `widget.json` (macOS) | the text the WidgetKit widget shows | overwritten each run |

Locations:
- Linux: `~/.config/aptechka/` (or `$XDG_CONFIG_HOME/aptechka/`); cache in `~/.cache/aptechka/`.
- Windows: `%APPDATA%\aptechka\`.
- macOS: `~/Library/Application Support/aptechka/`, and the widget container `~/Library/Containers/io.github.aptechka.widget/`.

To remove all of it, delete those folders. The Linux installer's `--uninstall` removes the program, the menu entry and autostart.

## What it changes

Nothing changes until you press a fix button and confirm. Each fix says what it will do and shows the equivalent manual command. Fixes that edit a config file make a backup first.
