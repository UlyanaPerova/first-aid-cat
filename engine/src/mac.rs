//! Проверки macOS. Данные — встроенными командами (scutil, networksetup, df, ps, tmutil), без лишних прав.
//! Устройство — с настоящего Mac (2026-10-31, macOS 26.2, arm64): VPN там — сетевые службы
//! (Outline/LibertyVPN, ProtonVPN, Happ, BlancVPN), состояние видно в `scutil --nc list`.
#![cfg(target_os = "macos")]

use std::time::Duration;

use crate::diag::{fx, run, Check, BAD, INFO, OK, WARN};

/// `scutil --proxy` → словарь «ключ : значение».
/// Настройки прокси macOS. None — scutil не ответил (не «прокси выключен»).
fn scutil_proxy() -> Option<std::collections::HashMap<String, String>> {
    let r = crate::diag::run_ex(&["/usr/sbin/scutil", "--proxy"], 10);
    if r.code() != 0 || !r.text().contains('{') {
        return None;
    }
    Some(r.text().lines().filter_map(|l| l.split_once(" : ").map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect())
}

/// Ни один прокси не включён (для проверки результата «выключить прокси»).
pub fn proxy_all_off() -> bool {
    scutil_proxy().is_some_and(|d| {
        ["HTTPEnable", "HTTPSEnable", "SOCKSEnable", "ProxyAutoConfigEnable"].iter().all(|k| d.get(*k).map(String::as_str) != Some("1"))
    })
}

// ───────────────────────────── Сеть ─────────────────────────────

pub(crate) fn check_proxy() -> Vec<Check> {
    let Some(d) = scutil_proxy() else {
        return vec![Check::new("net", "Системный прокси", INFO, "не проверить").hint("scutil --proxy не ответил.")];
    };
    let mut out = Vec::new();
    let mut any = false;
    for (kind, en, host, port) in [
        ("HTTP", "HTTPEnable", "HTTPProxy", "HTTPPort"),
        ("HTTPS", "HTTPSEnable", "HTTPSProxy", "HTTPSPort"),
        ("SOCKS", "SOCKSEnable", "SOCKSProxy", "SOCKSPort"),
    ] {
        if d.get(en).map(String::as_str) != Some("1") {
            continue;
        }
        any = true;
        let h = d.get(host).cloned().unwrap_or_default();
        let p = d.get(port).cloned().unwrap_or_default();
        let scheme = if kind == "SOCKS" { "socks5" } else { "http" };
        let off = vec![fx("mac_proxy_off", "Выключить системный прокси", None)];
        match crate::proxyaddr::ProxyAddr::parse(&format!("{scheme}://{h}:{p}")) {
            Some(px) => out.push(crate::proxyaddr::system_row(&format!("Системный прокси {kind}"), &px, off)),
            None => out.push(Check::new("net", &format!("Системный прокси {kind}"), WARN, "включён, но без адреса").fixes(off)),
        }
    }
    if d.get("ProxyAutoConfigEnable").map(String::as_str) == Some("1") {
        any = true;
        out.push(crate::proxyaddr::pac_row(
            d.get("ProxyAutoConfigURLString").map(String::as_str).unwrap_or(""),
            vec![fx("mac_proxy_off", "Выключить системный прокси", None)],
        ));
    }
    if !any {
        out.push(Check::new("net", "Системный прокси", OK, "выключен (сеть напрямую)"));
    }
    out.extend(crate::proxyaddr::env_row());
    out
}

/// VPN-службы macOS: `scutil --nc list` — «* (Connected) UUID VPN (bundle) "Имя" [VPN/…]».
pub(crate) fn check_vpn() -> Vec<Check> {
    let (_, out_) = run(&["scutil", "--nc", "list"], 10);
    let mut out = Vec::new();
    for l in out_.lines().filter(|l| l.contains('"')) {
        let state = l.split('(').nth(1).and_then(|x| x.split(')').next()).unwrap_or("").to_string();
        let name = l.split('"').nth(1).unwrap_or("VPN").to_string();
        let (st, v) = match state.as_str() {
            "Connected" => (OK, "подключён"),
            "Connecting" | "Disconnecting" | "Reasserting" => (WARN, "переподключается"),
            _ => (INFO, "выключен"),
        };
        out.push(Check::new("net", &format!("VPN {name}"), st, v));
    }
    // VPN-программы, которых нет в системном списке (scutil): работают своим помощником или прокси
    // (AdGuard VPN и т. п.). Запущена — ещё не значит «подключена», поэтому только подсказка итогу
    let procs = crate::procs::list();
    let mut apps: Vec<String> = Vec::new();
    for (proc_name, title) in VPN_APPS {
        if procs.iter().any(|p| p.name == proc_name || p.name.ends_with(&format!("/{proc_name}")))
            && !out.iter().any(|c| c.title.contains(title) && c.status == OK)
        {
            apps.push(title.to_string());
            out.push(Check::new("net", &format!("VPN {title}"), INFO, "программа запущена"));
        }
    }
    // итог — общий для всех систем (страна выхода против своей страны, «интернет падает из-за VPN»)
    let up: Vec<String> = out.iter().filter(|c| c.status == OK).map(|c| c.title.trim_start_matches("VPN ").to_string()).collect();
    let installed = !out.is_empty();
    let mut sum = crate::diag::vpn_summary_apps(&up, installed, &apps, Vec::new());
    sum.append(&mut out);
    sum
}

/// Интернет и DNS (страница проверки подключения Apple).
pub(crate) fn check_net() -> Vec<Check> {
    crate::netproxy::check_net_basic("mac_dns_flush")
}

// ───────────────────────────── Система ─────────────────────────────

pub(crate) fn check_disk() -> Vec<Check> {
    let mut out = Vec::new();
    let (_, df) = run(&["df", "-k", "/"], 10);
    if let Some(l) = df.lines().nth(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        let (size, avail) =
            (f.get(1).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0), f.get(3).and_then(|x| x.parse::<f64>().ok()).unwrap_or(0.0));
        let gb = avail / 1048576.0;
        let pct = avail / size.max(1.0);
        let st = if gb < 5.0 || pct < 0.05 {
            BAD
        } else if gb < 15.0 {
            WARN
        } else {
            OK
        };
        let c = Check::new("sys", "Диск", st, &format!("свободно {gb:.0} ГБ"));
        out.push(if st == OK {
            c
        } else {
            c.hint("Мало места: обновления не встанут, программы тормозят. Часто место съедают локальные снимки Time Machine и кэши.")
        });
    }
    let (_, snaps) = run(&["tmutil", "listlocalsnapshots", "/"], 15);
    let n = snaps.lines().filter(|l| l.contains("com.apple")).count();
    if n > 0 {
        out.push(
            Check::new("sys", "Снимки Time Machine", INFO, &format!("{n} на диске"))
                .hint("Локальные снимки занимают место в «Системных данных». macOS удаляет их сама, когда место нужно; можно и вручную.")
                .fixes(vec![fx("mac_thin_snapshots", "Убрать снимки", None)]),
        );
    }
    out
}

