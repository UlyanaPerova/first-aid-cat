// Example of your own checks and fixes for First Aid Cat (see CUSTOM.md).
//
// Copy this folder next to engine/ so that you have custom/mod.rs, then build as usual:
//     cp -r examples/custom custom
//     cd engine && cargo build --release
// build.rs notices custom/mod.rs and builds it into the program. Without the folder you get the normal build.
// custom/ is in .gitignore: it stays on your computer and never goes into the repository.
//
// No `//!` comments in this file: it is included into the program with include!().

use crate::catalog::{Cat, Unit};
use crate::diag::{fx, http_get_ms, Check, BAD, INFO, OK};

// ── Your checks ──
// Each Unit is one entry on the Metrics tab and one or more rows in its category tab.
// The row gets the program's look by itself (icon, colors, hint, action button), and the cat in the
// balloon talks about it like about any other problem.

pub fn units() -> Vec<Unit> {
    vec![Unit {
        id: "my_split_routing",
        cat: Cat::Net,
        title: "Local and foreign sites",
        about: "Mine: do foreign sites open as well as local ones (VPN on the router)",
        run: check_split_routing,
        default_on: true,
    }]
}

// Sites to compare. Put a site of your country first and two foreign ones after it.
const LOCAL: &str = "https://ya.ru";
const FOREIGN: [&str; 2] = ["https://www.youtube.com/generate_204", "https://telegram.org"];

// A check returns rows. Statuses: OK, WARN, BAD, INFO. A row can have a hint (.hint) and buttons (.fixes).
// Never claim OK when you could not find out: say "couldn't check" with INFO and .unknown().
fn check_split_routing() -> Vec<Check> {
    let ok = |c: Option<u16>| c.is_some_and(|c| c < 500);
    let ((local, _, _), local_ms) = http_get_ms(LOCAL, 6, 64 * 1024, None);
    let foreign: Vec<bool> = FOREIGN.iter().map(|u| ok(http_get_ms(u, 6, 64 * 1024, None).0 .0)).collect();
    if !ok(local) {
        return vec![Check::new("net", "Local and foreign sites", INFO, "couldn't check: no internet")
            .unknown()
            .hint("Even the local site does not open, so this is not about the VPN.")];
    }
    if foreign.iter().all(|f| *f) {
        return vec![Check::new("net", "Local and foreign sites", OK, &format!("both open · local {local_ms:.0} ms"))];
    }
    vec![Check::new("net", "Local and foreign sites", BAD, "foreign sites don't open")
        .hint("The local site opens but the foreign ones don't: the VPN part is broken (often the VPN on the router).")
        .fixes(vec![fx("my_open_router", "Open the router page", None)])]
}

// ── Your fixes ──
// A fix ID must start with my_. Every press goes through the program's single gate: first valid() checks the
// argument, then the confirmation window shows confirm() (or what()), then apply() runs. manual() is the
// command shown to the user as the way to do it by hand.

const ROUTER_PAGE: &str = "http://192.168.1.1";

pub fn valid(id: &str, arg: &str) -> bool {
    matches!(id, "my_open_router") && arg.is_empty()
}

pub fn confirm(_id: &str) -> Option<&'static str> {
    None
}

pub fn what(id: &str) -> &'static str {
    match id {
        "my_open_router" => "I'll open the router's web page in the browser.",
        _ => "I'll do it myself.",
    }
}

pub fn manual(id: &str, _arg: Option<&str>) -> Option<String> {
    match id {
        "my_open_router" => Some(format!("open {ROUTER_PAGE} in a browser")),
        _ => None,
    }
}

pub fn apply(id: &str, _arg: Option<&str>) -> (bool, String) {
    match id {
        "my_open_router" => {
            if crate::fixes::dry_run() {
                crate::fixes::log_dry(&format!("open {ROUTER_PAGE}"));
                return (true, "Opened the router page.".into());
            }
            // external commands only through crate::cmd (timeouts, clean environment)
            let opener = if cfg!(windows) {
                "explorer"
            } else if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let r = crate::cmd::exec(&[opener, ROUTER_PAGE], std::time::Duration::from_secs(10), crate::cmd::FIX);
            if r.code() == 0 {
                (true, "Opened the router page.".into())
            } else {
                (false, format!("Couldn't open it: {}", r.text().trim()))
            }
        }
        _ => (false, format!("Unknown action: {id}")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_known_fix_without_argument() {
        assert!(super::valid("my_open_router", ""));
        assert!(!super::valid("my_open_router", "x"));
        assert!(!super::valid("my_other", ""));
    }
}
