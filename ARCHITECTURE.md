# Architecture

First Aid Cat is one Rust crate (`engine/`) that builds a single binary, `aptechka`. On macOS a small Swift WidgetKit extension is added (`macos/`). There are no runtime dependencies beyond the operating system.

```
                 +--------------------------------------------------------------+
                 |  main.rs: event loop (winit), window, accessibility (AccessKit) |
                 +-----------+------------------+-----------------+--------------+
                             |                  |                 |
                    +--------v-------+  +-------v------+  +-------v--------+
                    |  ui.rs         |  |  scene.rs    |  |  widget.rs     |
                    |  XP-style UI   |  |  life.rs     |  |  widget_px.rs  |
                    |  gfx.rs        |  |  screens.rs  |  |  widget_tpl.rs |
                    |  spravka.rs    |  |  (living art)|  |  (desktop      |
                    +--------+-------+  +--------------+  |   widget)      |
                             |                            +-------+--------+
                             | results                            | results
                    +--------v------------------------------------v--------+
                    |  sched.rs: scheduler, one shared result set            |
                    |  runtime-folder results (for the widget), lock          |
                    +--------+---------------------------------------+------+
                             | runs checks                           | runs fixes
          +------------------v-----------------+       +-------------v------------+
          |  Checks                             |       |  fixes.rs                |
          |  diag.rs   (common + Linux)         |       |  validate() gate         |
          |  win.rs    (Windows, PowerShell)    |       |  elevate.rs (pkexec +    |
          |  mac.rs    (macOS)                  |       |   systemd-run sandbox,   |
          |  netproxy.rs proxyaddr.rs reach.rs  |       |   UAC, osascript)        |
          |  agents.rs disks.rs more.rs         |       |  dupes.rs shellcfg.rs    |
          |  footprint.rs shellcfg.rs procs.rs  |       +-------------+------------+
          +------------------+-----------------+                     |
                             |                                       |
                    +--------v---------------------------------------v--------+
                    |  cmd.rs: external commands (timeouts, output caps,      |
                    |          allowlisted environment, Windows job objects)  |
                    |  store.rs: private atomic files, safe reads, locks       |
                    +--------------------------------------------------------+
```

## Modules