/// Прожорливые программы: два замера `ps` с паузой.
pub(crate) fn check_hogs() -> Vec<Check> {
    let mut out = Vec::new();
    std::thread::sleep(Duration::from_millis(300));
    // владелец — по номеру (uid): имя пользователя ps обрезает, а $USER может быть не задан
    let (_, ps) = run(&["ps", "-Ao", "pcpu=,pid=,uid=,comm="], 10);
    let me = run(&["id", "-u"], 5).1.trim().to_string();
    for l in ps.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 {
            continue;
        }
        let pct: f64 = f[0].replace(',', ".").parse().unwrap_or(0.0);
        if pct < 90.0 {
            continue;
        }
        let name = f[3..].join(" ").rsplit('/').next().unwrap_or("").to_string();
        if matches!(name.as_str(), "mds_stores" | "mds" | "mdworker" | "mdworker_shared") {
            out.push(
                Check::new("sys", "Индексация Spotlight", INFO, &format!("{pct:.0}% процессора"))
                    .hint("Spotlight составляет поиск по файлам. Обычно затихает сам."),
            );
        } else if !me.is_empty() && f[2] == me {
            let all = crate::procs::list();
            let pid: i64 = f[1].parse().unwrap_or(0);
            let kind = all.iter().find(|p| p.pid == pid).map_or(crate::procs::Hog::Other, |p| crate::procs::hog_kind(p, &all));
            out.push(crate::more::hog_check(&name, pct, kind, fx("mac_force_quit", "Закрыть программу", Some(f[1]))));
        }
    }
    out
}

