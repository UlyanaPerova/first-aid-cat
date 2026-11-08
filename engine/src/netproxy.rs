#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
//! Прокси не только в системе: расширения браузеров (BlancVPN, v2ray, Proxy SwitchyOmega…), ручной прокси
//! в Firefox, переменные http_proxy в файлах оболочки, прокси у git / npm / pip / apt.
//!
//! Правило как у системного прокси: прокси на свой компьютер (127.0.0.1:порт), где никто не слушает, — поломка
//! (VPN выключили, прокси остался); слушает — справка «здесь включён прокси».
//! Только чтение. Чинить файлы браузера, пока он открыт, нельзя — даём подсказку и команду.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::diag::{fx, Check, BAD, INFO, OK, WARN};

fn home() -> PathBuf {
    crate::store::home_dir().unwrap_or_default()
}

/// Строка проверки для найденного прокси: где, адрес.
fn proxy_row(title: &str, where_: &str, addr: &str, fix: Option<crate::diag::Fix>, manual: &str) -> Check {
    use crate::proxyaddr::{redact, ProxyAddr};
    let Some(p) = ProxyAddr::parse(addr) else {
        return Check::new("net", title, INFO, &redact(addr)).hint(&format!("{where_}: настроен прокси."));
    };
    let a = p.shown();
    // свой компьютер — быстрый ответ; чужой сервер — даём больше времени
    let wait = Duration::from_millis(if p.is_local() { 300 } else { 2000 });
    if p.listening(wait) {
        return Check::new("net", title, INFO, &a).hint(&format!("{where_} ходит в интернет через прокси {a}."));
    }
    let why = if p.is_local() {
        "там никто не слушает — VPN выключен, а прокси остался"
    } else {
        "этот прокси-сервер не отвечает"
    };
    let c = Check::new("net", title, BAD, &format!("{a} — там пусто")).hint(&format!("{where_} отправляет интернет через {a}, но {why}. {manual}"));
    match fix {
        Some(f) => c.fixes(vec![f]),
        None => c,
    }
}

// ───────────────────────────── Firefox ─────────────────────────────

fn firefox_profiles() -> Vec<(String, PathBuf)> {
    let h = home();
    let mac = h.join("Library/Application Support");
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| h.join("AppData/Roaming"));
    let bases = [
        ("Firefox", h.join(".mozilla/firefox")),
        ("Firefox (snap)", h.join("snap/firefox/common/.mozilla/firefox")),
        ("Firefox (flatpak)", h.join(".var/app/org.mozilla.firefox/.mozilla/firefox")),
        ("LibreWolf", h.join(".librewolf")),
        ("Floorp", h.join(".floorp")),
        // macOS и Windows: профили лежат в Profiles
        ("Firefox", mac.join("Firefox/Profiles")),
        ("Firefox", roaming.join("Mozilla/Firefox/Profiles")),
        ("LibreWolf", roaming.join("librewolf/Profiles")),
    ];
    let mut out = Vec::new();
    for (name, b) in bases {
        let Ok(rd) = std::fs::read_dir(&b) else { continue };
        for e in rd.flatten() {
            if e.path().join("prefs.js").is_file() {
                out.push((name.to_string(), e.path()));
            }
        }
    }
    out
}

fn pref(prefs: &str, key: &str) -> Option<String> {
    let needle = format!("user_pref(\"{key}\", ");
    let l = prefs.lines().find(|l| l.starts_with(&needle))?;
    Some(l[needle.len()..].trim_end_matches(");").trim_matches('"').to_string())
}