| Module | Role |
|---|---|
| `main.rs` | Entry point, command line, winit event loop, window, accessibility tree |
| `ui.rs` | The main window: XP-style layout, tabs, table, menus, dialogs, mouse and keyboard |
| `gfx.rs` | Own drawing toolkit: rectangles, bevels, gradients, icons, TrueType text (skrifa) |
| `spravka.rs` | The "Medical Note from Dr. Meow" page and its export to PNG |
| `scene.rs`, `life.rs`, `screens.rs` | The living picture: layered art, breathing and frame animation, the small monitors that show live data |
| `colorman.rs` | Colour management for the display profile |
| `widget.rs`, `widget_px.rs`, `widget_tpl.rs`, `widget_look.rs` | The desktop widget: data, pixel-exact rendering of three looks, hit areas, size steps with the mouse wheel; `widget.json` for macOS |
| `widget_anim.rs` | Widget animation: blink frames, heart bubble, sparks, ear pull with a blush face, drawn loops (steam, ear twitch); frames are pure functions of time |
| `sched.rs` | Scheduler: each check has its own interval; runs in parallel with timeouts and panic isolation; one shared result set for the window and the widget |
| `catalog.rs` | Catalogue of checks by category (what can be switched on and off) |
| `diag.rs` | Common and Linux checks: network, VPN, clock, graphics, sessions of Claude Code, speed test, CLI output |
| `win.rs`, `mac.rs` | Windows and macOS checks |
| `netproxy.rs`, `proxyaddr.rs` | Proxies everywhere (system, browsers, terminal, git/npm/pip/apt); one parser for every proxy address; secret masking |
| `reach.rs` | Internet and DNS reachability, live DNS query |
| `agents.rs` | AI agents other than Claude: reachability, status pages, live sessions |
| `procs.rs` | Process list of the current user (Linux `/proc`, macOS `ps`, Windows NT API) |
| `disks.rs` | Mount points (statvfs with timeouts), S.M.A.R.T. health |
| `more.rs` | Frequent Linux problems: sound, Bluetooth, printing, keyboard layouts, packages, memory |
| `footprint.rs` | Digital footprint: secrets in plain text and who can read them |
| `dupes.rs` | Exact duplicate files |
| `shellcfg.rs` | Reading and safely editing shell startup files |
| `fixes.rs` | All fix actions, the validation gate, resource locks, manual equivalents |
| `elevate.rs` | The only way to run something as administrator |
| `cmd.rs` | The only way to run external commands |
| `store.rs`, `prefs.rs` | Files on disk: private settings folder, per-session runtime folder for measurement data, atomic writes, safe reads, settings |
| `cover.rs` | Linux: whether the widget is fully covered by other windows (asks the window manager), so its animation sleeps while nobody can see it. On Windows the same is done in `widget.rs` with `WindowFromPoint` |
| `notify.rs` | Desktop notification when a new problem appears in a category where something is ignored |
| `single.rs` | One instance per user; a second launch asks the first to show its window (the widget has its own lock, `widget.lock`) |
| `limits.rs` | AI subscription limits and token usage: Codex limits and tokens from its session logs, Claude Code limits from its status line (`--statusline`, connected in `~/.claude/settings.json`), Claude tokens from its logs; logs are read incrementally |
| `tray.rs` | The AI limits icon in the panel (`--tray`): StatusNotifierItem over D-Bus on Linux, a notification-area icon with the number drawn on it on Windows; the popup with limits; its own autostart entry |
| `clip.rs` | Clipboard without GTK |
| `i18n.rs` | Interface language (see below) |
| `quips.rs` | What the cat says in the balloon and the widget |
| `looksnap.rs` | Windows and macOS: the theme baseline ("Save current as baseline") and "Restore" |
| `licenses.rs` | License texts and the custom-checks guide embedded in the binary, opened from the Help menu |
| `art.rs` + `build.rs` | Pictures from `art/layers` embedded into the binary at build time |

A local build can add its own checks and fixes from an optional, untracked `custom/` folder next to `engine/` (`build.rs` turns on `cfg(aptechka_custom)` when `custom/mod.rs` exists). Its checks join the catalogue, its fixes use IDs starting with `my_` and still pass through `fixes::validate`. Release builds and CI never have this folder.

## Rendering

The window is drawn in software into a `softbuffer` surface. A patched copy of softbuffer 0.4.8 is in `engine/vendor/softbuffer`: an alpha channel, the display colour space and reused frame buffers on macOS, and size checks instead of panics. Every change is in `softbuffer-0.4.8.patch` next to it. Only changed regions are redrawn. The art scene is composited from PNG layers with `tiny-skia`. The widget is drawn on a small pixel canvas and scaled by whole numbers, so it stays crisp at any display scale.

## Checks and the scheduler

A check is a function that returns rows (`Check`: title, status `ok/warn/bad/info`, value, detail, possible fixes). The scheduler runs each check on its own interval and also on events (network change, wake from sleep). Every check runs in a worker with a deadline, behind `catch_unwind`. Results go into one shared set, used by the window, the widget and a file in the per-session runtime folder (`store::runtime_dir()`, memory-backed or cleared by the system; never the settings folder). A cross-process lock makes sure that only one process runs checks, even when the window and the widget are separate processes.

## Fixes

A fix is an ID plus an optional argument. `fixes::validate` is the single gate: it knows the argument shape for every ID. A fix that needs administrator rights goes through `elevate.rs`, which builds the whole command from absolute, root-owned paths and runs it in a sandbox. On Windows it uses PowerShell `-EncodedCommand`; on macOS, `osascript` with the command passed as an argument. Every fix returns `(success, message)` and has a manual equivalent that is shown to the user.

## Language

The program keeps working in Russian internally: checks and the logic that compares rows use Russian strings. Translation happens only at the display boundary: text drawing, window titles, the accessibility tree, terminal output and `widget.json`. `i18n.rs` looks strings up in the tables `engine/i18n/en/*.txt` (exact strings, templates with `{}`, plural forms). `tools/i18n.py check` fails if any user-visible Russian string in the code has no translation.
