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

The app keeps no long-term history of measurements: no speed log, no scan cache in your settings. While it runs, though, the latest results are written to a temporary folder (below). On some systems that folder is on disk, so results can stay there for a while, for example after a crash. This section says exactly where and for how long.

**In the settings folder** (permanent, owner-only: `0600` files in a `0700` folder, written atomically):

| File | What | Kept |
|---|---|---|
| `ui.json` | window layout, language, which checks are on, widget look | until you delete it |
| `snapshot.json` | your themes and extensions as you saved them (to notice when something breaks); written only when you press "Save current as baseline" | until you save again |
| `backups/` | a copy of a config file before the app edits it (for example before removing a proxy line), for undo. Lines that look like secrets are masked | 7 days, at most 10 copies |
| `widget.json` (macOS) | the few short rows the WidgetKit widget shows | overwritten each run |

**In the temporary session folder** (a `0700` folder owned by you, not a symlink, not shared with other users). The window and the widget are separate processes, and this folder is how they share results:

| What | Removed |
|---|---|
| the latest check results, the "checker is alive" mark, the "check now" request (`checks.json`, `checks.alive`, `check_now`) | when the last running copy of the app (window or widget) exits normally |
| the latest speed test (`speed.json`, `speed.net`); ignored after 6 hours | same |
| the last duplicate-file search (`dupes.json`: paths and sizes) | same |
| which problems you chose to ignore (`ignored.json`: check names) | same; also each time the window starts |
| the license texts, when you open Help → Licenses (`licenses/`) | at a later start, once older than 12 hours |

If the app is killed or crashes, these files stay behind. The next start removes any of them older than 12 hours, together with half-written temporary copies (`*.tmp…`) left by an interrupted write. Newer ones are reused or overwritten.

Where the session folder is, and whether the system clears it:

| System | Session folder | On disk? | Cleared by the system |
|---|---|---|---|
| Linux, usual desktop | `$XDG_RUNTIME_DIR/aptechka/` (for example `/run/user/1000/aptechka/`) | no, in memory (tmpfs) | at logout |
| Linux without `$XDG_RUNTIME_DIR` (some minimal or container setups) | `/tmp/aptechka-<uid>/` | depends: on disk unless `/tmp` is tmpfs | at reboot if `/tmp` is tmpfs; otherwise only by `systemd-tmpfiles` or similar, usually after days |
| macOS | `$TMPDIR/aptechka/` (a private per-user folder under `/var/folders/…`) | yes | files unused for a few days, and at reboot |
| Windows | `%TEMP%\aptechka\` | yes | not automatically (only Storage Sense or Disk Cleanup, if you use them) |

**Only in memory**, gone when the app closes:
- speed results for the "slower than usual for this network" hint (the last 20 of this session);
- which files the secret scan already looked at (path, size, time, *kinds* found; never the secret values).

Older versions kept measurement files in the settings folder; the new version deletes them at start (`checks.json`, `checks.alive`, `check_now`, `speed.json`, `speed.net`, `speed_history.jsonl`, `footprint*.json`, `dupes.json`, and the old `~/.cache/aptechka/dupes.json`).

Settings folder:
- Linux: `~/.config/aptechka/` (or `$XDG_CONFIG_HOME/aptechka/`).
- Windows: `%APPDATA%\aptechka\`.
- macOS: `~/Library/Application Support/aptechka/`, plus the widget container `~/Library/Containers/io.github.aptechka.widget/`.

To remove everything, delete the settings folder and the session folder. The Linux installer's `--uninstall` removes the program, the menu entry and autostart.

## Notifications

When you ignore a problem and a new one appears in the same category, the app shows a desktop notification (Linux `notify-send`, macOS Notification Center, Windows toast). It is shown locally; nothing is sent anywhere.

## What it changes

Nothing changes until you press a fix button and confirm. Each fix says what it will do and shows the equivalent manual command. Fixes that edit a config file make a backup first.