pub(crate) fn check_time() -> Vec<Check> {
    vec![crate::diag::clock_row(crate::diag::clock_offset(), None, vec![fx("mac_time_sync", "Сверить часы", None)])]
}

pub(crate) fn check_audio() -> Vec<Check> {
    let (code, _) = run(&["pgrep", "-x", "coreaudiod"], 5);
    if code == 0 {
        vec![Check::new("dev", "Звук (coreaudiod)", OK, "работает").fixes(vec![fx("mac_audio_restart", "Перезапустить звук", None)])]
    } else {
        vec![Check::new("dev", "Звук (coreaudiod)", BAD, "не запущен").fixes(vec![fx("mac_audio_restart", "Перезапустить звук", None)])]
    }
}

pub(crate) fn check_os() -> Vec<Check> {
    let (_, v) = run(&["sw_vers", "-productVersion"], 5);
    let (_, a) = run(&["uname", "-m"], 5);
    vec![Check::new("sys", "macOS", INFO, &format!("{} ({})", v.trim(), a.trim()))]
}

// ───────────────────────────── Лечение ─────────────────────────────

/// Список сетевых служб (без первой строки-пояснения и без «*» у выключенных).
/// VPN-программы без системного туннеля: (имя процесса, название). Их строка — в «Сети», а не в списке
/// открытых программ (раньше AdGuard VPN показывался дважды).
const VPN_APPS: [(&str, &str); 4] = [("AdGuard VPN", "AdGuard VPN"), ("Happ", "Happ"), ("Hiddify", "Hiddify"), ("v2rayN", "v2rayN")];

pub fn services() -> Vec<String> {
    let (_, o) = run(&["networksetup", "-listallnetworkservices"], 10);
    o.lines().skip(1).map(|l| l.trim_start_matches('*').trim().to_string()).filter(|l| !l.is_empty()).collect()
}

/// Скрипт выключения всех прокси на всех службах (для sh и для запроса пароля).
pub fn proxy_off_script() -> String {
    let mut s = Vec::new();
    for svc in services() {
        let q = svc.replace('\'', r"'\''");
        for opt in ["-setwebproxystate", "-setsecurewebproxystate", "-setsocksfirewallproxystate", "-setautoproxystate"] {
            s.push(format!("networksetup {opt} '{q}' off"));
        }
    }
    s.join("; ")
}

// ───────────────────────────── Зависшие и открытые программы ─────────────────────────────

/// Открытые программы с окнами: (имя, pid) из `lsappinfo list` (type="Foreground").
fn foreground_apps() -> Vec<(String, u32)> {
    let (_, o) = run(&["lsappinfo", "list"], 10);
    let mut v = Vec::new();
    let mut name = String::new();
    for l in o.lines() {
        let t = l.trim_start();
        // «12) "Safari" ASN:0x0-0x1234:» — начало блока программы
        if t.split(')').next().map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).unwrap_or(false) {
            name = t.split('"').nth(1).unwrap_or("").to_string();
        }
        if t.contains("type=\"Foreground\"") || t.contains("type=\"foreground\"") {
            if let Some(pid) = t.split("pid = ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|p| p.parse().ok()) {
                if !name.is_empty() {
                    v.push((name.clone(), pid));
                }
            }
        }
    }
    v
}