fn check_firefox(out: &mut Vec<Check>) {
    for (browser, prof) in firefox_profiles() {
        // включённые расширения с правом управлять прокси
        if let Ok(t) = crate::store::read_text(prof.join("extensions.json")) {
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(&t) {
                for a in j["addons"].as_array().into_iter().flatten() {
                    let perms = a["userPermissions"]["permissions"].as_array().cloned().unwrap_or_default();
                    let proxy = perms.iter().any(|p| p.as_str() == Some("proxy"));
                    if a["type"] == "extension" && a["active"] == true && proxy {
                        let name = a["defaultLocale"]["name"].as_str().unwrap_or("расширение");
                        out.push(Check::new("net", &format!("{browser}: {name}"), INFO, "включено").hint(
                            "Это расширение может пускать браузер через свой прокси или VPN. Если сайты не открываются только в браузере \
                             (или только в нём открываются) — дело в нём: выключи его на странице about:addons.",
                        ));
                    }
                }
            }
        }
        // ручной прокси в настройках Firefox: network.proxy.type 1 — вручную, 2 — PAC-файл
        let prefs = crate::store::read_text_lossy(prof.join("prefs.js")).unwrap_or_default();
        match pref(&prefs, "network.proxy.type").as_deref() {
            Some("1") => {
                for (hk, pk) in [("network.proxy.socks", "network.proxy.socks_port"), ("network.proxy.http", "network.proxy.http_port")] {
                    if let (Some(h), Some(p)) = (pref(&prefs, hk), pref(&prefs, pk)) {
                        if !h.is_empty() && p != "0" {
                            out.push(proxy_row(
                                &format!("{browser}: свой прокси"),
                                browser.as_str(),
                                &format!("{h}:{p}"),
                                None,
                                "Исправь в Firefox: Настройки → Основные → Параметры сети → «Без прокси» (или «Использовать системные»).",
                            ));
                            break;
                        }
                    }
                }
            }
            Some("2") => {
                let url = pref(&prefs, "network.proxy.autoconfig_url").unwrap_or_default();
                out.push(
                    Check::new("net", &format!("{browser}: прокси по PAC-файлу"), INFO, &crate::proxyaddr::url_host(&url))
                        .hint("Браузер берёт правила прокси из PAC-файла."),
                );
            }
            _ => {}
        }
    }
}

// ───────────────────────────── Chrome и родня ─────────────────────────────

fn chromium_dirs() -> Vec<(&'static str, PathBuf)> {
    let h = home();
    if cfg!(windows) {
        let l = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| h.join("AppData/Local"));
        let r = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| h.join("AppData/Roaming"));
        return vec![
            ("Chrome", l.join("Google/Chrome/User Data")),
            ("Edge", l.join("Microsoft/Edge/User Data")),
            ("Brave", l.join("BraveSoftware/Brave-Browser/User Data")),
            ("Яндекс Браузер", l.join("Yandex/YandexBrowser/User Data")),
            ("Vivaldi", l.join("Vivaldi/User Data")),
            ("Opera", r.join("Opera Software/Opera Stable")),
        ];
    }
    if cfg!(target_os = "macos") {
        let m = h.join("Library/Application Support");
        return vec![
            ("Chrome", m.join("Google/Chrome")),
            ("Edge", m.join("Microsoft Edge")),
            ("Brave", m.join("BraveSoftware/Brave-Browser")),
            ("Яндекс Браузер", m.join("Yandex/YandexBrowser")),
            ("Vivaldi", m.join("Vivaldi")),
            ("Arc", m.join("Arc/User Data")),
            ("Opera", m.join("com.operasoftware.Opera")),
        ];
    }
    let c = h.join(".config");
    vec![
        ("Chrome", c.join("google-chrome")),
        ("Chromium", c.join("chromium")),
        ("Chromium (snap)", h.join("snap/chromium/common/chromium")),
        ("Brave", c.join("BraveSoftware/Brave-Browser")),
        ("Яндекс Браузер", c.join("yandex-browser")),
        ("Vivaldi", c.join("vivaldi")),
        ("Opera", c.join("opera")),
        ("Edge", c.join("microsoft-edge")),
    ]
}

