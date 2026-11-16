# Your own checks and fixes

First Aid Cat can be built with your own checks and fixes, for things only you have: your router, your NAS, your VPN setup. They live in a folder called `custom/` next to `engine/`. That folder is in `.gitignore`, so it stays on your computer and never goes into the repository.

Your checks look like the built-in ones without any extra work: they get a row in the table with an icon, a color and a hint, an entry on the **Metrics** tab (where you can switch them off), action buttons, and the cat talks about their problems in the balloon. Your fixes go through the same safety gate as the built-in ones: argument check, confirmation window, the command shown for doing it by hand, and `DRY_RUN=1`.

You need to build the program from source for this. The steps below start from nothing.

## 1. Install the tools

- **Rust** 1.89 or newer: install it from [rustup.rs](https://rustup.rs). On Linux and macOS that is one command in the terminal; on Windows, the installer.
- **Git**, to download the source.
- On **Windows**, the C++ build tools that the Rust installer offers. On **macOS**, `xcode-select --install`.

Check that it works:

```sh
cargo --version
```

## 2. Get the source and build it once

```sh
git clone https://github.com/UlyanaPerova/first-aid-cat
cd first-aid-cat/engine
cargo build --release
```

The first build takes a few minutes. The program is `engine/target/release/aptechka` (`aptechka.exe` on Windows). Run it once to make sure it starts.

## 3. Start from the example

The repository has a working example in `examples/custom/mod.rs`. It compares a site of your country with two foreign sites and tells you when only the foreign ones fail (a typical sign of a broken VPN on the router), and it has one fix that opens the router's web page.

From the repository root:

```sh
cp -r examples/custom custom
cd engine
cargo build --release
```

That's all. `build.rs` notices `custom/mod.rs` and builds it into the program. Start the program: the **Network** tab now has a row "Local and foreign sites", and the **Metrics** tab has it in the list. If you delete the `custom/` folder and build again, you get the normal program back.

## 4. Add a check

A check is a function that returns rows. Add it to the list in `units()`:

```rust
Unit {
    id: "my_nas",                 // a unique ID starting with my_ (the Metrics tab remembers it)
    cat: Cat::Net,                // which tab: Net, Ai, Gpu, Dev, Look, Sys
    title: "My NAS",
    about: "Mine: does the NAS answer",
    run: check_nas,
    default_on: true,
},
```

and write the function:

```rust
fn check_nas() -> Vec<Check> {
    let open = std::net::TcpStream::connect_timeout(
        &"192.168.1.10:445".parse().unwrap(),
        std::time::Duration::from_secs(3),
    )
    .is_ok();
    if open {
        vec![Check::new("net", "My NAS", OK, "answers")]
    } else {
        vec![Check::new("net", "My NAS", BAD, "doesn't answer")
            .hint("Is it switched on? Is the cable in?")
            .fixes(vec![fx("my_wake_nas", "Wake it up", None)])]
    }
}
```

Statuses: `OK`, `WARN` (attention), `BAD` (a problem), `INFO` (for information). The first argument of `Check::new` is the tab key: `"net"`, `"ai"`, `"gpu"`, `"dev"`, `"theme"`, `"sys"`.

A few rules keep your check honest:

- If you could not find out, don't say OK. Use `INFO` with `.unknown()` and a hint like "couldn't check".
- Give every network request a timeout. `crate::diag::http_get_ms(url, timeout_s, max_bytes, None)` does that for you and returns the status code and the time.
- Run external programs only through `crate::cmd::exec`: it adds timeouts, caps the output and passes a clean environment.
- A check runs every few minutes. Don't download big files in it, or remember the last result and measure rarely (for example, at most every 30 minutes).

## 5. Add a fix

A fix ID must start with `my_`. You write five small functions; the program calls them in this order when you press the button:

| Function | What it does |
|---|---|
| `valid(id, arg)` | Says whether this ID and argument are allowed. Anything else is refused. |
| `confirm(id)` | Text of the confirmation window for fixes with consequences, or `None`. |
| `what(id)` | One sentence: what will happen. |
| `manual(id, arg)` | The command for doing the same by hand; it is shown to the user. |
| `apply(id, arg)` | Does the fix. Returns `(success, message)`. With `DRY_RUN=1` it must only describe what it would do. |

Look at `examples/custom/mod.rs`: `my_open_router` shows all five.

## 6. Let an AI agent write it for you

If you use Claude Code, Codex or another coding agent, open it in the repository folder and give it a request like this one, with your problem in the middle:

> Read CUSTOM.md and examples/custom/mod.rs. In the folder custom/ (create it from the example if it does not exist), add a check for this problem: **[describe what goes wrong, how you notice it, and what fixes it]**. Follow the rules in CUSTOM.md: honest statuses, timeouts, external commands only through crate::cmd, fix IDs starting with my_ with valid, confirm, what, manual and apply, and DRY_RUN support. Then run `cargo build --release` and `cargo test --release` in engine/ and fix any errors.

Then build and try the fix with `DRY_RUN=1` first:

```sh
DRY_RUN=1 engine/target/release/aptechka --fix my_wake_nas
```

## Translation

The interface has two languages. Write your texts in the language you use; they are shown as written. If you write them in Russian and also want them in the English interface, put the translations next to your code, in `custom/en.txt`, in the same format as `engine/i18n/en/*.txt`:

```text
ru: Мой NAS
en: My NAS

ru: не отвечает уже {} мин
en: no answer for {} min
```

From the repository root, `python3 tools/i18n.py missing custom` prints your strings that have no translation yet, ready to paste into that file, and `python3 tools/i18n.py check` counts them.

## Updating

Your `custom/` folder survives `git pull`. After updating, build again. If the program changed something your code uses, the compiler tells you exactly where.