/// Отчёты macOS о зависаниях и падениях за неделю: (программа, зависание?).
fn reports() -> Vec<(String, bool)> {
    let home = crate::store::home_str();
    let week = std::time::SystemTime::now() - Duration::from_secs(7 * 86400);
    let mut out = Vec::new();
    for dir in [format!("{home}/Library/Logs/DiagnosticReports"), "/Library/Logs/DiagnosticReports".to_string()] {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            let fresh = e.metadata().and_then(|m| m.modified()).map(|t| t > week).unwrap_or(false);
            if !fresh {
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.ends_with(".hang") {
                out.push((name.split('_').next().unwrap_or("").to_string(), true));
            } else if name.ends_with(".ips") {
                // первая строка .ips — JSON-заголовок: app_name, bug_type (298/288 — зависание, 309 — падение)
                let head = std::fs::read_to_string(&p).ok().and_then(|t| t.lines().next().map(String::from)).unwrap_or_default();
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(&head) {
                    let bt = j["bug_type"].as_str().unwrap_or("");
                    let app = j["app_name"].as_str().or(j["name"].as_str()).unwrap_or("").to_string();
                    if !app.is_empty() && matches!(bt, "298" | "288" | "309" | "109") {
                        out.push((app, bt != "309" && bt != "109"));
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn check_apps() -> Vec<Check> {
    let mut out = Vec::new();
    // «застрявшие» процессы: T — остановлен, U — ждёт ядро и не реагирует даже на kill.
    // На macOS U бывает и мгновенным (обычное ожидание диска), поэтому верим, только если процесс
    // в том же состоянии на трёх снимках подряд за ~2 с — иначе ложная «беда»
    let snap = || run(&["ps", "-Ao", "pid=,stat=,user=,comm="], 10).1;
    let first = snap();
    let later: Vec<String> = (0..2)
        .map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(900));
            snap()
        })
        .collect();
    let state_of = |ps: &str, pid: &str| -> Option<char> {
        ps.lines().find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == pid).then(|| it.next()?.chars().next()).flatten()
        })
    };
    let me = std::env::var("USER").unwrap_or_default();
    let mut stuck: Vec<u32> = Vec::new();
    for l in first.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 || f[2] != me {
            continue;
        }
        let st0 = f[1].chars().next();
        if matches!(st0, Some('U') | Some('T')) && !later.iter().all(|ps| state_of(ps, f[0]) == st0) {
            continue; // на следующих снимках уже в порядке — мимолётное
        }
        let name = f[3..].join(" ").rsplit('/').next().unwrap_or("").to_string();
        if f[1].starts_with('U') || f[1].starts_with('T') {
            stuck.push(f[0].parse().unwrap_or(0));
        }
        if f[1].starts_with('U') {
            out.push(Check::new("sys", &format!("Завис намертво: {name}"), BAD, "не реагирует даже на завершение").hint(
                "Процесс ждёт диск или драйвер и не закрывается никак. Часто из-за отключённого диска или сетевой папки. Помогает перезагрузка.",
            ));
        } else if f[1].starts_with('T') {
            out.push(Check::new("sys", &format!("Остановлена: {name}"), WARN, "на паузе").fixes(vec![fx(
                "mac_force_quit",
                "Закрыть принудительно",
                Some(f[0]),
            )]));
        }
    }
    // история за неделю
    let mut by: Vec<(String, bool, usize)> = Vec::new();
    for (app, hang) in reports() {
        match by.iter_mut().find(|x| x.0 == app && x.1 == hang) {
            Some(x) => x.2 += 1,
            None => by.push((app, hang, 1)),
        }
    }
    by.sort_by(|a, b| b.2.cmp(&a.2));
    for (app, hang, n) in by.into_iter().take(3) {
        out.push(Check::new(
            "sys",
            &app,
            if n >= 3 { WARN } else { INFO },
            &format!("{} {} за неделю", if hang { "зависала" } else { "падала" }, crate::diag::plural(n as usize, "раз", "раза", "раз")),
        ));
    }
    // открытые программы — кнопка принудительного закрытия (то, что не всегда может «Завершить принудительно»)
    let me_pid = std::process::id();
    for (name, pid) in foreground_apps() {
        // уже показана выше как «на паузе» / «завис намертво»
        if pid == me_pid || name == "Аптечка" || name == "aptechka" || stuck.contains(&pid) || VPN_APPS.iter().any(|(_, t)| *t == name) {
            continue;
        }
        let fix = if name == "Finder" {
            fx("mac_finder_restart", "Перезапустить Finder", None)
        } else {
            fx("mac_force_quit", "Закрыть принудительно", Some(&pid.to_string()))
        };
        let mut c = Check::new("sys", &format!("Открыта: {name}"), INFO, "работает")
            .hint("Если программа зависла и не закрывается — нажми «Закрыть принудительно» (несохранённое пропадёт).")
            .fixes(vec![fix]);
        c.key = "sys";
        out.push(c);
    }
    out
}