fn check_chromium(out: &mut Vec<Check>) {
    for (browser, dir) in chromium_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for prof in rd.flatten().map(|e| e.path()).filter(|p| p.join("Preferences").is_file()) {
            let mut seen = std::collections::HashSet::new();
            for file in ["Secure Preferences", "Preferences"] {
                let Ok(t) = crate::store::read_text(prof.join(file)) else { continue };
                let Ok(j) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
                let Some(exts) = j["extensions"]["settings"].as_object() else { continue };
                for (id, e) in exts {
                    let enabled = e["state"].as_i64() == Some(1) || (e.get("state").is_none() && e["disable_reasons"].is_null());
                    let has = |k: &str| e[k]["api"].as_array().map(|a| a.iter().any(|x| x.as_str() == Some("proxy"))).unwrap_or(false);
                    let manifest_proxy =
                        e["manifest"]["permissions"].as_array().map(|a| a.iter().any(|x| x.as_str() == Some("proxy"))).unwrap_or(false);
                    if enabled && (has("active_permissions") || has("granted_permissions") || manifest_proxy) && seen.insert(id.clone()) {
                        let name = e["manifest"]["name"].as_str().filter(|n| !n.starts_with("__MSG")).unwrap_or(id.as_str());
                        out.push(Check::new("net", &format!("{browser}: {name}"), INFO, "включено").hint(
                            "Расширение с правом управлять прокси: может пускать браузер через свой VPN. Если сайты не открываются \
                             только в браузере — выключи его в «Расширениях».",
                        ));
                    }
                }
            }
        }
    }
}

// ───────────────────────────── Переменные окружения ─────────────────────────────

/// Файлы оболочки и окружения, где задают прокси для терминала.
/// «путь:номер[:отпечаток]» → (путь, номер, отпечаток). Отпечаток — 8 шестнадцатеричных знаков.
pub(crate) fn split_env_spec(spec: &str) -> Option<(&str, usize, Option<&str>)> {
    let (head, last) = spec.rsplit_once(':')?;
    if last.len() == 8 && last.chars().all(|c| c.is_ascii_hexdigit()) {
        let (file, n) = head.rsplit_once(':')?;
        return Some((file, n.parse().ok()?, Some(last)));
    }
    Some((head, last.parse().ok()?, None))
}

/// «путь:номер» для «закомментировать прокси»: путь — один из файлов оболочки, которые проверяет Аптечка
/// (по настоящему пути), а не любой файл в домашней папке.
pub fn env_spec_ok(spec: &str) -> bool {
    let Some((file, n, _)) = split_env_spec(spec) else { return false };
    if n == 0 {
        return false;
    }
    let Ok(real) = std::fs::canonicalize(file) else { return false };
    env_files().iter().filter_map(|f| std::fs::canonicalize(f).ok()).any(|f| f == real)
}

fn env_files() -> Vec<PathBuf> {
    let h = home();
    let mut files: Vec<PathBuf> =
        [".bashrc", ".profile", ".bash_profile", ".bash_login", ".zshrc", ".zprofile", ".zshenv", ".zlogin"].iter().map(|f| h.join(f)).collect();
    files.push(PathBuf::from("/etc/environment"));
    for dir in [h.join(".config/environment.d"), PathBuf::from("/etc/profile.d")] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
            v.sort();
            files.extend(v);
        }
    }
    files
}

fn check_env(out: &mut Vec<Check>) {
    let h = home();
    for f in env_files() {
        let Ok(t) = crate::store::read_text_lossy(&f) else { continue };
        let short = f.to_string_lossy().replacen(&*h.to_string_lossy(), "~", 1);
        // чинить можно только свой файл (по настоящему пути, через ссылки) и строку верхнего уровня
        let mine = crate::shellcfg::real_in_home(&f, &h).is_some();
        for l in crate::shellcfg::find_proxy_lines(&t) {
            let Some((_, v)) = l.vars.iter().find(|(_, v)| !v.is_empty()) else { continue };
            let n = l.n;
            let (fix, manual) = match (mine, l.top_level && l.only_proxy) {
                (true, false) if !l.only_proxy => {
                    (None, format!("В строке {n} кроме прокси есть другие настройки — убери из неё только прокси в {short} вручную."))
                }
                (true, true) => {
                    let tag = t.lines().nth(n - 1).map(crate::shellcfg::line_tag).unwrap_or_default();
                    (Some(fx("env_proxy_off", "Закомментировать", Some(&format!("{}:{n}:{tag}", f.to_string_lossy())))), String::new())
                }
                (true, false) => (None, format!("Строка {n} внутри условия или функции — поправь её в {short} вручную.")),
                (false, _) => (None, format!("Это системный файл — поправь строку {n} вручную: sudo nano {}", f.display())),
            };
            out.push(proxy_row(
                &format!("Прокси в {short}"),
                &format!("Терминал (строка {n} в {short})"),
                v,
                fix,
                &format!("Из-за этого не работают терминал и ИИ-агенты в нём, хотя браузер работает. {manual}"),
            ));
        }
    }
}

// ───────────────────────────── git, npm, pip, apt ─────────────────────────────

fn check_tools(out: &mut Vec<Check>) {
    if crate::procs::which("git").is_some() {
        for key in ["http.proxy", "https.proxy"] {
            let (code, v) = crate::diag::run(&["git", "config", "--global", "--get", key], 5);
            if code == 0 && !v.trim().is_empty() {
                out.push(proxy_row("git: прокси", "git", v.trim(), Some(fx("git_proxy_off", "Убрать у git", Some(key))), ""));
            }
        }
    }
    let npmrc = crate::store::read_text_lossy(home().join(".npmrc")).unwrap_or_default();
    for l in npmrc.lines() {
        if let Some((k, v)) = l.split_once('=') {
            if matches!(k.trim(), "proxy" | "https-proxy") {
                out.push(proxy_row("npm: прокси", "npm", v.trim(), Some(fx("npm_proxy_off", "Убрать у npm", None)), ""));
                break;
            }
        }
    }
    for p in [home().join(".config/pip/pip.conf"), home().join(".pip/pip.conf"), PathBuf::from("/etc/pip.conf")] {
        // ключ «proxy» целиком (не «proxy-cert»), без закомментированных строк
        let found = crate::store::read_text_lossy(&p).ok().and_then(|t| {
            t.lines().find_map(|l| {
                let l = l.trim();
                let (k, v) = l.split_once(['=', ':'])?;
                (!l.starts_with(['#', ';']) && k.trim() == "proxy" && !v.trim().is_empty()).then(|| v.trim().to_string())
            })
        });
        if let Some(v) = found {
            out.push(proxy_row("pip: прокси", &p.to_string_lossy(), &v, None, "Убери строку proxy из этого файла."));
        }
    }
    if let Ok(rd) = std::fs::read_dir("/etc/apt/apt.conf.d") {
        for e in rd.flatten() {
            let t = crate::store::read_text_lossy(e.path()).unwrap_or_default();
            // «Acquire::http::Proxy "http://…";» — но не «"DIRECT"», «"false"» (это «без прокси») и не Proxy-Auto-Detect
            let hit = t.lines().find_map(|l| {
                let lt = l.trim_start();
                let low = lt.to_lowercase();
                if lt.starts_with("//")
                    || lt.starts_with('#')
                    || !low.starts_with("acquire::")
                    || !low.contains("::proxy ") && !low.contains("::proxy\"")
                {
                    return None;
                }
                let v = lt.split('"').nth(1)?.to_string();
                (!matches!(v.to_lowercase().as_str(), "direct" | "false" | "")).then_some(v)
            });
            if let Some(v) = hit {
                out.push(proxy_row(
                    "apt: прокси",
                    &format!("apt ({})", e.path().display()),
                    &v,
                    None,
                    &format!("Обновления не скачаются. Убери строку: sudo nano {}", e.path().display()),
                ));
            }
        }
    }
}

pub(crate) fn check_other_proxies() -> Vec<Check> {
    let mut out = Vec::new();
    check_firefox(&mut out);
    check_chromium(&mut out);
    check_env(&mut out);
    check_tools(&mut out);
    if out.is_empty() {
        out.push(Check::new("net", "Прокси в браузерах и программах", OK, "нет"));
    }
    out
}

/// Закомментировать строку с прокси в файле оболочки: «путь:номер» (с копией файла рядом: .aptechka.bak).
pub fn comment_env_line(spec: &str) -> (bool, String) {
    let Some((file, n, tag)) = split_env_spec(spec).filter(|(_, n, _)| *n >= 1) else {
        return (false, "Непонятно, какую строку.".into());
    };
    // настоящий файл (если ~/.bashrc — ссылка, правим то, на что она указывает, а ссылку оставляем)
    let Some(real) = crate::shellcfg::real_in_home(Path::new(file), &home()) else {
        return (false, "Этот файл системный (или вне домашней папки) — поправь его вручную.".into());
    };
    let dry = crate::fixes::dry_run();
    if dry {
        crate::fixes::log_dry(&format!("закомментировать строку {n} в {}", real.display()));
    }
    match crate::shellcfg::comment_out_tagged(&real, n, tag, dry) {
        Ok(bak) => (true, format!("Строка {n} закомментирована (копия: {}). Открой новый терминал.", bak.display())),
        Err(e) => (false, e),
    }
}

#[cfg(test)]
mod tests {}

/// Интернет и DNS (Windows и macOS; на Linux — вместе с маршрутом в diag::check_network).
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn check_net_basic(flush_fix: &str) -> Vec<Check> {
    crate::reach::rows(flush_fix, Vec::new())
}

// ───────────────────────────── Страница входа в сеть (кафе, гостиница) ─────────────────────────────

/// Страница входа (captive portal): сеть пускает только на свою страницу «примите условия / введите номер».
/// Спрашиваем адреса проверки подключения (Google, Apple) без следования переадресации: сеть с порталом
/// отвечает переадресацией или своей страницей вместо ожидаемого ответа. Раньше переадресация выполнялась
/// молча, и такая сеть выглядела как «интернет есть».
/// Что ответил адрес проверки подключения.
#[derive(Debug, Clone, PartialEq)]
enum Probe {
    /// ровно то, что должно быть — сеть пускает в интернет
    Clean,
    /// переадресация или чужая страница — сеть подменяет ответ (страница входа); адрес страницы, если дан
    Portal(String),
    /// ответ непонятный
    Odd,
    /// нет ответа
    Silent,
}

/// Адрес из Location — полный: «/login?x» → «http://хост/login?x» (относительный открылся бы не там).
fn absolute(loc: &str, base: &str) -> String {
    if loc.is_empty() || loc.contains("://") {
        return loc.to_string();
    }
    let origin: String = base.splitn(4, '/').take(3).collect::<Vec<_>>().join("/");
    if loc.starts_with("//") {
        format!("http:{loc}")
    } else if loc.starts_with('/') {
        format!("{origin}{loc}")
    } else {
        format!("{origin}/{loc}")
    }
}

fn probe(agent: &ureq::Agent, url: &str, want: Option<&str>) -> Probe {
    let (code, body, loc) = match agent.get(url).call() {
        Ok(r) => {
            let code = r.status();
            let loc = r.header("location").unwrap_or("").to_string();
            let mut b = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::Read::take(r.into_reader(), 20_000), &mut b);
            (code, b, loc)
        }
        Err(ureq::Error::Status(code, r)) => (code, String::new(), r.header("location").unwrap_or("").to_string()),
        Err(_) => return Probe::Silent,
    };
    let ok = match want {
        None => code == 204,
        Some(w) => code == 200 && body.contains(w),
    };
    if ok {
        Probe::Clean
    } else if (300..400).contains(&code) || code == 200 || code == 511 {
        Probe::Portal(absolute(&loc, url))
    } else {
        Probe::Odd
    }
}

/// Решение по ответам: хоть один честный ответ — сеть пускает (страница входа перехватывает всё);
/// страница входа — когда так ответили все, кто ответил, и таких хотя бы двое.
fn captive_verdict(r: &[Probe]) -> Option<Check> {
    if r.contains(&Probe::Clean) {
        return Some(Check::new("net", "Вход в сеть", OK, "не нужен"));
    }
    let portals: Vec<&String> = r.iter().filter_map(|p| if let Probe::Portal(l) = p { Some(l) } else { None }).collect();
    let answered = r.iter().filter(|p| **p != Probe::Silent).count();
    if answered == 0 {
        return None; // нет ответа вовсе — это «нет интернета», о нём скажут строки «Интернет»
    }
    let page = portals.iter().find(|l| l.starts_with("http")).map(|l| l.to_string());
    // адрес пришёл от чужой сети — на кнопке видно, куда она ведёт
    let target = page.as_deref().unwrap_or("http://neverssl.com");
    // настоящий хост — после последнего «@» (до него идёт «логин», которым прячут чужой адрес); при усечении сохраняем конец
    let host = target.split("://").nth(1).and_then(|r| r.split(['/', '?', '#', '\\']).next()).unwrap_or(target);
    let host = host.rsplit('@').next().unwrap_or(host);
    let hc: Vec<char> = host.chars().collect();
    let host = if hc.len() > 40 { format!("…{}", hc[hc.len() - 39..].iter().collect::<String>()) } else { host.to_string() };
    let label = format!("Открыть страницу входа ({})", host);
    let fix = || vec![fx("open_url", &label, Some(target))];
    if portals.len() >= 2 && portals.len() == answered {
        let hint = match &page {
            Some(p) => format!("Сеть просит войти на своей странице (как в кафе или гостинице): {}", crate::diag::trunc(p, 80)),
            None => {
                "Сеть пускает только на свою страницу (как в кафе, гостинице, на вокзале). Открой любой сайт в браузере — появится страница входа."
                    .into()
            }
        };
        return Some(Check::new("net", "Вход в сеть", WARN, "нужен вход на странице сети").hint(&hint).fixes(fix()));
    }
    if !portals.is_empty() {
        return Some(
            Check::new("net", "Вход в сеть", INFO, "возможно, нужен")
                .hint("Один адрес проверки подменён, другой не ответил. Если сайты не открываются — открой страницу входа.")
                .fixes(fix()),
        );
    }
    Some(Check::new("net", "Вход в сеть", INFO, "не понять").hint("Адреса проверки ответили неожиданно."))
}

pub fn check_captive() -> Vec<Check> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .redirects(0)
        .user_agent(concat!("aptechka/", env!("CARGO_PKG_VERSION")))
        .build();
    // (адрес, что должно прийти): 204 без тела у Google, «Success» у Apple — оба сразу, параллельно
    let probes = [("http://connectivitycheck.gstatic.com/generate_204", None), ("http://captive.apple.com/hotspot-detect.html", Some("Success"))];
    let res: Vec<Probe> = std::thread::scope(|sc| {
        let hs: Vec<_> = probes
            .iter()
            .map(|(u, w)| {
                let a = &agent;
                sc.spawn(move || probe(a, u, *w))
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap_or(Probe::Silent)).collect()
    });
    captive_verdict(&res).into_iter().collect()
}

#[cfg(test)]
#[test]
fn captive_rules() {
    use Probe::*;
    let st = |r: &[Probe]| captive_verdict(r).map(|c| c.status);
    assert_eq!(st(&[Clean, Portal("http://x/".into())]), Some(OK.into()));
    assert_eq!(st(&[Portal("http://login/".into()), Portal(String::new())]), Some(WARN.into()));
    assert_eq!(st(&[Portal("http://login/".into()), Silent]), Some(INFO.into()));
    assert_eq!(st(&[Silent, Silent]), None);
    assert_eq!(absolute("/login?a=1", "http://captive.apple.com/hotspot-detect.html"), "http://captive.apple.com/login?a=1");
    assert_eq!(absolute("https://portal.net/x", "http://a/b"), "https://portal.net/x");
}

#[cfg(test)]
#[test]
fn captive_label_shows_real_host() {
    let u = "http://accounts.google.com.signin.verify-your-account.example@evil.test/login";
    let c = captive_verdict(&[Probe::Portal(u.into()), Probe::Portal(u.into())]).unwrap();
    let label = &c.fixes[0].label;
    assert!(label.contains("(evil.test)"), "{label}");
    let long = format!("http://{}.example.org/x", "a".repeat(60));
    let c = captive_verdict(&[Probe::Portal(long.clone()), Probe::Portal(long)]).unwrap();
    assert!(c.fixes[0].label.ends_with(".example.org)"), "{}", c.fixes[0].label);
}

// ───────────────────────────── Батарея ─────────────────────────────

/// Ноутбук сейчас работает от батареи (не от сети). Настольный компьютер без батареи — всегда false.
/// Дёшево: чтение sysfs, один вызов GetSystemPowerStatus, у macOS — pmset (с пределом времени).
pub fn on_battery() -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else { return false };
        let (mut has_battery, mut ac_online) = (false, false);
        for e in rd.flatten() {
            let p = e.path();
            let rd_s = |f: &str| crate::store::read_text(p.join(f)).unwrap_or_default().trim().to_string();
            match rd_s("type").as_str() {
                "Battery" if rd_s("scope") != "Device" => has_battery = true,
                "Mains" | "USB" | "USB_C" | "USB_PD" | "USB_PD_DRP" => ac_online |= rd_s("online") == "1",
                _ => {}
            }
        }
        return has_battery && !ac_online;
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Sps {
            ac: u8,
            flag: u8,
            pct: u8,
            saver: u8,
            life: u32,
            full: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetSystemPowerStatus(s: *mut Sps) -> i32;
        }
        let mut s = Sps { ac: 0, flag: 0, pct: 0, saver: 0, life: 0, full: 0 };
        // ac 0 — не от сети; flag 128 — батареи нет
        return unsafe { GetSystemPowerStatus(&mut s) } != 0 && s.flag & 128 == 0 && s.pct != 255 && s.ac == 0;
    }
    #[cfg(target_os = "macos")]
    {
        let (_, t) = crate::diag::run(&["pmset", "-g", "batt"], 6);
        return pmset_on_battery(&t);
    }
    #[allow(unreachable_code)]
    false
}

/// По выводу `pmset -g batt`: питание от батареи («Now drawing from 'Battery Power'»), и батарея есть.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn pmset_on_battery(t: &str) -> bool {
    t.contains("InternalBattery") && t.contains("'Battery Power'")
}

/// Батарея ноутбука: заряд, заряжается ли от сети, износ. Linux — /sys/class/power_supply, Windows —
/// GetSystemPowerStatus, macOS — pmset. Нет батареи (настольный) — строки нет.
pub fn check_battery() -> Vec<Check> {
    #[cfg(target_os = "linux")]
    {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") else { return out };
        let ac_online = std::fs::read_dir("/sys/class/power_supply").into_iter().flatten().flatten().any(|e| {
            let t = crate::store::read_text(e.path().join("type")).unwrap_or_default();
            // зарядка: блок питания (Mains) или USB-C / USB PD
            matches!(t.trim(), "Mains" | "USB" | "USB_C" | "USB_PD" | "USB_PD_DRP")
                && crate::store::read_text(e.path().join("online")).unwrap_or_default().trim() == "1"
        });
        for e in rd.flatten() {
            let p = e.path();
            let rd_s = |f: &str| crate::store::read_text(p.join(f)).unwrap_or_default().trim().to_string();
            if rd_s("type") != "Battery" || rd_s("scope") == "Device" {
                continue; // мышки и наушники тоже «батареи» — только своя
            }
            let cap: i64 = rd_s("capacity").parse().unwrap_or(-1);
            let status = rd_s("status");
            let full: f64 = rd_s("energy_full").parse().or_else(|_| rd_s("charge_full").parse()).unwrap_or(0.0);
            let design: f64 = rd_s("energy_full_design").parse().or_else(|_| rd_s("charge_full_design").parse()).unwrap_or(0.0);
            let wear = (design > 0.0 && full > 0.0).then(|| full / design * 100.0);
            out.extend(battery_rows(cap, &status, ac_online, wear));
        }
        return out;
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Sps {
            ac: u8,
            flag: u8,
            pct: u8,
            saver: u8,
            life: u32,
            full: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetSystemPowerStatus(s: *mut Sps) -> i32;
        }
        let mut s = Sps { ac: 0, flag: 0, pct: 0, saver: 0, life: 0, full: 0 };
        if unsafe { GetSystemPowerStatus(&mut s) } == 0 || s.flag & 128 != 0 || s.pct == 255 {
            return vec![]; // батареи нет
        }
        let status = if s.flag & 8 != 0 {
            "Charging"
        } else if s.ac == 1 {
            "Not charging"
        } else {
            "Discharging"
        };
        return battery_rows(s.pct as i64, status, s.ac == 1, None);
    }
    #[cfg(target_os = "macos")]
    {
        let (_, t) = crate::diag::run(&["pmset", "-g", "batt"], 6);
        // «Now drawing from 'AC Power'» / «-InternalBattery-0 (id=…)	87%; charging; …»
        let Some(l) = t.lines().find(|l| l.contains("InternalBattery")) else { return vec![] };
        let pct: i64 = l.split('\t').nth(1).and_then(|x| x.split('%').next()).and_then(|x| x.trim().parse().ok()).unwrap_or(-1);
        let ac = t.contains("'AC Power'");
        // состояния pmset: charging, finishing charge, charged, discharging, «AC attached; not charging»
        // (порядок важен: «discharging» и «not charging» тоже содержат «charging»)
        let status = if l.contains("not charging") {
            "Not charging"
        } else if l.contains("discharging") {
            "Discharging"
        } else if l.contains("; charged") {
            "Full"
        } else if l.contains("charging") || l.contains("finishing charge") {
            "Charging"
        } else if ac {
            "Full"
        } else {
            "Discharging"
        };
        return battery_rows(pct, status, ac, None);
    }
    #[allow(unreachable_code)]
    {
        Vec::new()
    }
}

fn battery_rows(cap: i64, status: &str, ac: bool, wear: Option<f64>) -> Vec<Check> {
    let mut out = Vec::new();
    if cap < 0 {
        return out;
    }
    let state = match status {
        "Charging" => "заряжается",
        "Full" if ac => "заряжена, от сети",
        "Full" => "полная",
        "Discharging" => "от батареи",
        _ => "не заряжается",
    };
    // включён в сеть, батарея не полная и не заряжается — блок питания или контроллер (бывает и нарочно:
    // ограничение заряда до 80% — поэтому «внимание», а не «беда»)
    let stuck = ac && status == "Not charging" && cap < 75;
    let st = if stuck || cap < 10 { WARN } else { OK };
    let mut c = Check::new("sys", "Батарея", st, &format!("{cap}%, {state}"));
    if stuck {
        c = c.hint(
            "Ноутбук включён в сеть, а батарея не заряжается. Проверь блок питания и разъём; если включено «ограничение заряда» — это нормально.",
        );
    } else if cap < 10 && status == "Discharging" {
        c = c.hint("Почти разряжена — подключи зарядку.");
    }
    out.push(c);
    if let Some(w) = wear {
        let st = if w < 60.0 { WARN } else { INFO };
        let c = Check::new("sys", "Износ батареи", st, &format!("держит {w:.0}% от новой"));
        out.push(if w < 60.0 {
            c.hint("Батарея заметно износилась: ноутбук без зарядки проработает меньше. Это нормально через несколько лет.")
        } else {
            c
        });
    }
    out
}

// ───────────────────────────── Сигнал Wi-Fi ─────────────────────────────

/// Сила сигнала Wi-Fi (Linux: /proc/net/wireless, уровень в дБм). Слабый сигнал — частая причина
/// «тормозит интернет», которую путают с провайдером.
pub fn check_wifi_signal() -> Vec<Check> {
    let t = crate::store::read_text("/proc/net/wireless").unwrap_or_default();
    let mut out = Vec::new();
    for l in t.lines().skip(2) {
        let f: Vec<&str> = l.split_whitespace().collect();
        let (Some(name), Some(level)) = (f.first(), f.get(3)) else { continue };
        let Ok(dbm) = level.trim_end_matches('.').parse::<f64>() else { continue };
        if dbm >= 0.0 {
            continue; // не в дБм (старые драйверы) — не берёмся судить
        }
        let (st, word) = if dbm >= -60.0 {
            (OK, "хороший")
        } else if dbm >= -72.0 {
            (OK, "средний")
        } else if dbm >= -80.0 {
            (WARN, "слабый")
        } else {
            (BAD, "очень слабый")
        };
        let c = Check::new("net", "Сигнал Wi-Fi", st, &format!("{word} ({dbm:.0} дБм, {})", name.trim_end_matches(':')));
        out.push(if st == OK {
            c
        } else {
            c.hint("Слабый сигнал: интернет будет тормозить и обрываться. Подойди ближе к роутеру или убери преграды между ними.")
        });
    }
    out
}
