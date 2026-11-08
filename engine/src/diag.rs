#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
//! Диагностика «Аптечки»: порт проверок из aptechka/checks.py (сеть, VPN, прокси,
//! связь с Claude, видеокарта, система, часы).
//!
//! Все проверки только читают: ничего не чинят, не меняют настройки и не трогают процессы.
//! Кнопки починки (`Fix`) здесь только описаны данными, выполняет их другой код.
//!
//! Сбой одной проверки не роняет программу (panic=unwind + catch_unwind в планировщике), но и здесь по возможности без unwrap/expect/индексов без проверки.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Статусы строки. Ещё бывают «checking» и «off» — их ставят планировщик и экраны, не сами проверки.
pub const OK: &str = "ok";
pub const WARN: &str = "warn";
pub const BAD: &str = "bad";
pub const INFO: &str = "info";

/// Кнопка починки: (идентификатор, подпись, аргумент). Здесь только описание.
#[derive(Clone, Debug)]
pub struct Fix {
    pub id: String,
    pub label: String,
    pub arg: Option<String>,
}

/// Результат одной проверки: строка таблицы.
#[derive(Clone, Debug)]
pub struct Check {
    /// Вкладка: "net", "ai", "gpu", "dev", "theme", "sys".
    pub key: &'static str,
    pub title: String,
    /// ok | warn | bad | info | checking | off
    pub status: String,
    pub value: String,
    /// Подсказка.
    pub detail: String,
    /// Кнопки починки (первая — на строке таблицы).
    pub fixes: Vec<Fix>,
    /// Код показателя из каталога, который дал эту строку ("" — не из каталога, например замер скорости).
    pub unit: &'static str,
    /// Проверка не смогла выполниться (программа не ответила, нет доступа, просмотрено не всё): статус
    /// остаётся info, но итог не должен говорить «всё в порядке». Обычные «для сведения» (пояс, IP) — false.
    pub unchecked: bool,
}

/// «Игнорировать»: красные и жёлтые строки, которые велели не считать бедой (prefs::ignored), становятся
/// «для сведения» — итог, виджет и кот в картинке их не замечают. Сама проверка идёт как шла.
pub fn apply_ignored(checks: &[Check]) -> Vec<Check> {
    ignore_in(checks, &crate::prefs::ignored())
}

fn ignore_in(checks: &[Check], ig: &std::collections::HashSet<String>) -> Vec<Check> {
    checks
        .iter()
        .cloned()
        .map(|mut c| {
            if ig.contains(&c.title) && matches!(c.status.as_str(), "warn" | "bad") {
                c.status = "info".into();
            }
            c
        })
        .collect()
}

impl Check {
    pub(crate) fn new(key: &'static str, title: &str, status: &str, value: &str) -> Check {
        Check {
            key,
            // логин и пароль из адресов (прокси, репозитории) не попадают ни в одну строку — ни на экран,
            // ни в checks.json, ни в виджет
            title: crate::proxyaddr::redact(title),
            status: status.to_string(),
            value: crate::proxyaddr::redact(value),
            detail: String::new(),
            fixes: Vec::new(),
            unit: "",
            unchecked: false,
        }
    }
    /// Пометить строку «не удалось проверить».
    pub(crate) fn unknown(mut self) -> Check {
        self.unchecked = true;
        self
    }
    pub(crate) fn hint(mut self, h: &str) -> Check {
        self.detail = crate::proxyaddr::redact(h);
        self
    }
    pub(crate) fn fixes(mut self, f: Vec<Fix>) -> Check {
        self.fixes = f;
        self
    }
}

/// Число со словом в нужной форме: plural(4, "раз", "раза", "раз") → «4 раза».
pub(crate) fn plural(n: usize, one: &str, few: &str, many: &str) -> String {
    let w = match (n % 10, n % 100) {
        (_, 11..=14) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    };
    format!("{n} {w}")
}

pub(crate) fn fx(id: &str, label: &str, arg: Option<&str>) -> Fix {
    Fix { id: id.to_string(), label: label.to_string(), arg: arg.map(|s| s.to_string()) }
}

pub(crate) fn log(msg: &str) {
    eprintln!("[диагностика] {}", term_safe(&crate::proxyaddr::redact(msg)));
}

/// Текст для терминала без управляющих последовательностей (ESC, OSC, BEL, обратный порядок письма):
/// название сессии или имя файла не перерисуют терминал и не подделают соседние строки.
pub(crate) fn term_safe(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\t' || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}') { '�' } else { c })
        .collect()
}

pub(crate) fn home() -> String {
    crate::store::home_str()
}

/// Запуск без мигающего окна консоли на Windows (у GUI-программы каждая команда открыла бы своё окно).
pub(crate) fn quiet(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    // запускаемым программам — только нужное окружение (список разрешённых имён, см. cmd::env_allowed)
    crate::cmd::scrub_env(c);
    c
}

pub(crate) fn trunc(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ───────────────────────────── Помощники ─────────────────────────────

/// Запустить команду, вернуть (код, stdout). Никогда не паникует.
/// По таймауту убивает только собственный дочерний процесс команды и возвращает 127.
pub(crate) fn run(cmd: &[&str], timeout_s: u64) -> (i32, String) {
    // код и вывод; если программы нет или она не ответила — вывод пустой (а не текст ошибки запуска)
    let r = run_ex(cmd, timeout_s);
    (r.code(), r.text().to_string())
}

/// Запуск с полным исходом (ответила / ошибка / программы нет / не ответила) — когда «нет программы»
/// нужно отличить от «программа сказала нет» (crate::cmd).
pub(crate) fn run_ex(cmd: &[&str], timeout_s: u64) -> crate::cmd::Outcome {
    crate::cmd::exec(cmd, Duration::from_secs(timeout_s), crate::cmd::CHECK)
}

/// GET без падений: (http-код или None, тело, ошибка).
/// По умолчанию идёт напрямую, мимо прокси (ureq сам прокси из окружения не берёт):
/// так видно, работает ли сам туннель. proxy = "http://host:port" идёт через него.
pub(crate) fn http_get(url: &str, timeout_s: u64, limit: u64, proxy: Option<&str>) -> (Option<u16>, String, String) {
    http_get_ms(url, timeout_s, limit, proxy).0
}

/// Программа стояла (сон ноутбука, зависание системы): попытка длилась заметно дольше
/// своего лимита. Тогда её «нет ответа» — не про сеть, пробуем ещё раз.
fn stalled(t: Instant, timeout_s: u64) -> bool {
    t.elapsed() > Duration::from_secs(timeout_s + 3)
}

/// Как http_get, плюс время ответа в мс (последней попытки).
pub(crate) fn http_get_ms(url: &str, timeout_s: u64, limit: u64, proxy: Option<&str>) -> ((Option<u16>, String, String), f64) {
    let t = Instant::now();
    let r = http_get_once(url, timeout_s, limit, proxy);
    if r.0.is_some() || !stalled(t, timeout_s) {
        return (r, t.elapsed().as_secs_f64() * 1000.0);
    }
    let t = Instant::now();
    (http_get_once(url, timeout_s, limit, proxy), t.elapsed().as_secs_f64() * 1000.0)
}

fn http_get_once(url: &str, timeout_s: u64, limit: u64, proxy: Option<&str>) -> (Option<u16>, String, String) {
    let mut b = ureq::AgentBuilder::new().timeout(Duration::from_secs(timeout_s)).user_agent(concat!("aptechka/", env!("CARGO_PKG_VERSION")));
    if let Some(p) = proxy {
        match ureq::Proxy::new(p) {
            Ok(px) => b = b.proxy(px),
            Err(e) => return (None, String::new(), trunc(&format!("Proxy: {e}"), 80)),
        }
    }
    match b.build().get(url).call() {
        Ok(resp) => {
            let code = resp.status();
            let mut buf = Vec::new();
            let _ = resp.into_reader().take(limit).read_to_end(&mut buf);
            (Some(code), String::from_utf8_lossy(&buf).to_string(), String::new())
        }
        Err(ureq::Error::Status(code, _)) => (Some(code), String::new(), String::new()),
        Err(e) => (None, String::new(), short_err(&e.to_string())),
    }
}

/// GET без переадресаций: (код, начало тела, ошибка). Для адресов проверки подключения: переадресация —
/// признак страницы входа, а не «интернет есть».
pub(crate) fn http_get_noredirect(url: &str, timeout_s: u64, limit: u64) -> (Option<u16>, String, String) {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_s))
        .redirects(0)
        .user_agent(concat!("aptechka/", env!("CARGO_PKG_VERSION")))
        .build();
    match agent.get(url).call() {
        Ok(resp) => {
            let code = resp.status();
            let mut buf = Vec::new();
            let _ = resp.into_reader().take(limit).read_to_end(&mut buf);
            (Some(code), String::from_utf8_lossy(&buf).to_string(), String::new())
        }
        Err(ureq::Error::Status(code, _)) => (Some(code), String::new(), String::new()),
        Err(e) => (None, String::new(), short_err(&e.to_string())),
    }
}

/// Ошибка запроса коротко; ошибка сертификата помечается в начале (в длинном тексте она в самом конце
/// и при обрезке терялась — а по ней видно «неверные часы»).
fn short_err(e: &str) -> String {
    let low = e.to_lowercase();
    if low.contains("certificate") || low.contains("notvalidyet") || low.contains("expired") {
        let tail = e.rsplit(':').next().unwrap_or("").trim();
        return format!("certificate: {}", trunc(tail, 60));
    }
    trunc(e, 80)
}

/// Дата из заголовка Date (HEAD) для проверки часов. Как в Python: сбой или ошибка HTTP
/// с заголовком тоже годятся.
pub(crate) fn http_date(url: &str, timeout_s: u64) -> Option<String> {
    // без переадресаций: http://… часто переадресует на https://, а там при неверных часах TLS не сложится;
    // заголовок Date есть и у ответа-переадресации
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_s))
        .redirects(0)
        .user_agent(concat!("aptechka/", env!("CARGO_PKG_VERSION")))
        .build();
    match agent.head(url).call() {
        Ok(r) => r.header("Date").map(|s| s.to_string()),
        Err(ureq::Error::Status(_, r)) => r.header("Date").map(|s| s.to_string()),
        Err(_) => None,
    }
}

fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    (host, port).to_socket_addrs().ok()?.next()
}

/// Можно ли открыть TCP-соединение: (да/нет, миллисекунды).
fn tcp_ok(host: &str, port: u16, timeout_s: u64) -> (bool, Option<f64>) {
    let t = Instant::now();
    let r = tcp_ok_once(host, port, timeout_s);
    if r.0 || !stalled(t, timeout_s) {
        return r;
    }
    tcp_ok_once(host, port, timeout_s)
}

fn tcp_ok_once(host: &str, port: u16, timeout_s: u64) -> (bool, Option<f64>) {
    let t = Instant::now();
    let Some(addr) = resolve(host, port) else { return (false, None) };
    match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(timeout_s)) {
        Ok(_) => (true, Some(t.elapsed().as_secs_f64() * 1000.0)),
        Err(_) => (false, None),
    }
}

/// Слово после ключа: "dev eth0" -> "eth0".
fn token_after<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut it = text.split_whitespace();
    while let Some(t) = it.next() {
        if t == key {
            return it.next();
        }
    }
    None
}

fn tokens_after<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    let mut v = Vec::new();
    let mut it = text.split_whitespace();
    while let Some(t) = it.next() {
        if t == key {
            if let Some(n) = it.next() {
                v.push(n);
            }
        }
    }
    v
}

fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

// ───────────────────────────── Сеть ─────────────────────────────

pub(crate) fn check_network() -> Vec<Check> {
    let mut out = Vec::new();
    let reconnect = || vec![fx("net_reconnect", "Переподключить сеть", None)];
    // маршрут в интернет по IPv4, а нет — по IPv6 (сети только с IPv6: мобильные, часть провайдеров)
    let route_r = run_ex(&["ip", "route", "get", "1.1.1.1"], 6);
    let route6_r = run_ex(&["ip", "-6", "route", "get", "2606:4700:4700::1111"], 6);
    let (route_r, fam) = if route_r.code() == 0 && token_after(route_r.text(), "dev").is_some() || route6_r.code() != 0 {
        (route_r, "")
    } else {
        (route6_r, " (только IPv6)")
    };
    let (code, route) = (route_r.code(), route_r.text().to_string());
    let dev = token_after(&route, "dev");
    let src = token_after(&route, "src");
    match dev {
        Some(dev) if code == 0 => {
            out.push(Check::new("net", "Подключение", OK, &format!("через {dev}{fam}")).fixes(reconnect()));
            out.push(Check::new("net", "Мой локальный IP", INFO, src.unwrap_or("?")));
        }
        // программы ip нет или она не ответила — о маршруте ничего не знаем (а не «нет маршрута»)
        _ if !route_r.answered() => {
            out.push(Check::new("net", "Подключение", INFO, "не проверить").unknown().hint("Нет программы ip (iproute2) или она не ответила."));
        }
        _ => {
            out.push(Check::new("net", "Подключение", BAD, "нет маршрута").hint("Кабель/Wi-Fi не подключены или сеть выключена.").fixes(reconnect()))
        }
    }

    // Интернет и DNS — общая проверка для всех систем
    out.extend(crate::reach::rows("dns_flush", reconnect()));
    out
}

struct Iface {
    name: String,
    up: bool,
    addrs: Vec<String>,
    kind: LinkKind,
}

/// Что за устройство — по ядру (/sys/class/net), а не по имени: имя туннелю даёт кто угодно
/// (tailscale0, CloudflareWARP, outline-tun0), а tap-… у виртуальных машин — тоже «tun», но не VPN.
#[derive(Clone, Copy, PartialEq, Debug)]
enum LinkKind {
    /// туннель уровня IP: tun, WireGuard, IPsec (xfrm/vti) — так устроены VPN
    Tunnel,
    /// PPP без PPPoE — PPTP/L2TP VPN (PPPoE — это DSL-интернет провайдера, не VPN)
    PppVpn,
    /// всё прочее: проводная сеть, Wi-Fi, мосты и tap виртуальных машин, PPPoE
    Other,
}

fn link_kind(name: &str) -> LinkKind {
    let base = std::path::Path::new("/sys/class/net").join(name);
    let read = |f: &str| std::fs::read_to_string(base.join(f)).unwrap_or_default();
    let devtype = read("uevent").lines().find_map(|l| l.strip_prefix("DEVTYPE=").map(str::to_string)).unwrap_or_default();
    // ARPHRD: 65534 — без канального уровня (tun, wireguard), 512 — ppp, 768/776/778 — ipip/sit/gre (vti)
    match read("type").trim() {
        "65534" => LinkKind::Tunnel,
        "512" if pppoe_sessions() == 0 => LinkKind::PppVpn,
        "768" | "769" if devtype == "vti" || devtype == "vti6" => LinkKind::Tunnel,
        _ if devtype == "wireguard" || devtype == "xfrm" => LinkKind::Tunnel,
        _ => LinkKind::Other,
    }
}

/// Сколько сейчас PPPoE-сессий (DSL-подключений провайдера).
fn pppoe_sessions() -> usize {
    std::fs::read_to_string("/proc/net/pppoe").map_or(0, |t| t.lines().skip(1).filter(|l| !l.trim().is_empty()).count())
}

/// Понятное имя туннеля без известного клиента.
fn tunnel_label(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    let known = [
        ("tailscale", "Tailscale"),
        ("cloudflarewarp", "Cloudflare WARP"),
        ("zt", "ZeroTier"),
        ("nordlynx", "NordVPN"),
        ("mullvad", "Mullvad"),
        ("wg", "WireGuard"),
        ("ppp", "PPTP/L2TP"),
        ("ipsec", "IPsec"),
        ("vti", "IPsec"),
        ("xfrm", "IPsec"),
    ];
    match known.iter().find(|(p, _)| n.starts_with(p)) {
        Some((_, label)) => format!("{label} ({name})"),
        None => format!("туннель {name}"),
    }
}

/// Все сетевые интерфейсы: имя, поднят ли, адреса.
fn interfaces() -> Vec<Iface> {
    let (_, js) = run(&["ip", "-j", "addr"], 6);
    let mut res = Vec::new();
    let v: serde_json::Value = serde_json::from_str(if js.is_empty() { "[]" } else { &js }).unwrap_or(serde_json::Value::Null);
    if let Some(arr) = v.as_array() {
        for i in arr {
            let Some(name) = i.get("ifname").and_then(|x| x.as_str()) else { continue };
            let flags: Vec<&str> =
                i.get("flags").and_then(|f| f.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
            let up = flags.contains(&"LOWER_UP") && !flags.contains(&"NO-CARRIER");
            let addrs = i
                .get("addr_info")
                .and_then(|a| a.as_array())
                .map(|a| a.iter().map(|x| x.get("local").and_then(|l| l.as_str()).unwrap_or("").to_string()).collect())
                .unwrap_or_default();
            res.push(Iface { name: name.to_string(), up, addrs, kind: link_kind(name) });
        }
    }
    res
}

/// active / failed / inactive / activating… / missing (не установлена) / unknown (systemctl не ответил или
/// ответил непонятно). Один запуск systemctl на службу (раньше — два, и ещё раз позже).
fn service_state(unit: &str) -> String {
    let r = run_ex(&["systemctl", "show", "-p", "LoadState,ActiveState", unit], 6);
    if !r.answered() {
        return "unknown".into();
    }
    let get = |k: &str| r.text().lines().find_map(|l| l.strip_prefix(k)?.strip_prefix('=')).unwrap_or("").trim().to_string();
    let (load, active) = (get("LoadState"), get("ActiveState"));
    const KNOWN: [&str; 6] = ["active", "inactive", "failed", "activating", "deactivating", "reloading"];
    if load == "not-found" {
        "missing".into()
    } else if load == "loaded" && KNOWN.contains(&active.as_str()) {
        active
    } else {
        "unknown".into()
    }
}

/// Адрес TUN-интерфейса из конфига Happ (sing-box), чтобы найти его туннель.
fn happ_tun_addr() -> Option<String> {
    let txt = crate::store::read_text(format!("{}/.config/Happ/config.json", home())).ok()?;
    let cfg: serde_json::Value = serde_json::from_str(&txt).ok()?;
    for ib in cfg.get("inbounds")?.as_array()? {
        if ib.get("type").and_then(|t| t.as_str()) == Some("tun") {
            let empty = |v: &serde_json::Value| v.is_null() || v.as_str().is_some_and(|s| s.is_empty()) || v.as_array().is_some_and(|a| a.is_empty());
            let mut a = ib.get("address").cloned().unwrap_or(serde_json::Value::Null);
            if empty(&a) {
                a = ib.get("inet4_address").cloned().unwrap_or(serde_json::Value::Null);
            }
            let first = match &a {
                serde_json::Value::Array(v) => v.first().cloned().unwrap_or(serde_json::Value::Null),
                other => other.clone(),
            };
            let s = first.as_str()?.to_string();
            let h = s.split('/').next().unwrap_or("").to_string();
            return if h.is_empty() { None } else { Some(h) };
        }
    }
    None
}

/// VPN-канал: служба, как узнать туннель, локальный порт прокси.
struct Channel {
    name: &'static str,
    service: &'static str,
    iface_prefixes: &'static [&'static str],
    procs: &'static [&'static str],
    proxy_port: Option<u16>,
    fix: &'static str,
    svc_fix: &'static str,
}

const VPN_CHANNELS: &[Channel] = &[
    Channel {
        name: "Happ",
        service: "happd.service",
        iface_prefixes: &["happ", "sing", "xray"],
        procs: &["Happ"],
        proxy_port: Some(10809),
        fix: "happ_restart",
        svc_fix: "svc_restart",
    },
    Channel {
        name: "Outline",
        service: "outline_proxy_controller.service",
        iface_prefixes: &["outline"],
        procs: &["Outline", "outline-client"],
        proxy_port: None,
        fix: "outline_restart",
        svc_fix: "svc_restart",
    },
    Channel {
        name: "Proton VPN",
        service: "me.proton.vpn.split_tunneling.service",
        iface_prefixes: &["proton", "pvpn"],
        procs: &["protonvpn-app"],
        proxy_port: None,
        fix: "proton_restart",
        svc_fix: "svc_restart",
    },
];

pub(crate) fn check_vpn() -> Vec<Check> {
    let mut out: Vec<Check> = Vec::new();
    let ifs = interfaces();
    // процессы — по имени программы (comm) и имени файла argv[0], а не по любому слову в аргументах:
    // «grep Happ» или «vim Happ.md» клиентом не считаются
    let procs = crate::procs::list();
    let running = |want: &str| {
        procs.iter().any(|p| p.name == want || p.args.first().and_then(|a| std::path::Path::new(a).file_name()).is_some_and(|f| f == want))
    };
    let happ_addr = happ_tun_addr();
    // соединения NetworkManager: (имя, устройство) — Proton VPN поднимает WireGuard-соединение «ProtonVPN …»
    let nm: Vec<(String, String)> = run(&["nmcli", "-t", "-f", "NAME,DEVICE", "connection", "show", "--active"], 6)
        .1
        .lines()
        .filter_map(|l| l.rsplit_once(':').map(|(n, d)| (n.replace("\\:", ":"), d.to_string())))
        .collect();

    // службы всех клиентов — параллельно (раньше по очереди: до 6 с на каждый вызов)
    let states: Vec<(&str, String)> = std::thread::scope(|sc| {
        let hs: Vec<_> = VPN_CHANNELS.iter().map(|c| (c.service, sc.spawn(move || service_state(c.service)))).collect();
        hs.into_iter().map(|(n, h)| (n, h.join().unwrap_or_else(|_| "unknown".into()))).collect()
    });
    let mut up_channels: Vec<String> = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    let mut proxy_up: Vec<&str> = Vec::new();
    for ch in VPN_CHANNELS {
        let mut tun: Vec<&str> = ifs.iter().filter(|i| ch.iface_prefixes.iter().any(|p| i.name.starts_with(p))).map(|i| i.name.as_str()).collect();
        // общие имена (tun0, wg0) приписываем клиенту только по надёжному признаку, а не по имени:
        // Happ — адрес туннеля из его настроек; Proton — соединение NetworkManager с его именем
        let owned: Vec<&str> = ifs
            .iter()
            .filter(|i| match ch.name {
                "Happ" => happ_addr.as_ref().is_some_and(|a| i.addrs.contains(a)),
                "Proton VPN" => nm.iter().any(|(n, d)| *d == i.name && n.to_lowercase().contains("proton")),
                _ => false,
            })
            .map(|i| i.name.as_str())
            .collect();
        for n in owned {
            if !tun.contains(&n) {
                tun.push(n);
            }
        }
        let svc = states.iter().find(|(n, _)| *n == ch.service).map(|(_, s)| s.clone()).unwrap_or_default();
        let gui = ch.procs.iter().any(|p| running(p));
        let alive = gui || svc == "active";
        let tun_up: Vec<&str> = tun.iter().copied().filter(|n| ifs.iter().any(|i| i.name == *n && i.up)).collect();
        claimed.extend(tun_up.iter().map(|n| n.to_string()));
        // порт 10809 и подобные — у многих клиентов (v2rayN, Nekoray, Hiddify): засчитываем его этому каналу,
        // только когда сам клиент работает
        let port_up = alive && ch.proxy_port.is_some_and(|p| tcp_ok("127.0.0.1", p, 1).0);
        if port_up {
            proxy_up.push(ch.name);
        }

        let fixes = || vec![fx(ch.fix, &format!("Перезапустить {}", ch.name), None)];
        let mut parts: Vec<String> = Vec::new();
        if !tun_up.is_empty() {
            parts.push(format!("туннель {}", tun_up.join(",")));
        }
        if port_up {
            parts.push(format!("прокси :{}", ch.proxy_port.unwrap_or(0)));
        }
        let title = format!("Канал {}", ch.name);
        if !parts.is_empty() {
            // туннель или прокси поднят: канал работает, что бы ни писала служба
            up_channels.push(ch.name.to_string());
            out.push(
                Check::new("net", &title, OK, &format!("включён: {}", parts.join(", ")))
                    .hint(if gui { "приложение открыто" } else { "работает в фоне" })
                    .fixes(fixes()),
            );
        } else if svc == "missing" && !gui {
            out.push(Check::new("net", &title, INFO, "не установлен"));
        } else if svc == "unknown" && !gui {
            continue; // systemctl не ответил — про этот клиент ничего не знаем, строку не показываем
        } else if svc == "failed" || svc == "inactive" {
            let mut f = vec![fx(ch.svc_fix, "Перезапустить службу", Some(ch.service))];
            f.extend(fixes());
            out.push(
                Check::new("net", &title, BAD, &format!("служба {svc}"))
                    .hint(&format!("Служба {} не работает, клиент не сможет подключиться.", ch.service))
                    .fixes(f),
            );
        } else {
            let down = if tun.is_empty() { String::new() } else { format!(" (интерфейс {} лежит)", tun.join(",")) };
            let hint = format!("служба готова{}", if gui { ", приложение открыто" } else { "" });
            out.push(Check::new("net", &title, INFO, &format!("выключен{down}")).hint(&hint).fixes(fixes()));
        }
    }

    // поднятые туннели, которые не приписаны ни одному известному клиенту (WireGuard, openvpn, свой VPN):
    // VPN на этом компьютере есть — назовём его по имени туннеля
    for i in &ifs {
        // по типу устройства: tap виртуальных машин и PPPoE провайдера сюда не попадают
        let vpn_like = matches!(i.kind, LinkKind::Tunnel | LinkKind::PppVpn);
        if vpn_like && i.up && !claimed.contains(&i.name) {
            let label = tunnel_label(&i.name);
            up_channels.push(label.clone());
            out.push(
                Check::new("net", &format!("Туннель {}", i.name), OK, "поднят")
                    .hint(&format!("{label}: VPN-туннель без известного Аптечке клиента.")),
            );
        }
    }
    // конфликт — когда маршрут в интернет держат сразу два туннеля (default или пара 0.0.0.0/1 + 128.0.0.0/1),
    // а не когда просто существуют два (корпоративный wg0 для рабочей сети рядом с VPN — норма)
    let routes = run(&["ip", "route", "show"], 6).1;
    let routing: Vec<&str> = routes
        .lines()
        .filter(|l| l.starts_with("default") || l.starts_with("0.0.0.0/1") || l.starts_with("128.0.0.0/1"))
        .filter_map(|l| token_after(l, "dev"))
        .collect();
    let mut routing_tunnels: Vec<&str> = ifs
        .iter()
        .filter(|i| matches!(i.kind, LinkKind::Tunnel | LinkKind::PppVpn) && routing.contains(&i.name.as_str()))
        .map(|i| i.name.as_str())
        .collect();
    routing_tunnels.dedup();
    if up_channels.len() > 1 && routing_tunnels.len() > 1 {
        out.push(Check::new("net", "Конфликт VPN", WARN, &up_channels.join(" + ")).hint(
            "Включено сразу несколько VPN. Они могут драться за маршруты, \
                   и сеть начинает пропадать. Оставь один.",
        ));
    }

    // Маршрут по умолчанию ведёт в мёртвый туннель: классическая причина «нет сети»
    let (_, r) = run(&["ip", "route", "show", "default"], 6);
    for dev in tokens_after(&r, "dev") {
        if ifs.iter().any(|i| i.name == dev && !i.up) {
            out.push(
                Check::new("net", "Маршрут", BAD, &format!("ведёт в лежащий {dev}"))
                    .hint("Весь трафик идёт в выключенный туннель. Перезапусти VPN или переподключи сеть.")
                    .fixes(vec![fx("net_reconnect", "Переподключить сеть", None)]),
            );
        }
    }

    let fixes: Vec<Fix> = VPN_CHANNELS.iter().map(|c| fx(c.fix, &format!("Запустить {}", c.name), None)).collect();
    let installed = !up_channels.is_empty() || states.iter().any(|(_, s)| s != "missing" && s != "unknown");
    let up: Vec<String> = up_channels.clone();
    // включено только прокси-портом, туннелей нет
    let proxy_mode = !up.is_empty() && !proxy_up.is_empty() && claimed.is_empty() && routing_tunnels.is_empty();
    let mut sum = vpn_summary_full(&up, installed, &[], fixes, proxy_mode);
    sum.append(&mut out);
    sum
}

// Итог по VPN — один для Linux, Windows и macOS.
//
// up — VPN, поднятые на ЭТОМ компьютере (туннель или прокси, видно без интернета); installed — стоит ли
// хоть один VPN-клиент. Как решаем:
// 1) интернета нет совсем (ни Яндекс, ни Cloudflare по TCP) — «VPN не проверить», причину покажут строки сети;
// 2) туннель поднят, Яндекс открывается, а заграница — нет: интернет падает ИЗ-ЗА VPN (сайты для этого не нужны);
// 3) страна выхода в интернет (несколько сервисов) — сравниваем со СВОЕЙ страной компьютера (часовой пояс
//    или регион системы, а не «не Россия»): отличается — трафик уходит через VPN, хоть на роутере;
//    совпадает — VPN на выходе не видно.

/// То же, apps — VPN-программы, которые запущены, но туннеля их на компьютере не видно (AdGuard VPN в режиме
/// прокси, Windows-клиент без туннеля): выход за границу тогда скорее через них, чем «на роутере».
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn vpn_summary_apps(up: &[String], installed: bool, apps: &[String], fixes: Vec<Fix>) -> Vec<Check> {
    vpn_summary_full(up, installed, apps, fixes, false)
}

/// proxy_mode — VPN включён только как прокси на порту (без туннеля): он работает для программ, которые
/// ходят через порт, а прямой путь (по нему меряется страна выхода) идёт мимо него — так и задумано.
pub(crate) fn vpn_summary_full(up: &[String], installed: bool, apps: &[String], fixes: Vec<Fix>, proxy_mode: bool) -> Vec<Check> {
    let mut out = Vec::new();
    // заграница: Cloudflare по TCP или страница проверки Google по имени (сеть только с IPv6, закрытый 1.1.1.1)
    let world_ok = tcp_ok("1.1.1.1", 443, 3).0 || http_get_noredirect("http://connectivitycheck.gstatic.com/generate_204", 4, 100).0 == Some(204);
    let home_ok = world_ok || tcp_ok("77.88.8.8", 443, 3).0 || crate::reach::internet().is_some();
    let home = home_country();
    let t = "VPN (итог)";
    if !home_ok && !world_ok {
        out.push(
            Check::new("net", t, INFO, "не проверить: нет интернета")
                .unknown()
                .hint("Сначала нужен интернет — смотри строки «Подключение», «Интернет», «DNS»."),
        );
        return out;
    }
    if !up.is_empty() && !world_ok {
        out.push(Check::new("net", t, BAD, &format!("{} включён, но через него не пускает", up.join(", ")))
            .hint("Местные сайты открываются, а зарубежные — нет: трафик уходит в туннель и там теряется. Переподключи VPN или выбери другой сервер.")
            .fixes(fixes));
        return out;
    }
    // отпечаток локального состояния: поменялся (VPN включили/выключили) — страну спросим заново, не из кэша
    let route = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let dflt =
        route.lines().skip(1).find(|l| l.split_whitespace().nth(1) == Some("00000000")).and_then(|l| l.split_whitespace().next()).unwrap_or("");
    // в отпечаток — и куда сейчас уходит трафик (смена выхода в Tailscale, правила ip rule меняют его)
    let via = run(&["ip", "route", "get", "1.1.1.1"], 4).1;
    let sig = format!("{}|{dflt}|{}", up.join(","), token_after(&via, "dev").unwrap_or(""));
    let (ip, exit, org) = where_am_i(&sig);
    if let Some(ip) = &ip {
        let c = exit.as_ref().map(|c| format!("· {c}")).unwrap_or_default();
        out.push(Check::new("net", "Мой внешний IP", INFO, &format!("{ip} {c}")).hint(&trunc(&org, 60)));
    }
    let home_s = home.clone().unwrap_or_else(|| "?".into());
    let c = match (exit.as_deref(), home.as_deref()) {
        // туннель на компьютере, выход за границей (или своя страна неизвестна)
        (Some(e), h) if !up.is_empty() && Some(e) != h => Check::new("net", t, OK, &format!("включён: {} · выход {e}", up.join(", "))),
        // выход в своей стране: решает маршрут на самом компьютере, а не страна. Трафик идёт в туннель —
        // значит, сервер VPN в этой же стране (рабочий VPN, свой сервер, Tailscale) — это не поломка
        (Some(e), Some(_)) if !up.is_empty() && tunnel_routed() == Some(true) => Check::new("net", t, INFO, &format!("включён: {} · выход {e}", up.join(", ")))
            .hint("Трафик идёт в туннель, а выход — в твоей же стране: сервер VPN там же (рабочий VPN, свой сервер). Если нужен выход за границу — выбери другой сервер."),
        (Some(e), Some(_)) if !up.is_empty() && proxy_mode => Check::new("net", t, INFO, &format!("включён как прокси: {} · напрямую выход {e}", up.join(", ")))
            .hint("VPN работает в режиме прокси: через него ходят программы, настроенные на его порт (браузер с расширением, системный прокси). Остальные — напрямую, это нормально для такого режима."),
        (Some(e), Some(_)) if !up.is_empty() && tunnel_routed() == Some(false) => Check::new("net", t, WARN, &format!("включён, но трафик мимо него (выход {e})"))
            .hint("Туннель поднят, но маршрут в интернет идёт не через него. Проверь режим (TUN/прокси) и правила маршрутизации в клиенте.")
            .fixes(fixes),
        (Some(e), Some(_)) if !up.is_empty() => Check::new("net", t, INFO, &format!("включён: {} · выход {e}", up.join(", ")))
            .hint("Выход — в твоей же стране. Это нормально, если сервер VPN там же; если ждала другую страну — проверь сервер и режим в клиенте."),
        (None, _) if !up.is_empty() => Check::new("net", t, OK, &format!("включён: {}", up.join(", ")))
            .hint("Страну выхода проверить не удалось (сервисы не ответили), но туннель поднят."),
        // на компьютере VPN нет, а выход — в другой стране: VPN на роутере или в сети
        (Some(e), Some(h)) if e != h && !apps.is_empty() => Check::new("net", t, OK, &format!("выход {e} (похоже, {})", apps.join(", ")))
            .hint(&format!("Интернет видит тебя из {e} (компьютер настроен на {h}). Туннеля на компьютере не видно, но запущен {} — скорее всего, трафик идёт через него (или через VPN на роутере).", apps.join(", "))),
        (Some(e), Some(h)) if e != h => Check::new("net", t, OK, &format!("выход {e} (VPN вне компьютера)"))
            .hint(&format!("На компьютере VPN не включён, но интернет видит тебя из {e}, а компьютер настроен на {h}: VPN работает на роутере или в сети.")),
        (Some(e), None) => Check::new("net", t, INFO, &format!("выход {e}"))
            .hint("Не знаю, в какой стране компьютер (часовой пояс и регион системы не подсказали), поэтому не сужу, включён ли VPN."),
        (Some(e), Some(_)) if installed => Check::new("net", t, INFO, "выключен")
            .hint(&format!("VPN-клиент есть, но не включён: интернет видит тебя из {e}, как и без VPN. Открой клиент и подключись."))
            .fixes(fixes),
        (Some(_), Some(_)) => Check::new("net", t, INFO, "не используется").hint(&format!("VPN не установлен; выход в интернет — из {home_s}.")),
        (None, _) => Check::new("net", t, INFO, "не удалось проверить").unknown()
            .hint("Сервисы, по которым видно страну выхода в интернет, не ответили. Проверю в следующий раз."),
    };
    out.insert(0, c);
    out
}

/// Уходит ли трафик в интернет через туннель (Linux: устройство маршрута до 1.1.1.1 — туннель по типу).
/// None — не узнать (другая система, нет программы ip).
fn tunnel_routed() -> Option<bool> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let r = run_ex(&["ip", "route", "get", "1.1.1.1"], 4);
    let dev = token_after(r.text(), "dev")?.to_string();
    Some(matches!(link_kind(&dev), LinkKind::Tunnel | LinkKind::PppVpn))
}

/// Своя страна компьютера (двухбуквенный код): настройка home_country в ui.json, иначе часовой пояс
/// (/etc/localtime → zone.tab: Europe/Moscow → RU, Europe/Minsk → BY), на Windows — регион системы
/// (HKCU\Control Panel\International\Geo, Name), по языку системы не гадаем (en_US — не значит США).
pub(crate) fn home_country() -> Option<String> {
    let ok = |c: &str| (c.len() == 2 && c.chars().all(|x| x.is_ascii_uppercase())).then(|| c.to_string());
    if let Some(c) = crate::prefs::load().get("home_country").and_then(|v| v.as_str()).and_then(ok) {
        return Some(c);
    }
    #[cfg(windows)]
    {
        let (_, r) = run(&["reg", "query", "HKCU\\Control Panel\\International\\Geo", "/v", "Name"], 6);
        if let Some(c) = r.lines().find(|l| l.contains("Name")).and_then(|l| l.split_whitespace().last()).and_then(ok) {
            return Some(c);
        }
    }
    #[cfg(not(windows))]
    if let Ok(link) = std::fs::read_link("/etc/localtime") {
        let l = link.to_string_lossy().to_string();
        if let Some(zone) = l.split("zoneinfo/").nth(1) {
            for tab in ["/usr/share/zoneinfo/zone.tab", "/usr/share/zoneinfo/zone1970.tab"] {
                if let Ok(t) = std::fs::read_to_string(tab) {
                    if let Some(c) = t.lines().filter(|x| !x.starts_with('#')).find_map(|x| {
                        let f: Vec<&str> = x.split('\t').collect();
                        (f.get(2) == Some(&zone)).then(|| f[0].split(',').next().unwrap_or("").to_string())
                    }) {
                        if let Some(c) = ok(&c) {
                            return Some(c);
                        }
                    }
                }
            }
        }
    }
    // язык системы (LANG=en_US) — не страна: по нему не гадаем, «своя страна неизвестна» — честнее
    None
}

/// Внешний IP, страна и провайдер. Несколько сервисов по очереди (у ipinfo.io бесплатный лимит запросов
/// с одного адреса: через общий VPN-выход его выбирают чужие запросы, и он отвечает 429), удачный ответ
/// запоминается на 10 минут (Аптечка и виджет проверяют часто — не дёргать сервисы зря).
fn where_am_i(sig: &str) -> (Option<String>, Option<String>, String) {
    use std::sync::Mutex;
    static LAST: Mutex<Option<(std::time::Instant, String, Option<String>, Option<String>, String)>> = Mutex::new(None);
    if let Some((t, s, ip, c, org)) = LAST.lock().ok().and_then(|g| g.clone()) {
        if t.elapsed() < std::time::Duration::from_secs(600) && c.is_some() && s == sig {
            return (ip, c, org);
        }
    }
    let (mut ip, mut country, mut org) = (None, None, String::new());
    // 1) Cloudflare: «ip=…» и «loc=XX» строками
    let (code, body, _) = http_get("https://www.cloudflare.com/cdn-cgi/trace", 6, 20_000, None);
    if code == Some(200) {
        let get = |k: &str| body.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        ip = get("ip=");
        country = get("loc=").filter(|c| c != "XX" && c != "T1");
    }
    // 2) ipinfo.io — ещё и провайдер (для подсказки); 3) country.is
    let s = |j: &serde_json::Value, k: &str| j.get(k).and_then(|v| v.as_str()).map(|x| x.to_string()).filter(|x| !x.is_empty());
    let (code, body, _) = http_get("https://ipinfo.io/json", 6, 200_000, None);
    if code == Some(200) {
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(&body) {
            ip = ip.or_else(|| s(&j, "ip"));
            country = country.or_else(|| s(&j, "country"));
            org = s(&j, "org").unwrap_or_default();
        }
    }
    if country.is_none() {
        let (code, body, _) = http_get("https://api.country.is", 6, 20_000, None);
        if let Some(j) = (code == Some(200)).then(|| serde_json::from_str::<serde_json::Value>(&body).ok()).flatten() {
            ip = ip.or_else(|| s(&j, "ip"));
            country = s(&j, "country");
        }
    }
    if ip.is_none() {
        let (code, body, _) = http_get("https://api.ipify.org", 6, 200_000, None);
        ip = if code == Some(200) && !body.trim().is_empty() { Some(body.trim().to_string()) } else { None };
    }
    if let Ok(mut g) = LAST.lock() {
        *g = Some((std::time::Instant::now(), sig.to_string(), ip.clone(), country.clone(), org.clone()));
    }
    (ip, country, org)
}

/// Прокси, настроенный в системе для программ: переменные окружения (https_proxy, all_proxy, http_proxy),
/// Linux — настройки GNOME (ручной режим), Windows — «Параметры Интернета» (реестр), macOS — scutil --proxy.
pub(crate) fn configured_proxy() -> Option<crate::proxyaddr::ProxyAddr> {
    use crate::proxyaddr::ProxyAddr;
    for k in ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY", "http_proxy", "HTTP_PROXY"] {
        if let Some(p) = std::env::var(k).ok().and_then(|v| ProxyAddr::parse(&v)) {
            return Some(p);
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(p) = gnome_proxies().into_iter().next() {
        return Some(p);
    }
    #[cfg(windows)]
    {
        let (_, en) = run(&["reg", "query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings", "/v", "ProxyEnable"], 6);
        if en.lines().any(|l| l.contains("ProxyEnable") && l.trim_end().ends_with("0x1")) {
            let (_, sv) = run(&["reg", "query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings", "/v", "ProxyServer"], 6);
            if let Some(p) = sv.lines().find(|l| l.contains("ProxyServer")).and_then(|l| l.split_whitespace().last()).and_then(ProxyAddr::parse) {
                return Some(p);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let (_, t) = run(&["scutil", "--proxy"], 6);
        let get = |k: &str| t.lines().find_map(|l| l.trim().strip_prefix(k).and_then(|r| r.trim().strip_prefix(':')).map(|x| x.trim().to_string()));
        for (en, host, port, scheme) in [
            ("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
            ("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
            ("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5"),
        ] {
            if get(en).as_deref() == Some("1") {
                if let Some(p) = get(host).zip(get(port)).and_then(|(h, p)| ProxyAddr::parse(&format!("{scheme}://{h}:{p}"))) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Ручной прокси GNOME: адреса по порядку https, http, socks (только заполненные).
fn gnome_proxies() -> Vec<crate::proxyaddr::ProxyAddr> {
    if gsettings(&["org.gnome.system.proxy", "mode"]).trim_matches('\'') != "manual" {
        return Vec::new();
    }
    [("https", "http"), ("http", "http"), ("socks", "socks5")]
        .iter()
        .filter_map(|(kind, scheme)| {
            let schema = format!("org.gnome.system.proxy.{kind}");
            let h = gsettings(&[&schema, "host"]);
            let p = gsettings(&[&schema, "port"]);
            let h = h.trim_matches('\'');
            (!h.is_empty()).then(|| crate::proxyaddr::ProxyAddr::parse(&format!("{scheme}://{h}:{}", p.trim()))).flatten()
        })
        .collect()
}

/// Напрямую интернета нет — а через прокси системы есть? Тогда его адрес (без логина и пароля).
pub(crate) fn proxy_internet() -> Option<String> {
    let px = configured_proxy()?;
    (crate::proxyaddr::through(&px) == crate::proxyaddr::Through::Works).then(|| px.shown())
}

fn gsettings(args: &[&str]) -> String {
    let mut c = vec!["gsettings", "get"];
    c.extend_from_slice(args);
    run(&c, 6).1
}

/// Системный прокси, который остался после выключенного VPN, кладёт всю сеть.
/// Прокси в файлах (/etc/environment, ~/.bashrc, apt) — в netproxy::check_other_proxies.
pub(crate) fn check_proxy() -> Vec<Check> {
    let mut out = Vec::new();
    let off_fix = || vec![fx("proxy_off", "Выключить системный прокси", None)];
    let mode_r = run_ex(&["gsettings", "get", "org.gnome.system.proxy", "mode"], 6);
    let mode = mode_r.text().to_string();
    match mode.trim().trim_matches('\'') {
        "none" => out.push(Check::new("net", "Системный прокси", OK, "выключен (сеть напрямую)")),
        "manual" => match gnome_proxies().first() {
            Some(p) => out.push(crate::proxyaddr::system_row("Системный прокси", p, off_fix())),
            None => out.push(Check::new("net", "Системный прокси", WARN, "ручной, но без адреса").fixes(off_fix())),
        },
        "auto" => out.push(crate::proxyaddr::pac_row(&gsettings(&["org.gnome.system.proxy", "autoconfig-url"]), off_fix())),
        // gsettings завис или не запустился — не «прокси выключен». (Нет схемы GNOME — не GNOME, строки нет.)
        _ if matches!(mode_r, crate::cmd::Outcome::Timeout { .. } | crate::cmd::Outcome::SpawnError(_)) => {
            out.push(Check::new("net", "Системный прокси", INFO, "не удалось проверить").unknown().hint("gsettings не ответил вовремя."));
        }
        _ => {}
    }
    out.extend(crate::proxyaddr::env_row());
    out
}

/// Связь с сайтом — одна проверка для всех строк «Связь с …» (Claude, Codex, Gemini…).
/// Любой ответ сервера (401/404/405…) — достучаться можно. Различаем:
/// - 451 — сайт сам говорит «недоступно по закону» (блок по стране);
/// - 403 — «запрещено»: так отвечают и при блоке по стране, и защита от ботов, и рабочий фильтр — без диагноза;
/// - ошибка сертификата — чаще всего неверные часы (или сеть подменяет сертификаты: антивирус, фильтр);
/// - напрямую не пускают, а задан системный прокси — пробуем через него (рабочие сети).
pub(crate) fn site_row(cat: &'static str, title: &str, url: &str) -> Check {
    let ((code, _, err), ms) = http_get_ms(url, 7, 1000, None);
    let (code, err, ms, via) = match code {
        Some(_) => (code, err, ms, None),
        None => match configured_proxy() {
            Some(px) => {
                let ((c2, _, e2), ms2) = http_get_ms(url, 7, 1000, Some(&px.url()));
                if c2.is_some() {
                    (c2, e2, ms2, Some(px.shown()))
                } else {
                    (None, err, ms, None)
                }
            }
            None => (None, err, ms, None),
        },
    };
    let via_s = via.map(|v| format!(" · через прокси {v}")).unwrap_or_default();
    let low = err.to_lowercase();
    match code {
        Some(c) if c < 500 && c != 403 && c != 451 => Check::new(cat, title, OK, &format!("{ms:.0} мс{via_s}")),
        Some(451) => Check::new(cat, title, BAD, "блок по стране (451)").hint("Сайт не работает в этой стране. Нужен VPN."),
        Some(403) => Check::new(cat, title, WARN, "доступ запрещён (403)")
            .hint("Сервер отказал. Так бывает при блоке по стране (тогда поможет VPN), при защите от ботов и в рабочих сетях с фильтром."),
        Some(c) => Check::new(cat, title, WARN, &format!("ошибка {c}")).hint("Похоже, сбой на их стороне."),
        // Сертификат просрочен или «ещё не действует» — почти всегда неверные часы.
        None if low.contains("notvalidyet") || low.contains("expired") => Check::new(cat, title, WARN, "ошибка сертификата")
            .hint("Сертификат сайта для этого компьютера просрочен или ещё не действует — чаще всего неверные часы (смотри «Системное время»)."),
        // Чужой выпускающий: сеть подменяет сертификаты (антивирус, рабочий фильтр, свой центр сертификации).
        // Связь при этом есть — нет доверия; программа доверяет только общеизвестным центрам, браузер может работать.
        None if low.contains("certificate") || low.contains("unknownissuer") => Check::new(cat, title, WARN, "сертификату нет доверия").hint(
            "Связь есть, но сертификат сайта выдан не тем, кого программа знает. Так бывает, когда сеть подменяет \
                 сертификаты: антивирус с проверкой HTTPS, рабочий фильтр. Браузер при этом может работать. Если сеть чужая — осторожно.",
        ),
        None => Check::new(cat, title, BAD, "нет ответа").hint(if err.is_empty() {
            "Не достучаться. Проверь интернет и VPN."
        } else {
            &err
        }),
    }
}

pub(crate) fn check_claude_reach() -> Vec<Check> {
    let mut out = Vec::new();
    for (title, url) in [("Claude API", "https://api.anthropic.com/v1/models"), ("claude.ai", "https://claude.ai/favicon.ico")] {
        out.push(site_row("net", &format!("Связь с {title}"), url));
    }
    let (_, body, _) = http_get("https://status.claude.com/api/v2/status.json", 6, 200_000, None);
    if let Ok(j) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(st) = j.get("status") {
            let ind = st.get("indicator").and_then(|x| x.as_str()).unwrap_or("none");
            let status = match ind {
                "none" => OK,
                "minor" => WARN,
                _ => BAD,
            };
            let value = if ind == "none" {
                "всё работает".to_string()
            } else {
                st.get("description").and_then(|x| x.as_str()).unwrap_or(ind).to_string()
            };
            out.push(Check::new("net", "Серверы Anthropic", status, &value).hint(if ind == "none" {
                ""
            } else {
                "Это проблема на их стороне, не у тебя."
            }));
        }
    }
    out
}

// ───────────────────────────── Графика ─────────────────────────────

/// Видеокарты на шине PCI (Linux /sys): (производитель, драйвер ядра). Производители: 10de — NVIDIA,
/// 1002 — AMD, 8086 — Intel. Только устройства-«дисплеи» (класс 0x03…).
pub(crate) fn pci_gpus() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/bus/pci/devices") else { return out };
    for e in rd.flatten() {
        let p = e.path();
        let class = std::fs::read_to_string(p.join("class")).unwrap_or_default();
        if !class.trim().starts_with("0x03") {
            continue;
        }
        let vendor = std::fs::read_to_string(p.join("vendor")).unwrap_or_default().trim().trim_start_matches("0x").to_lowercase();
        let driver =
            std::fs::read_link(p.join("driver")).ok().and_then(|l| l.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
        out.push((vendor, driver));
    }
    out
}

/// Кто рисует экран и как NVIDIA (если она вообще есть). Раньше проверка была под один компьютер
/// (NVIDIA + AMD): на машине без NVIDIA давала «Драйвер NVIDIA: беда» и кнопку «рисовать на NVIDIA».
pub(crate) fn check_gpu() -> Vec<Check> {
    let mut out = Vec::new();
    let gpus = pci_gpus();
    let nvidia = gpus.iter().find(|(v, _)| v == "10de");
    let has_prime = run_ex(&["prime-select", "query"], 6);
    // переключать на NVIDIA можно, только если она есть и есть prime-select (Ubuntu)
    let can_prime = nvidia.is_some() && has_prime.answered();
    let prime_fix = || if can_prime { vec![fx("prime_nvidia", "Рисовать на NVIDIA (prime-select)", None)] } else { vec![] };
    let (_, gl) = run(&["glxinfo", "-B"], 8);
    let renderer = gl
        .lines()
        .find_map(|l| l.find("OpenGL renderer string: ").map(|i| l[i + "OpenGL renderer string: ".len()..].trim().to_string()))
        .unwrap_or_default();
    let low = renderer.to_lowercase();
    let integrated = if low.contains("amd") || low.contains("radeon") {
        "встроенная AMD"
    } else if low.contains("intel") {
        "встроенная Intel"
    } else {
        "другая видеокарта"
    };
    if renderer.is_empty() {
        out.push(Check::new("gpu", "Кто рисует экран", INFO, "не удалось узнать").hint("Нет программы glxinfo (mesa-utils) или она не ответила."));
    } else if low.contains("llvmpipe") || low.contains("softpipe") || low.contains("software") {
        out.push(
            Check::new("gpu", "Кто рисует экран", BAD, "ПРОЦЕССОР (llvmpipe)")
                .hint(if gpus.is_empty() {
                    "Видеокарта не нашлась — всё рисует процессор (так бывает в виртуальной машине)."
                } else {
                    "Видеокарта отвалилась, и всё рисует процессор: отсюда тормоза. Обычно лечится перезагрузкой."
                })
                .fixes(prime_fix()),
        );
    } else if low.contains("nvidia") {
        out.push(Check::new("gpu", "Кто рисует экран", OK, renderer.split('/').next().unwrap_or("")));
    } else if nvidia.is_some() {
        // есть NVIDIA, а рисует встроенная — так бывает нарочно (экономия батареи), поэтому «внимание», не «беда»
        out.push(
            Check::new("gpu", "Кто рисует экран", WARN, &trunc(&renderer, 40))
                .hint(&format!("Рисует {integrated}, а не NVIDIA. Если игры и графика тормозят — переключи на NVIDIA."))
                .fixes(prime_fix()),
        );
    } else {
        out.push(Check::new("gpu", "Кто рисует экран", OK, &trunc(&renderer, 40)));
    }

    // NVIDIA — только если она есть
    if let Some((_, driver)) = nvidia {
        let (code, smi) = run(
            &[
                "nvidia-smi",
                "--query-gpu=name,driver_version,temperature.gpu,utilization.gpu,memory.used,memory.total",
                "--format=csv,noheader,nounits",
            ],
            8,
        );
        if code == 0 && !smi.is_empty() {
            let p: Vec<&str> = smi.lines().next().unwrap_or("").split(',').map(|x| x.trim()).collect();
            let parsed = (|| -> Option<(i64, i64, i64)> { Some((p.get(2)?.parse().ok()?, p.get(4)?.parse().ok()?, p.get(5)?.parse().ok()?)) })();
            match (parsed, p.get(1), p.get(3), p.first()) {
                (Some((temp, used, total)), Some(drv), Some(util), Some(name)) => {
                    let st = if temp < 80 {
                        OK
                    } else if temp < 88 {
                        WARN
                    } else {
                        BAD
                    };
                    out.push(Check::new("gpu", "Драйвер NVIDIA", OK, drv));
                    out.push(
                        Check::new(
                            "gpu",
                            "Видеокарта",
                            st,
                            &format!("{temp}°C · загрузка {util}% · {}/{} ГБ", used.div_euclid(1024), total.div_euclid(1024)),
                        )
                        .hint(name),
                    );
                }
                _ => out.push(Check::new("gpu", "Драйвер NVIDIA", OK, &trunc(&smi, 50))),
            }
        } else if driver == "nouveau" {
            out.push(
                Check::new("gpu", "Драйвер NVIDIA", INFO, "открытый (nouveau)")
                    .hint("Работает открытый драйвер: для обычной работы хватает, для игр и ИИ лучше фирменный NVIDIA."),
            );
        } else if driver.is_empty() {
            out.push(
                Check::new("gpu", "Драйвер NVIDIA", WARN, "не загружен")
                    .hint("Карта NVIDIA есть, но драйвер к ней не подключён. Часто после обновления ядра — перезагрузись."),
            );
        } else {
            out.push(
                Check::new("gpu", "Драйвер NVIDIA", BAD, "не отвечает")
                    .hint("Драйвер загружен, но nvidia-smi не отвечает: драйвер сломался после обновления ядра. Перезагрузись."),
            );
        }
        if has_prime.answered() {
            let mode = has_prime.text().trim().to_string();
            let names: HashMap<&str, &str> =
                HashMap::from([("on-demand", "по запросу (гибрид)"), ("nvidia", "всегда NVIDIA"), ("intel", "всегда встроенная")]);
            // «всегда встроенная» — выбор человека (экономия батареи), не ошибка
            out.push(
                Check::new("gpu", "Режим PRIME", INFO, names.get(mode.as_str()).copied().unwrap_or(&mode))
                    .hint("«По запросу» иногда отдаёт рабочий стол встроенной карте."),
            );
        }
    }
    let sess = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "?".into());
    out.push(Check::new("gpu", "Сессия", INFO, &sess));
    out
}

// ───────────────────────────── Часы ─────────────────────────────

/// Дни от 1970-01-01 до гражданской даты (алгоритм Говарда Хиннанта).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Насколько наши часы расходятся с интернетом: секунды (+ — наши спешат). Одна сверка для всех систем.
///
/// Время берём из заголовка Date у серверов разных компаний сразу (параллельно), поправку на дорогу —
/// середина запроса, итог — медиана. Нужны хотя бы два сервера, согласные между собой (±10 с): один сервер
/// (или прокси, страница входа в сеть) может отдавать неверное время, и раньше часы объявлялись «врущими».
pub(crate) fn clock_offset() -> Option<f64> {
    // по http, без TLS: при сильно неверных часах проверка сертификата падает, и сверить было бы нечем —
    // ровно тогда, когда сверка нужнее всего
    const URLS: [&str; 4] = [
        "http://connectivitycheck.gstatic.com/generate_204",
        "http://www.msftconnecttest.com/connecttest.txt",
        "http://captive.apple.com/hotspot-detect.html",
        "http://ya.ru/",
    ];
    let (tx, rx) = mpsc::channel();
    for u in URLS {
        let tx = tx.clone();
        thread::spawn(move || {
            let t0 = SystemTime::now();
            let d = http_date(u, 5).and_then(|d| parse_http_date(&d));
            let t1 = SystemTime::now();
            let mid = |a: SystemTime| a.duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            // Date округлён вниз до секунды — в среднем сервер на 0,5 с «позже»
            let _ = tx.send(d.map(|real| (mid(t0) + mid(t1)) / 2.0 - (real + 0.5)));
        });
    }
    drop(tx);
    let until = Instant::now() + Duration::from_secs(7);
    let mut offs: Vec<f64> = Vec::new();
    while let Ok(r) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        offs.extend(r);
    }
    agreed_median(offs)
}

/// Медиана, если хотя бы два значения в пределах 10 с от неё; иначе None (источники спорят).
fn agreed_median(mut offs: Vec<f64>) -> Option<f64> {
    offs.sort_by(|a, b| a.total_cmp(b));
    let m = *offs.get(offs.len() / 2)?;
    (offs.iter().filter(|o| (*o - m).abs() <= 10.0).count() >= 2).then_some(m)
}

/// Строка «Системное время» по расхождению — общая для всех систем. synced — система сама сверяет часы
/// и говорит, что сверила (тогда небольшое расхождение с сайтами — их неточность, а не наших часов).
pub(crate) fn clock_row(off: Option<f64>, synced: Option<bool>, fix: Vec<Fix>) -> Check {
    let Some(off) = off else {
        let hm = chrono_hm();
        return Check::new("sys", "Системное время", INFO, &hm).hint("Не с чем сверить: сайты не ответили или назвали разное время.");
    };
    let a = off.abs();
    // «+0», а не «-0»
    let r = off.round() + 0.0;
    if a < 60.0 {
        return Check::new("sys", "Системное время", OK, &format!("точное ({r:+.0} с)"));
    }
    if a < 90.0 && synced == Some(true) {
        // система говорит «сверено», а сайты расходятся на минуту — скорее неточны сайты (кэш, прокси)
        return Check::new("sys", "Системное время", OK, &format!("сверено системой ({r:+.0} с к сайтам)"));
    }
    let mins = a as i64 / 60;
    let (h, m) = (mins / 60, mins % 60);
    let days = h / 24;
    let txt = if mins == 0 {
        format!("{a:.0} с")
    } else if days >= 365 {
        format!("{:.1} года", days as f64 / 365.0).replace('.', ",")
    } else if days >= 2 {
        format!("{days} дн")
    } else if h > 0 {
        format!("{h} ч {m} мин")
    } else {
        format!("{m} мин")
    };
    let dir = if off > 0.0 { "спешит" } else { "отстаёт" };
    // пороги на всех системах одни: с 60 с — внимание, с 90 с VPN (VMess/xray) уже не подключается
    let (st, hint) = if a < 90.0 {
        (WARN, "Ещё чуть-чуть — и VPN перестанет подключаться. Сверь часы.")
    } else {
        (BAD, "Из-за неверных часов VPN (VMess/xray) может не подключаться, а сайты — ругаться на сертификаты.")
    };
    Check::new("sys", "Системное время", st, &format!("{dir} на {txt}")).hint(hint).fixes(fix)
}

/// Время «ЧЧ:ММ» по часам компьютера (местное — как показывает система).
fn chrono_hm() -> String {
    #[cfg(unix)]
    {
        let r = run(&["date", "+%H:%M"], 6).1;
        if !r.trim().is_empty() {
            return r.trim().to_string();
        }
    }
    let s = now_secs() as i64;
    format!("{:02}:{:02} UTC", s / 3600 % 24, s / 60 % 60)
}

fn parse_http_date(s: &str) -> Option<f64> {
    let t: Vec<&str> = s.split_whitespace().collect();
    let off = if t.first()?.ends_with(',') { 1 } else { 0 };
    let day: i64 = t.get(off)?.parse().ok()?;
    let mname = *t.get(off + 1)?;
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
        .iter()
        .position(|m| m.eq_ignore_ascii_case(mname))? as i64
        + 1;
    let year: i64 = t.get(off + 2)?.parse().ok()?;
    let hms: Vec<i64> = t.get(off + 3)?.split(':').filter_map(|x| x.parse().ok()).collect();
    if hms.len() != 3 {
        return None;
    }
    Some((days_from_civil(year, mon, day) * 86400 + hms[0] * 3600 + hms[1] * 60 + hms[2]) as f64)
}

/// Сверка часов с интернетом: VPN (VMess/xray) не подключается, если часы врут > 90 с.
pub(crate) fn check_clock() -> Vec<Check> {
    let fix = || vec![fx("time_sync", "Включить автосверку времени", None)];
    // параллельно со сверкой — что говорит сама система: включена ли автосверка и сверено ли
    let off = thread::spawn(clock_offset);
    let show = |p: &str| {
        let r = run_ex(&["timedatectl", "show", "-p", p, "--value"], 6);
        r.answered().then(|| r.text().trim().to_string())
    };
    let ntp_on = show("NTP");
    let synced = show("NTPSynchronized").map(|v| v == "yes");
    let tz = show("Timezone").unwrap_or_default();
    let mut out = vec![clock_row(off.join().ok().flatten(), synced, fix())];
    // timedatectl нет или не ответил — про автосверку не знаем: и не предлагаем её включать
    let auto_off = ntp_on.as_deref() == Some("no");
    out.push(
        Check::new("sys", "Часовой пояс", INFO, if tz.is_empty() { "?" } else { &tz })
            .hint(if auto_off { "Автосверка времени выключена." } else { "" })
            .fixes(if auto_off { fix() } else { vec![] }),
    );
    out
}

// ───────────────────────────── Темы ─────────────────────────────

/// Что считаем «темой»: (вид, схема/путь, ключ, подпись). Как THEME_KEYS в Python.
const THEME_KEYS: [(&str, &str, &str, &str); 6] = [
    ("gsettings", "org.gnome.desktop.interface", "gtk-theme", "Тема окон (GTK)"),
    ("gsettings", "org.gnome.desktop.interface", "icon-theme", "Иконки"),
    ("gsettings", "org.gnome.desktop.interface", "cursor-theme", "Курсор"),
    ("gsettings", "org.gnome.desktop.interface", "color-scheme", "Светлая/тёмная"),
    ("gsettings", "org.gnome.desktop.wm.preferences", "button-layout", "Кнопки окон"),
    ("dconf", "/org/gnome/shell/extensions/user-theme/name", "", "Тема панели (Shell)"),
];

/// Расширения GNOME, которые бывают «доком».
const DOCKS: [&str; 3] = ["dash2dock-lite@icedman.github.com", "dash-to-dock@micxgx.gmail.com", "ubuntu-dock@ubuntu.com"];

/// Файл эталона тем. Тот же, что у Python-версии (~/.config/aptechka/snapshot.json): эталон общий.
pub fn snapshot_path() -> String {
    // в проверенной закрытой папке настроек (своя, 0700, не ссылка), как и все файлы Аптечки
    crate::store::private_dir().join("snapshot.json").to_string_lossy().into_owned()
}

/// Ключ темы из нашего списка (для починки «вернуть как было»: чужой ключ dconf/gsettings не выполняем).
pub(crate) fn theme_key_known(key: &str) -> bool {
    THEME_KEYS.iter().any(|(kind, a, b, _)| key_id(kind, a, b) == key)
}

fn key_id(kind: &str, a: &str, b: &str) -> String {
    format!("{kind}|{a}|{b}")
}

/// Прочитать значение одного ключа (только чтение). Пустая строка — не вышло.
/// Значение ключа темы. None — прочитать не удалось (нет gsettings/dconf, не ответил, нет схемы):
/// это «не знаю», а не «пусто» — иначе любой сбой чтения выглядел как «тема сбилась».
fn read_key(kind: &str, a: &str, b: &str) -> Option<String> {
    let r = if kind == "gsettings" { run_ex(&["gsettings", "get", a, b], 6) } else { run_ex(&["dconf", "read", a], 6) };
    (r.code() == 0).then(|| r.text().to_string())
}

/// Включённые расширения GNOME; None — не узнать (нет gnome-extensions или GNOME не ответил).
fn enabled_extensions() -> Option<Vec<String>> {
    let r = run_ex(&["gnome-extensions", "list", "--enabled"], 6);
    (r.code() == 0).then(|| r.text().split_whitespace().map(|s| s.to_string()).collect())
}

/// Эталон тем: значения ключей, включённые расширения, время и наличие gtk.css для GTK4.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub keys: Vec<(String, String)>,
    pub extensions: Vec<String>,
    pub time: String,
    pub gtk4css: bool,
}

impl Snapshot {
    pub fn key(&self, id: &str) -> Option<&str> {
        self.keys.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_str())
    }
}

/// Снять текущее состояние тем (только чтение системы, ничего не пишет).
fn capture_snapshot() -> Snapshot {
    // Windows и macOS: свои ключи (реестр, defaults) — looksnap.rs; расширений GNOME и gtk.css там нет
    #[cfg(any(windows, target_os = "macos"))]
    {
        let (keys, time) = crate::looksnap::capture();
        return Snapshot { keys, extensions: Vec::new(), time, gtk4css: false };
    }
    #[allow(unreachable_code)]
    let (_, time) = run(&["date", "+%a %b %e %H:%M:%S %Y"], 6); // как time.ctime() в Python
    Snapshot {
        // непрочитанные ключи в эталон не попадают (их не с чем будет сравнивать)
        keys: THEME_KEYS.iter().filter_map(|(kind, a, b, _)| Some((key_id(kind, a, b), read_key(kind, a, b)?))).collect(),
        extensions: enabled_extensions().unwrap_or_default(),
        time,
        gtk4css: std::path::Path::new(&format!("{}/.config/gtk-4.0/gtk.css", home())).exists(),
    }
}

/// Прочитать файл эталона; None — файла нет или он испорчен.
fn read_snapshot_file() -> Option<Snapshot> {
    let txt = crate::store::read_text(snapshot_path()).ok()?;
    let j: serde_json::Value = serde_json::from_str(&txt).ok()?;
    let strs = |v: Option<&serde_json::Value>| -> Vec<String> {
        v.and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    let keys = j
        .get("keys")
        .and_then(|k| k.as_object())
        .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    Some(Snapshot {
        keys,
        extensions: strs(j.get("extensions")),
        time: j.get("time").and_then(|t| t.as_str()).unwrap_or("?").to_string(),
        gtk4css: j.get("gtk4css").and_then(|t| t.as_bool()).unwrap_or(false),
    })
}

/// Эталон для проверок. Отличие от Python: если файла нет, текущее состояние берётся
/// «в уме» и на диск НЕ пишется (проверки только читают). Записывает эталон кнопка «Запомнить…».
pub fn load_snapshot() -> Snapshot {
    read_snapshot_file().unwrap_or_else(capture_snapshot)
}

/// Записать текущее состояние тем как эталон (формат файла как у Python: отступ 2, без \uXXXX).
pub fn take_snapshot() -> Result<Snapshot, String> {
    let snap = capture_snapshot();
    let q = |s: &str| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into());
    let keys: Vec<String> = snap.keys.iter().map(|(k, v)| format!("    {}: {}", q(k), q(v))).collect();
    let exts: Vec<String> = snap.extensions.iter().map(|e| format!("    {}", q(e))).collect();
    let body = format!(
        "{{\n  \"keys\": {{\n{}\n  }},\n  \"extensions\": [{}],\n  \"time\": {},\n  \"gtk4css\": {}\n}}",
        keys.join(",\n"),
        if exts.is_empty() { String::new() } else { format!("\n{}\n  ", exts.join(",\n")) },
        q(&snap.time),
        snap.gtk4css
    );
    let path = snapshot_path();
    crate::store::write_private(std::path::Path::new(&path), body.as_bytes()).map_err(|e| e.to_string())?;
    log(&format!("эталон тем записан: {path}"));
    Ok(snap)
}

fn dir_has(name: &str, dirs: &[String]) -> bool {
    dirs.iter().any(|d| std::path::Path::new(d).join(name).is_dir())
}

/// Состояние расширения из `gnome-extensions info`: регулярка `State: (\S+(?: \S+)?)`.
fn ext_state(info: &str) -> String {
    let mut from = 0;
    while let Some(i) = info[from..].find("State: ") {
        let rest = &info[from + i + "State: ".len()..];
        let first: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
        if !first.is_empty() {
            let after = &rest[first.len()..];
            let second: String = match after.strip_prefix(' ') {
                Some(t) => t.chars().take_while(|c| !c.is_whitespace()).collect(),
                None => String::new(),
            };
            return if second.is_empty() { first } else { format!("{first} {second}") };
        }
        from += i + 1;
    }
    "?".to_string()
}

pub(crate) fn check_themes() -> Vec<Check> {
    let mut out = Vec::new();
    let snap = load_snapshot();
    let h = home();
    let theme_dirs = [format!("{h}/.themes"), format!("{h}/.local/share/themes"), "/usr/share/themes".to_string()];
    let icon_dirs = [format!("{h}/.icons"), format!("{h}/.local/share/icons"), "/usr/share/icons".to_string()];
    for (kind, a, b, title) in THEME_KEYS {
        let key = key_id(kind, a, b);
        let Some(now) = read_key(kind, a, b) else {
            out.push(Check::new("theme", title, INFO, "не проверить").unknown().hint("Настройка не прочиталась (gsettings/dconf не ответили)."));
            continue;
        };
        let want = snap.key(&key).unwrap_or(&now).to_string();
        let name = now.trim_matches('\'').to_string();
        let shown = if name.is_empty() { "—".to_string() } else { name.clone() };
        if now != want {
            out.push(Check::new("theme", title, WARN, &shown).hint(&format!("Сбилось! Было: {}", want.trim_matches('\''))).fixes(vec![fx(
                "theme_restore",
                "Вернуть как было",
                Some(&key),
            )]));
            continue;
        }
        // тема выставлена, но папки с ней нет — тоже поломка
        // (в Python тут `str | bool` падало при пустом имени панели; здесь пустое имя = не поломка)
        // Adwaita и HighContrast встроены в сам GTK — папки у них может не быть
        let builtin = b == "gtk-theme" && matches!(name.as_str(), "Adwaita" | "Adwaita-dark" | "HighContrast" | "HighContrastInverse");
        let missing = !name.is_empty()
            && !builtin
            && ((b == "gtk-theme" || kind == "dconf") && !dir_has(&name, &theme_dirs)
                || (b == "icon-theme" || b == "cursor-theme") && !dir_has(&name, &icon_dirs));
        if missing {
            out.push(Check::new("theme", title, BAD, &shown).hint("Тема выбрана, но её файлов нет. Её стёрло обновление, нужно переустановить."));
        } else {
            out.push(Check::new("theme", title, OK, &shown));
        }
    }

    if snap.gtk4css && !std::path::Path::new(&format!("{h}/.config/gtk-4.0/gtk.css")).exists() {
        out.push(
            Check::new("theme", "Стиль GTK4/libadwaita", WARN, "пропал gtk.css")
                .hint("Новые приложения стали «стандартными». Переустанови WhiteSur/Tahoe с флагом -l."),
        );
    }

    // экран заблокирован — GNOME сам выключает расширения до разблокировки: их «inactive» сейчас не поломка
    if screen_locked() {
        if !snap.extensions.is_empty() {
            out.push(
                Check::new("theme", "Расширения", INFO, "экран заблокирован")
                    .hint("Пока экран заблокирован, GNOME выключает расширения — проверю после разблокировки."),
            );
        }
        out.push(Check::new("theme", "Эталон", INFO, &snap.time).hint("С этим состоянием сравниваются темы.").fixes(vec![fx(
            "snapshot",
            "Запомнить текущее как норму",
            None,
        )]));
        return out;
    }
    let Some(enabled) = enabled_extensions() else {
        if !snap.extensions.is_empty() {
            out.push(
                Check::new("theme", "Расширения", INFO, "не проверить").unknown().hint("gnome-extensions не ответил — не знаю, какие включены."),
            );
        }
        out.push(Check::new("theme", "Эталон", INFO, &snap.time).hint("С этим состоянием сравниваются темы.").fixes(vec![fx(
            "snapshot",
            "Запомнить текущее как норму",
            None,
        )]));
        return out;
    };
    for uuid in &snap.extensions {
        let short = uuid.split('@').next().unwrap_or(uuid);
        let (_, info) = run(&["gnome-extensions", "info", uuid], 6);
        let state = ext_state(&info);
        let fix = || vec![fx("ext_restart", "Перезапустить", Some(uuid))];
        let title = format!("Расширение {short}");
        if !enabled.contains(uuid) {
            out.push(Check::new("theme", &title, WARN, "выключено").hint("Было включено, а теперь нет.").fixes(vec![fx(
                "ext_enable",
                "Включить",
                Some(uuid),
            )]));
        } else if matches!(state.as_str(), "ACTIVE" | "ENABLED") {
            out.push(Check::new("theme", &title, OK, "работает").fixes(fix()));
        } else if matches!(state.as_str(), "ERROR" | "OUT OF DATE" | "OUT_OF_DATE") {
            out.push(
                Check::new("theme", &title, BAD, &state.to_lowercase())
                    .hint(
                        "Сломалось, скорее всего после обновления GNOME. Попробуй перезапустить \
                 или обновить на extensions.gnome.org.",
                    )
                    .fixes(fix()),
            );
        } else {
            out.push(Check::new("theme", &title, WARN, &state.to_lowercase()).fixes(fix()));
        }
    }
    out.push(Check::new("theme", "Эталон", INFO, &snap.time).hint("С этим состоянием сравниваются темы.").fixes(vec![fx(
        "snapshot",
        "Запомнить текущее как норму",
        None,
    )]));
    out
}

/// Имя активного окна, если оно на весь экран (X11), иначе None.
fn active_fullscreen() -> Option<String> {
    let (code, w) = run(&["xprop", "-root", "_NET_ACTIVE_WINDOW"], 6);
    let w = if code == 0 { w.split_whitespace().last().unwrap_or("").to_string() } else { String::new() };
    if w.is_empty() || w == "0x0" {
        return None;
    }
    let (_, st) = run(&["xprop", "-id", &w, "_NET_WM_STATE", "WM_CLASS"], 6);
    if !st.contains("_NET_WM_STATE_FULLSCREEN") {
        return None;
    }
    // WM_CLASS(STRING) = "instance", "Class": берём второе слово
    let class = st.split("WM_CLASS(STRING) = \"").nth(1).and_then(|r| {
        let (_, tail) = r.split_once("\", \"")?;
        let c = tail.split('"').next()?;
        if c.is_empty() {
            None
        } else {
            Some(c.to_string())
        }
    });
    Some(class.unwrap_or_else(|| "окно".to_string()))
}

/// Сколько кнопок-значков оболочки GNOME сейчас показано на экране (док и панель приложений).
/// Через дерево доступности (AT-SPI, python3-gi — есть в Ubuntu с GNOME); None — спросить не удалось.
fn dock_buttons_shown() -> Option<usize> {
    const PROBE: &str = r#"
import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
d = Atspi.get_desktop(0)
n = 0
def walk(x, depth):
    global n
    if x is None or depth > 25:
        return
    try:
        if x.get_role_name() == 'push button':
            st = x.get_state_set()
            e = x.get_extents(Atspi.CoordType.SCREEN)
            # значок приложения: есть название, показан и лежит на экране
            if x.get_name() and st.contains(Atspi.StateType.SHOWING) and e.width > 0 and e.x > -10000:
                n += 1
        for j in range(x.get_child_count()):
            walk(x.get_child_at_index(j), depth + 1)
    except Exception:
        pass
for i in range(d.get_child_count()):
    a = d.get_child_at_index(i)
    if a is not None and a.get_name() == 'gnome-shell':
        walk(a, 0)
        print('shown', n)
"#;
    let (code, out) = run(&["python3", "-c", PROBE], 8);
    if code != 0 {
        return None;
    }
    out.lines().find_map(|l| l.strip_prefix("shown ")).and_then(|v| v.trim().parse().ok())
}

/// Расширение дока этого пользователя: включённое сейчас, иначе — бывшее включённым в эталоне.
pub(crate) fn current_dock() -> Option<String> {
    let enabled = enabled_extensions()?;
    DOCKS.iter().find(|d| enabled.iter().any(|e| e == *d)).map(|d| d.to_string()).or_else(|| {
        let snap = load_snapshot();
        DOCKS.iter().find(|d| snap.extensions.iter().any(|e| e == *d)).map(|d| d.to_string())
    })
}

/// Заблокирован ли экран: у GNOME (org.gnome.ScreenSaver.GetActive) или по logind (LockedHint сеанса).
pub(crate) fn screen_locked() -> bool {
    let r = run_ex(
        &[
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnome.ScreenSaver",
            "--object-path",
            "/org/gnome/ScreenSaver",
            "--method",
            "org.gnome.ScreenSaver.GetActive",
        ],
        4,
    );
    if r.code() == 0 {
        return r.text().contains("true");
    }
    let sid = std::env::var("XDG_SESSION_ID").unwrap_or_default();
    !sid.is_empty() && run(&["loginctl", "show-session", &sid, "-p", "LockedHint", "--value"], 4).1.trim() == "yes"
}

pub(crate) fn check_dock() -> Vec<Check> {
    if screen_locked() {
        return Vec::new(); // на экране блокировки дока нет — это не поломка
    }
    // gnome-extensions не ответил (не GNOME, занят) или дока нет — про док ничего не утверждаем
    let Some(dock) = current_dock() else { return Vec::new() };
    let fix = || vec![fx("dock_repair", "Починить док", Some(&dock))];
    let (_, info) = run(&["gnome-extensions", "info", &dock], 6);
    let state = ext_state(&info);
    if matches!(state.as_str(), "ACTIVE" | "ENABLED") {
        // расширение «работает», но сам док мог пропасть с экрана (GNOME отцепил его значки) — спросим
        // у оболочки через дерево доступности, показан ли хоть один значок дока
        // «пропал» — только если так два раза подряд (минимум с интервалом проверки): на миг док прячется сам
        // (полноэкранное окно в Wayland, где его не видно через xprop, анимация)
        static ZEROS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let zero = dock_buttons_shown() == Some(0) && active_fullscreen().is_none();
        let n = if zero {
            ZEROS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
        } else {
            ZEROS.store(0, std::sync::atomic::Ordering::Relaxed);
            0
        };
        if n >= 2 {
            return vec![Check::new("theme", "Док", BAD, "пропал с экрана")
                .hint(
                    "Расширение дока включено, но самого дока на экране нет — так бывает после сбоя GNOME \
                 (например, когда док прятали на время полноэкранного окна). Нажми «Починить док».",
                )
                .fixes(fix())];
        }
        return vec![Check::new("theme", "Док", OK, "на месте").fixes(fix())];
    }
    if let Some(fs) = active_fullscreen() {
        return vec![Check::new("theme", "Док", INFO, &format!("спрятан: {fs} на весь экран"))
            .hint(
                "Это нормально: док прячется, пока окно в полноэкранном режиме (F11). \
             Выйди из него — док вернётся сам.",
            )
            .fixes(fix())];
    }
    vec![Check::new("theme", "Док", BAD, "пропал")
        .hint(&format!("Расширение дока не работает (состояние: {}). Нажми «Починить док».", state.to_lowercase()))
        .fixes(fix())]
}

// ───────────────────────────── Репозитории ─────────────────────────────

/// Адреса репозиториев из настроек apt (старый и новый формат): хост → первый адрес.
fn apt_hosts() -> BTreeMap<String, String> {
    let mut files: Vec<std::path::PathBuf> = vec!["/etc/apt/sources.list".into()];
    if let Ok(rd) = std::fs::read_dir("/etc/apt/sources.list.d") {
        for e in rd.flatten() {
            let p = e.path();
            if matches!(p.extension().and_then(|x| x.to_str()), Some("list" | "sources")) {
                files.push(p);
            }
        }
    }
    let mut urls: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in files {
        let Ok(txt) = crate::store::read_text_lossy(&f) else { continue };
        if f.extension().and_then(|x| x.to_str()) == Some("sources") {
            // блоки разделены пустыми строками
            let mut blocks: Vec<Vec<&str>> = vec![Vec::new()];
            for line in txt.lines() {
                if line.trim().is_empty() {
                    blocks.push(Vec::new());
                } else if let Some(b) = blocks.last_mut() {
                    b.push(line);
                }
            }
            for block in blocks {
                let off = block.iter().any(|l| {
                    l.get(..8).is_some_and(|p| p.eq_ignore_ascii_case("enabled:"))
                        && l[8..].trim_start().get(..2).is_some_and(|p| p.eq_ignore_ascii_case("no"))
                });
                if off {
                    continue;
                }
                if let Some(rest) = block.iter().find_map(|l| l.strip_prefix("URIs:")) {
                    urls.extend(rest.split_whitespace().map(String::from));
                }
            }
        } else {
            for line in txt.lines() {
                let Some(rest) = line.trim_start().strip_prefix("deb") else { continue };
                let rest = rest.strip_prefix("-src").unwrap_or(rest);
                if !rest.starts_with(char::is_whitespace) {
                    continue;
                }
                let mut rest = rest.trim_start();
                if rest.starts_with('[') {
                    if let Some(end) = rest.find(']') {
                        let tail = &rest[end + 1..];
                        if tail.starts_with(char::is_whitespace) {
                            rest = tail.trim_start();
                        }
                    }
                }
                if let Some(u) = rest.split_whitespace().next() {
                    urls.insert(u.to_string());
                }
            }
        }
    }
    let mut hosts = BTreeMap::new();
    for u in urls {
        let Some((scheme, after)) = u.split_once("://") else { continue };
        if scheme != "http" && scheme != "https" {
            continue;
        }
        // имя сервера — без «логин:пароль@» / «токен@» (закрытые репозитории): только оно и показывается
        let authority = after.split('/').next().unwrap_or("");
        let host = authority.rsplit('@').next().unwrap_or("");
        if !host.is_empty() {
            hosts.entry(host.to_string()).or_insert(u.clone());
        }
    }
    hosts
}

/// Доступ к репозиториям: отвечают ли серверы обновлений системы и программ.
pub(crate) fn check_repos() -> Vec<Check> {
    let hosts = apt_hosts();
    if hosts.is_empty() {
        return Vec::new();
    }
    let list: Vec<(String, String)> = hosts.into_iter().collect();
    // apt ходит через свой прокси (apt-cacher-ng, рабочий) — проверяем тем же путём, что и он
    let proxy = apt_proxy();
    let mut res: Vec<(String, bool)> = Vec::new();
    for chunk in list.chunks(8) {
        // до 8 запросов одновременно, как в Python
        let handles: Vec<_> = chunk
            .iter()
            .map(|(h, u)| {
                let (h, u, px) = (h.clone(), u.clone(), proxy.clone());
                thread::spawn(move || {
                    let px = if u.starts_with("https") { px.1 } else { px.0 };
                    let (code, _, _) = http_get(&u, 6, 200, px.as_deref());
                    // 401/403/407 — сервер ответил, но не пускает (закрытый репозиторий без доступа, прокси требует
                    // пароль): обновления оттуда не скачаются — это не «доступен»
                    (h, code.is_some_and(|c| c < 500 && !matches!(c, 401 | 403 | 407)))
                })
            })
            .collect();
        for (h, handle) in chunk.iter().zip(handles) {
            res.push(handle.join().unwrap_or((h.0.clone(), false)));
        }
    }
    let good = res.iter().filter(|(_, ok)| *ok).count();
    let mut bad: Vec<&str> = res.iter().filter(|(_, ok)| !*ok).map(|(h, _)| h.as_str()).collect();
    bad.sort();
    let n = format!("{good} из {}", res.len());
    if bad.is_empty() {
        return vec![Check::new("sys", "Доступ к репозиториям", OK, &format!("OK ({n})")).hint("Обновления системы и программ скачиваются.")];
    }
    let st = if good == 0 { BAD } else { WARN };
    let via = match &proxy.0 {
        Some(p) => format!(" (проверяла через прокси apt {})", crate::proxyaddr::ProxyAddr::parse(p).map(|x| x.shown()).unwrap_or_default()),
        None => String::new(),
    };
    vec![Check::new("sys", "Доступ к репозиториям", st, &format!("доступно {n}")).hint(&format!(
        "Не отвечают или не пускают: {}{via}. Обновления от них не скачаются. Часто это из-за VPN или прокси.",
        trunc(&bad.join(", "), 120)
    ))]
}

/// Прокси, через который ходит apt: (для http, для https). Из `apt-config dump` — итог всех файлов настроек,
/// как его видит сам apt; «DIRECT» и «false» — без прокси.
fn apt_proxy() -> (Option<String>, Option<String>) {
    let dump = run(&["apt-config", "dump"], 6).1;
    let get = |key: &str| {
        dump.lines()
            .find_map(|l| {
                let (k, v) = l.trim().split_once(' ')?;
                (k.eq_ignore_ascii_case(key)).then(|| v.trim().trim_end_matches(';').trim_matches('"').to_string())
            })
            .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("direct") && !v.eq_ignore_ascii_case("false"))
            .and_then(|v| crate::proxyaddr::ProxyAddr::parse(&v).map(|p| p.url()))
    };
    let http = get("Acquire::http::Proxy");
    (http.clone(), get("Acquire::https::Proxy").or(http))
}

// ───────────────────────────── Система ─────────────────────────────

/// Температура датчика и его «критическая» отметка (из самого датчика, если он её знает), °C.
fn hwmon_temp(names: &[&str]) -> Option<(f64, Option<f64>)> {
    let milli = |p: std::path::PathBuf| std::fs::read_to_string(p).ok().and_then(|s| s.trim().parse::<i64>().ok()).map(|v| v as f64 / 1000.0);
    for e in std::fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let d = e.path();
        let Ok(name) = std::fs::read_to_string(d.join("name")) else { continue };
        if names.contains(&name.trim()) {
            if let Some(t) = milli(d.join("temp1_input")).filter(|t| *t > 0.0) {
                let crit = milli(d.join("temp1_crit")).or_else(|| milli(d.join("temp1_max"))).filter(|c| *c > 50.0 && *c < 130.0);
                return Some((t, crit));
            }
        }
    }
    None
}

/// Процессора не хватает уже минуту: PSI /proc/pressure/cpu (some avg60 ≥ 25%) или, без него, средняя
/// загрузка за минуту ≥ 90% потоков.
fn cpu_starved(l1: f64, cores: usize) -> bool {
    match std::fs::read_to_string("/proc/pressure/cpu")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("some"))?.split_whitespace().find_map(|w| w.strip_prefix("avg60="))?.parse::<f64>().ok())
    {
        Some(avg60) => avg60 >= 25.0,
        None => l1 >= cores as f64 * 0.9,
    }
}

/// Доля времени, когда процессор был занят, за `span` (по /proc/stat: всё, кроме idle и iowait).
fn cpu_busy(span: Duration) -> Option<f64> {
    let sample = || -> Option<(u64, u64)> {
        let s = std::fs::read_to_string("/proc/stat").ok()?;
        let f: Vec<u64> = s.lines().next()?.strip_prefix("cpu ")?.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        // user nice system idle iowait irq softirq steal …: idle = idle + iowait; guest уже входит в user
        let idle = f.get(3)? + f.get(4).copied().unwrap_or(0);
        Some((idle, f.iter().take(8).sum()))
    };
    let (i0, t0) = sample()?;
    thread::sleep(span);
    let (i1, t1) = sample()?;
    let dt = t1.checked_sub(t0).filter(|d| *d > 0)?;
    Some(100.0 * (1.0 - (i1.saturating_sub(i0)) as f64 / dt as f64))
}

/// Место на дисках: домашняя папка — всегда; другие разделы (данные, второй диск) — только когда беда.
/// Порог и в гигабайтах, и в процентах: на диске 4 ТБ 20 ГБ — тесно, на 64 ГБ — нормально.
/// Ещё две частые поломки, при которых места «полно»: кончились иноды (миллионы мелких файлов — новый файл
/// не создать) и раздел перешёл в «только чтение» (так ext4 защищается при ошибках диска).
fn disk_rows() -> Vec<Check> {
    let all = crate::disks::list();
    if all.is_empty() {
        return vec![Check::new("sys", "Диск", INFO, "не проверить").unknown().hint("Не удалось узнать разделы дисков.")];
    }
    let home_dev = crate::disks::for_path(&all, &home()).map(|f| f.dev.clone());
    // разделы, которые и задуманы «только для чтения» (fstab с ro), не тревожат
    let fstab = crate::store::read_text_lossy("/etc/fstab").unwrap_or_default();
    let ro_planned = |t: &str| {
        fstab.lines().any(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.get(1) == Some(&t) && f.get(3).is_some_and(|o| o.split(',').any(|x| x == "ro"))
        })
    };
    let mut out = Vec::new();
    // /boot и /boot/efi — своя проверка (место под ядра)
    for fs in all.into_iter().filter(|f| !f.target.starts_with("/boot")) {
        let free_gb = fs.avail / 1e9;
        let pct = fs.avail / fs.size * 100.0;
        let is_home = home_dev.as_deref() == Some(fs.dev.as_str());
        let title = if is_home { "Диск".to_string() } else { format!("Диск {}", fs.target) };
        if fs.ro && !ro_planned(&fs.target) && (is_home || fs.target == "/") {
            out.push(Check::new("sys", &title, BAD, "только для чтения").hint(
                "Раздел перешёл в режим «только чтение» — так система защищает данные при ошибках диска. \
                 Сохранить ничего не получится. Перезагрузи компьютер; если повторится — диск пора проверить (sudo fsck) и, возможно, менять.",
            ));
            continue;
        }
        if let Some(fi) = fs.inodes_free.filter(|f| *f < 0.1) {
            out.push(Check::new("sys", &title, if fi < 0.02 { BAD } else { WARN }, &format!("кончаются иноды ({:.0}% свободно)", fi * 100.0))
                .hint("Место есть, но кончились «номерки» для файлов: новые файлы не создаются. Обычно виноваты миллионы мелких файлов (кэш, почта, node_modules)."));
            continue;
        }
        // маленький раздел (флешка, 300 МБ) судим по процентам, большой — ещё и по гигабайтам
        let st = if pct < 3.0 || (free_gb < 1.0 && pct < 10.0) {
            BAD
        } else if free_gb < 10.0 && pct < 10.0 {
            WARN
        } else {
            OK
        };
        // полный архивный диск, флешка с копиями, раздел Windows — это не авария: беда только у системных
        // разделов и домашнего, остальным — «заполнен» для сведения
        let system = is_home || matches!(fs.target.as_str(), "/" | "/home" | "/var" | "/usr" | "/tmp");
        let st = if system || st == OK { st } else { INFO };
        if !is_home && st == OK {
            continue;
        }
        let free_s = if free_gb < 1.0 { format!("{:.0} МБ", fs.avail / 1e6) } else { format!("{free_gb:.0} ГБ") };
        let size_s = if fs.size < 1e9 { format!("{:.0} МБ", fs.size / 1e6) } else { format!("{:.0} ГБ", fs.size / 1e9) };
        let c = Check::new("sys", &title, st, &format!("свободно {free_s} из {size_s} ({pct:.0}%)"));
        out.push(match st {
            OK => c,
            INFO => c.hint(
                "Этот диск почти заполнен. Если на нём хранятся архивы или копии — это нормально; если на него пишут программы — освободи место.",
            ),
            _ => c.hint("Места мало: программы перестают сохранять файлы, обновления не ставятся. Почисти корзину, загрузки, кэш."),
        });
    }
    out
}

pub(crate) fn check_system() -> Vec<Check> {
    let mut out = Vec::new();
    let cores = thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    // занятость процессора — по /proc/stat за секунду. «Средняя загрузка» (loadavg) считает и тех, кто ждёт
    // диск или сеть: при зависшей сетевой папке она огромная, а процессор свободен
    let l1: f64 = std::fs::read_to_string("/proc/loadavg").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(0.0);
    let threads = plural(cores, "поток", "потока", "потоков");
    match cpu_busy(Duration::from_secs(1)) {
        // «не хватает» — не секунда на 100%, а программы ждут процессор последнюю минуту: PSI (some avg60 —
        // доля времени, когда хоть кто-то ждал), без PSI — средняя загрузка за минуту выше числа потоков
        Some(pct) => out.push(Check::new("sys", "Процессор", if pct >= 85.0 && cpu_starved(l1, cores) { WARN } else { OK }, &format!("занят на {pct:.0}% ({threads})"))
            .hint(&if l1 > cores as f64 * 1.5 && pct < 50.0 {
                format!("Очередь большая (средняя загрузка {l1:.1}), а процессор свободен — программы ждут диск или сеть (зависшая сетевая папка, медленный диск).")
            } else {
                String::new()
            })),
        None => out.push(Check::new("sys", "Процессор", INFO, "не проверить").unknown().hint("Не читается /proc/stat.")),
    }
    if let Some((t, crit)) = hwmon_temp(&["k10temp", "coretemp", "zenpower"]) {
        // порог — от самого процессора (его критическая отметка); не знает — для AMD (Tctl, у новых Ryzen
        // 95 °C под нагрузкой — норма) 95/100, для прочих 85/95
        let (warn, bad) = match crit {
            Some(c) => (c - 10.0, c - 2.0),
            None if hwmon_temp(&["k10temp", "zenpower"]).is_some() => (95.0, 100.0),
            None => (85.0, 95.0),
        };
        out.push(Check::new(
            "sys",
            "Температура CPU",
            if t < warn {
                OK
            } else if t < bad {
                WARN
            } else {
                BAD
            },
            &format!("{t:.0}°C"),
        ));
    }
    let mut mem: HashMap<String, f64> = HashMap::new();
    for line in std::fs::read_to_string("/proc/meminfo").unwrap_or_default().lines() {
        if let Some((k, v)) = line.split_once(':') {
            if let Some(n) = v.split_whitespace().next().and_then(|x| x.parse::<f64>().ok()) {
                mem.insert(k.to_string(), n);
            }
        }
    }
    if let (Some(total), Some(avail)) = (mem.get("MemTotal"), mem.get("MemAvailable")) {
        let used = 1.0 - avail / total;
        out.push(Check::new(
            "sys",
            "Память",
            if used < 0.85 {
                OK
            } else if used < 0.95 {
                WARN
            } else {
                BAD
            },
            &format!("{:.0}% из {:.0} ГБ", used * 100.0, total / 1048576.0),
        ));
    }
    out.extend(disk_rows());
    let up: f64 = std::fs::read_to_string("/proc/uptime").ok().and_then(|s| s.split_whitespace().next().and_then(|x| x.parse().ok())).unwrap_or(0.0);
    let d = (up / 86400.0).floor() as i64;
    let hh = ((up % 86400.0) / 3600.0).floor() as i64;
    let mm = ((up % 3600.0) / 60.0).floor() as i64;
    out.push(
        Check::new(
            "sys",
            "Без перезагрузки",
            if d < 14 { INFO } else { WARN },
            &if d > 0 { format!("{d} д {hh} ч") } else { format!("{hh} ч {mm} мин") },
        )
        .hint(if d < 14 {
            ""
        } else {
            "Давно не перезагружалась. Иногда это помогает от странностей."
        }),
    );
    let failed_r = run_ex(&["systemctl", "--failed", "--no-legend", "--plain"], 6);
    // systemctl нет или не ответил — не проверить (а не «упавших нет»)
    if !failed_r.answered() {
        out.push(Check::new("sys", "Упавшие службы", INFO, "не проверить").unknown().hint("Нет программы systemctl или она не ответила."));
        return out;
    }
    let failed = failed_r.text().to_string();
    // проверка формы: имя службы — «имя.тип» (cups.service, home.mount); прочие строки (предупреждения,
    // «0 loaded units listed») — не службы
    let unit_types = [".service", ".mount", ".socket", ".timer", ".path", ".target", ".device", ".swap", ".scope", ".slice", ".automount"];
    let units: Vec<&str> = failed.lines().filter_map(|l| l.split_whitespace().find(|w| unit_types.iter().any(|t| w.ends_with(t)))).collect();
    // старые снимки snap-приложений часто «падают» безвредно, из-за них не тревожимся
    let is_harmless = |u: &&str| u.starts_with("snap-") && u.ends_with(".mount");
    let harmless = units.iter().filter(|u| is_harmless(u)).count();
    let bad_units: Vec<&str> = units.iter().copied().filter(|u| !is_harmless(u)).collect();
    if !bad_units.is_empty() {
        out.push(
            Check::new("sys", "Упавшие службы", WARN, &trunc(&bad_units.join(", "), 60))
                .hint(&format!("Подробности: systemctl status {}", bad_units[0])),
        );
    } else if harmless == 0 {
        out.push(Check::new("sys", "Упавшие службы", OK, "нет"));
    } else {
        out.push(
            Check::new("sys", "Упавшие службы", INFO, &format!("только старые snap ({harmless})"))
                .hint("Безвредно: это снимки старых версий snap-программ."),
        );
    }
    out
}

// ───────────────────────────── Всё вместе ─────────────────────────────

/// Проверки в том же порядке, что и в Python (CHECKS), только перенесённые сюда.
/// Запустить включённые показатели (catalog.rs) параллельно потоками; порядок — как в каталоге.
/// Строкам ставится категория показателя: по ней строка попадает на свою вкладку.
pub fn run_all() -> Vec<Check> {
    let started = Instant::now();
    let units: Vec<&'static crate::catalog::Unit> =
        crate::catalog::units().iter().filter(|u| crate::prefs::unit_enabled(u.id, u.default_on)).collect();
    log(&format!("запускаю проверки ({} из {}) параллельно", units.len(), crate::catalog::units().len()));
    let rxs: Vec<(&'static crate::catalog::Unit, Receiver<Vec<Check>>)> = units
        .into_iter()
        .map(|u| {
            let (tx, rx) = mpsc::channel();
            let f = u.run;
            thread::spawn(move || {
                let _ = tx.send(f());
            });
            (u, rx)
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut all = Vec::new();
    for (u, rx) in rxs {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut rows = match rx.recv_timeout(left) {
            Ok(r) => r,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                log(&format!("проверка {} не уложилась в 30 с", u.id));
                vec![Check::new("sys", u.title, WARN, "не отвечает").hint("Проверка не уложилась в 30 секунд.")]
            }
            // поток проверки упал (паника) — остальные проверки живы
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log(&format!("проверка {} упала", u.id));
                vec![Check::new("sys", u.title, INFO, "не получилось проверить").hint("Внутренняя ошибка проверки — подробности в журнале.")]
            }
        };
        for c in &mut rows {
            c.key = u.cat.key();
            c.unit = u.id;
        }
        all.extend(rows);
    }
    log(&format!("проверки завершены за {:.1} с, итог: {}", started.elapsed().as_secs_f64(), worst(&all)));
    all
}

/// Сколько строк «не удалось проверить» (с явной пометкой, а не по тексту).
pub fn unchecked_count(results: &[Check]) -> usize {
    results.iter().filter(|c| c.unchecked).count()
}

/// Худший статус из списка (bad > warn > ok > info); пустой список считается «ok».
/// Непроверенные строки на статус не влияют (они info): о них говорит `unchecked_count`.
pub fn worst(results: &[Check]) -> &str {
    let rank = |s: &str| match s {
        "bad" => 3,
        "warn" => 2,
        "ok" => 1,
        _ => 0,
    };
    let mut best: Option<&Check> = None;
    for r in results {
        if best.is_none_or(|b| rank(&r.status) > rank(&b.status)) {
            best = Some(r);
        }
    }
    best.map_or("ok", |b| b.status.as_str())
}

/// Режим `--diag`: один прогон и таблица в stdout.
pub fn print_table() {
    let all = run_all();
    // ширина на экране терминала: эмодзи (🗽 и т. п.) занимают две клетки, селекторы вариантов — ноль
    let cols = |s: &str| -> usize {
        s.chars()
            .map(|ch| match ch as u32 {
                0xFE00..=0xFE0F | 0x200D => 0,
                0x1F000..=0x1FAFF | 0x2600..=0x27BF => 2,
                _ => 1,
            })
            .sum()
    };
    // ширина — по уже переведённым названиям (по-английски они другой длины)
    let titles: Vec<String> = all.iter().map(|c| term_safe(&crate::i18n::tr(&c.title))).collect();
    let w = titles.iter().map(|t| cols(t)).max().unwrap_or(0);
    for (c, t) in all.iter().zip(&titles) {
        let pad = " ".repeat(w.saturating_sub(cols(t)));
        println!("{:<5} {}{}  {}", c.status, t, pad, term_safe(&crate::i18n::tr(&c.value)));
    }
    println!("\n{} {}", crate::i18n::pick("итог:", "total:"), worst(&all));
}

// ───────────────────────────── Сессии Claude Code ─────────────────────────────

/// Сессия Claude Code (поля как в словаре из claude_sessions в Python, плюс `path`).
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Session {
    pub id: String,
    pub cwd: String,
    /// Первое сообщение пользователя; у живой сессии без файла пустое.
    pub title: String,
    /// Время изменения файла сессии, секунды Unix.
    pub mtime: f64,
    pub live: bool,
    /// "busy" | "idle" | "" (из ~/.claude/sessions/*.json; у неживых пусто).
    pub status: String,
    /// Путь к файлу .jsonl (дополнение к Python; у живой сессии без файла пустой).
    pub path: String,
    /// Чей агент: "claude" или код из agents.rs ("codex", "qwen", "kimi", "gemini", "agy").
    pub agent: &'static str,
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Процесс жив? Без libc: смотрим /proc/<pid> (os.kill(pid, 0) в Python).
pub(crate) fn alive(pid: i64) -> bool {
    crate::procs::alive(pid)
}

fn mtime_of(p: &std::path::Path) -> Option<f64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs_f64(),
        Err(e) => -e.duration().as_secs_f64(),
    })
}

/// Как glob("dir/*<suffix>"): скрытые (с точки) имена пропускаются.
fn list_visible(dir: &str, suffix: &str) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !n.starts_with('.') && n.ends_with(suffix)
        })
        .map(|e| e.path())
        .collect()
}

static TITLE_CACHE: std::sync::Mutex<BTreeMap<String, String>> = std::sync::Mutex::new(BTreeMap::new());

/// Название сессии (первое сообщение): оно не меняется, поэтому запоминаем.
fn session_title(path: &str, limit: usize) -> String {
    if let Some(t) = lock(&TITLE_CACHE).get(path) {
        return t.clone();
    }
    let t = session_title_read(path, limit);
    // «без названия» не запоминаем: в только что начатой сессии первое сообщение ещё не записано
    if !t.is_empty() && !t.contains("без названия") {
        lock(&TITLE_CACHE).insert(path.to_string(), t.clone());
    }
    t
}

/// Убрать теги `<...>` (в Python это re.sub(r"<[^>]+>", " ", c)): пустой `<>` остаётся как есть.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('>') {
            Some(j) if j > 0 => {
                out.push(' ');
                rest = &after[j + 1..];
            }
            Some(_) => {
                out.push('<');
                rest = after;
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Первое сообщение пользователя в сессии, как короткое название.
fn session_title_read(path: &str, limit: usize) -> String {
    const NONE: &str = "(без названия)";
    let Ok(f) = crate::store::open_regular(std::path::Path::new(path)) else { return NONE.to_string() };
    let mut rd = std::io::BufReader::new(f);
    let mut buf = Vec::new();
    for i in 0.. {
        if i > 200 {
            break;
        }
        // строка не длиннее 1 МБ в памяти (остаток пропускается)
        if crate::agents::read_line_capped(&mut rd, &mut buf, 1 << 20) == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        // быстрый отсев — по слову «user» (без привязки к пробелам), решение — по разобранному JSON
        if !line.contains("user") {
            continue;
        }
        // битая строка — пропускаем и ищем дальше (раньше одна такая строка обрывала поиск названия)
        let Ok(j) = serde_json::from_str::<serde_json::Value>(line.trim_end_matches(['\n', '\r'])) else {
            continue;
        };
        if j.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        let c = match j.get("message").and_then(|m| m.get("content")) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(a)) => {
                a.iter().filter(|x| x.is_object()).map(|x| x.get("text").and_then(|t| t.as_str()).unwrap_or("")).collect::<Vec<_>>().join(" ")
            }
            _ => continue,
        };
        if c.contains("<command-name>") || c.contains("<local-command") || c.starts_with("[Request interrupted") {
            continue;
        }
        // в названии не должно оказаться ключа, вставленного в первую реплику
        let c = crate::footprint::mask_secrets(&strip_tags(&c).split_whitespace().collect::<Vec<_>>().join(" "));
        if !c.is_empty() && !c.starts_with("Caveat") {
            let n = c.chars().count();
            return if n > limit { format!("{}…", trunc(&c, limit)) } else { c };
        }
    }
    NONE.to_string()
}

/// Найденные папки по имени и когда искали (пустой результат через 10 минут ищем заново: папку могли создать).
static DIR_INDEX: std::sync::Mutex<BTreeMap<String, (Instant, Vec<String>)>> = std::sync::Mutex::new(BTreeMap::new());

/// Обход домашней папки как os.walk: до глубины 5, без скрытых и служебных папок, не дольше 3 с.
/// По ссылкам вглубь не идём; в подключённые диски (сетевые, FUSE, флешки внутри домашней папки) — тоже:
/// отвалившийся сетевой диск повесил бы обход. Возвращает все папки с именем `name`.
fn walk_find(home: &str, name: &str) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    #[cfg(unix)]
    let dev = |p: &std::path::Path| std::fs::symlink_metadata(p).ok().map(|m| std::os::unix::fs::MetadataExt::dev(&m));
    #[cfg(not(unix))]
    let dev = |_: &std::path::Path| Some(0u64);
    let home_dev = dev(std::path::Path::new(home));
    let mut found = Vec::new();
    let mut stack = vec![(std::path::PathBuf::from(home), 0usize)];
    while let Some((root, depth)) = stack.pop() {
        if Instant::now() > deadline {
            log(&format!("поиск папки «{name}»: не уложилась в 3 с, искала не везде"));
            break;
        }
        let Ok(rd) = std::fs::read_dir(&root) else { continue };
        let mut dirs = Vec::new();
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if depth >= 5 || n.starts_with('.') || matches!(n.as_str(), "snap" | "node_modules" | "__pycache__") {
                continue;
            }
            // тип — из самого списка папки (без лишнего обращения к файлу); ссылки не разворачиваем
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                dirs.push(n);
            }
        }
        if dirs.iter().any(|n| n == name) {
            found.push(root.join(name).to_string_lossy().to_string());
        }
        for n in dirs {
            let p = root.join(n);
            if dev(&p) == home_dev {
                stack.push((p, depth + 1));
            }
        }
    }
    found
}

/// Папку перенесли: ищем её в домашней папке по имени (последней части пути).
/// Если имя неоднозначное, сверяем и предыдущие части; из совпадений берём самую свежую.
fn find_moved_dir(old: &str) -> Option<String> {
    let parts: Vec<&str> = old.split('/').filter(|p| !p.is_empty()).collect();
    let name = *parts.last()?;
    let stale = lock(&DIR_INDEX)
        .get(name)
        .is_some_and(|(t, c)| !c.iter().all(|d| std::path::Path::new(d).is_dir()) || (c.is_empty() && t.elapsed() > Duration::from_secs(600)));
    if stale {
        lock(&DIR_INDEX).remove(name); // что-то переехало, пока аптечка была открыта: ищем заново
    }
    let cached = lock(&DIR_INDEX).get(name).map(|(_, c)| c.clone());
    let cands = match cached {
        Some(c) => c,
        None => {
            let t = Instant::now();
            let f = walk_find(&home(), name);
            log(&format!("поиск перенесённой папки «{name}»: найдено {}, {:.0} мс", f.len(), t.elapsed().as_secs_f64() * 1000.0));
            lock(&DIR_INDEX).insert(name.to_string(), (Instant::now(), f.clone()));
            f
        }
    };
    for k in (1..=parts.len()).rev() {
        // чем длиннее совпадает хвост пути, тем лучше; при одинаково подходящих нескольких (src, app, web —
        // таких папок десятки) не гадаем: лучше команда без cd, чем cd в чужой проект
        let tail = format!("/{}", parts.get(parts.len() - k..)?.join("/"));
        let hits: Vec<&String> = cands.iter().filter(|c| c.ends_with(&tail)).collect();
        match hits.len() {
            0 => continue,
            1 => return Some(hits[0].clone()),
            _ => return None,
        }
    }
    None
}

/// путь файла сессии → (сколько байт уже прочитано, последняя папка)
static CWD_CACHE: std::sync::Mutex<BTreeMap<String, (u64, Option<String>)>> = std::sync::Mutex::new(BTreeMap::new());

/// Рабочая папка сессии: последняя, где шла работа. Если её перенесли, найдём по имени.
/// Файл читается постепенно: при следующем опросе только дописанный хвост.
fn session_cwd(path: &str) -> String {
    use std::io::{Seek, SeekFrom};
    let h = home();
    let (mut done, mut last) = lock(&CWD_CACHE).get(path).cloned().unwrap_or((0, None));
    let mut read_tail = || -> std::io::Result<()> {
        let size = std::fs::metadata(path)?.len();
        if size < done {
            // файл перезаписали: читаем заново
            done = 0;
            last = None;
        }
        let mut f = crate::store::open_regular(std::path::Path::new(path))?;
        f.seek(SeekFrom::Start(done))?;
        // построчно, строка в памяти не длиннее 1 МБ (раньше хвост — при первом чтении весь файл — читался
        // в память целиком). Обрывок в конце (строку ещё дописывают) — на следующий раз; огромный обрывок
        // пропускаем насовсем, чтобы не перечитывать его каждую минуту
        const CAP: usize = 1 << 20;
        let mut rd = std::io::BufReader::new(f);
        let mut buf = Vec::new();
        let claude_dir = format!("{h}/.claude");
        loop {
            let got = crate::agents::read_line_capped(&mut rd, &mut buf, CAP);
            if got == 0 {
                break;
            }
            let whole = buf.last() == Some(&b'\n') || got > CAP;
            if !whole && got <= CAP {
                break;
            }
            done += got as u64;
            if got > CAP {
                continue;
            }
            let line = String::from_utf8_lossy(&buf);
            if !line.contains("\"cwd\"") {
                continue;
            }
            let Ok(j) = serde_json::from_str::<serde_json::Value>(&line) else { continue };
            // служебные папки Claude (память, черновики) не считаются рабочей папкой сессии
            if let Some(cwd) = j.get("cwd").and_then(|c| c.as_str()) {
                if !cwd.is_empty() && !cwd.starts_with(&claude_dir) && !cwd.starts_with("/tmp") {
                    last = Some(cwd.to_string());
                }
            }
        }
        lock(&CWD_CACHE).insert(path.to_string(), (done, last.clone()));
        Ok(())
    };
    let _ = read_tail();
    match last {
        None => h,
        Some(l) if std::path::Path::new(&l).is_dir() => l,
        Some(l) => find_moved_dir(&l).unwrap_or(h),
    }
}

/// Процесс Claude Code в терминале (не приложение Claude для рабочего стола и не его помощники).
fn looks_like_claude(p: &crate::procs::Proc) -> bool {
    let all = format!("{} {}", p.name, p.args.join(" "));
    p.is("claude") && !all.contains(".app/Contents/") && !all.contains("AnthropicClaude") && !all.contains("WindowsApps")
}

/// Номер процесса из файла сессии — из того же пространства номеров, что и мы (Linux: контейнеры и flatpak
/// живут в своём). Пустое или незнакомое значение — считаем своим.
fn same_pid_domain(d: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        if let (Some(i), Ok(ns)) = (d.find("pid:["), std::fs::read_link("/proc/self/ns/pid")) {
            return d[i..] == *ns.to_string_lossy();
        }
        !d.starts_with("darwin") && !d.starts_with("win")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = d;
        true
    }
}

/// Живые процессы Claude и последние `limit` сессий для --resume.
/// Живые сессии, чьих файлов нет среди последних, добавляются всё равно.
pub fn claude_sessions(limit: usize) -> Vec<Session> {
    let h = home();
    // sessionId → (cwd, status), в порядке обхода
    let mut live: Vec<(String, Option<String>, String)> = Vec::new();
    let procs = crate::procs::list();
    let mut live_pids: Vec<i64> = Vec::new();
    for f in list_visible(&format!("{h}/.claude/sessions"), ".json") {
        let Ok(txt) = crate::store::read_text(&f) else { continue };
        let Ok(j) = serde_json::from_str::<serde_json::Value>(&txt) else { continue };
        let pid = j.get("pid").and_then(|p| p.as_i64()).unwrap_or(-1);
        let Some(sid) = j.get("sessionId").and_then(|s| s.as_str()) else { continue };
        // номер процесса из другого «мира» (контейнер, flatpak) здесь ничего не значит — такие найдём по процессам ниже
        if !same_pid_domain(j.get("pidDomain").and_then(|d| d.as_str()).unwrap_or("")) {
            continue;
        }
        // номер мог достаться другой программе (Claude закрылся аварийно и не убрал файл)
        let reused = procs.iter().find(|p| p.pid == pid).is_some_and(|p| !looks_like_claude(p));
        if alive(pid) && !reused && !live.iter().any(|(s, _, _)| s == sid) {
            live_pids.push(pid);
            let cwd = j.get("cwd").and_then(|c| c.as_str()).filter(|c| !c.is_empty()).map(|c| c.to_string());
            let st = j.get("status").and_then(|c| c.as_str()).unwrap_or("").to_string();
            live.push((sid.to_string(), cwd, st));
        }
    }
    let mut files: Vec<(std::path::PathBuf, f64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(format!("{h}/.claude/projects")) {
        for d in rd.flatten() {
            let n = d.file_name();
            if n.to_string_lossy().starts_with('.') || !d.path().is_dir() {
                continue;
            }
            for p in list_visible(&d.path().to_string_lossy(), ".jsonl") {
                let m = mtime_of(&p).unwrap_or(0.0);
                files.push((p, m));
            }
        }
    }
    files.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Session> = Vec::new();
    for (p, m) in files.iter().take(limit) {
        let ps = p.to_string_lossy().to_string();
        let sid = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let info = live.iter().find(|(s, _, _)| *s == sid);
        out.push(Session {
            cwd: info.and_then(|i| i.1.clone()).unwrap_or_else(|| session_cwd(&ps)),
            title: session_title(&ps, 60),
            mtime: *m,
            live: info.is_some(),
            status: info.map(|i| i.2.clone()).unwrap_or_default(),
            path: ps,
            id: sid,
            agent: "claude",
        });
    }
    // процессы Claude, о которых нет записи в ~/.claude/sessions (старые версии, другой «мир» номеров):
    // живой — сессия из той же папки, а если папка процесса неизвестна (Windows) — самая свежая с его запуска
    // как у остальных агентов: только с терминалом (фоновые помощники Claude — не сессия человека)
    let claudes = crate::procs::without_wrappers(procs.iter().filter(|p| looks_like_claude(p) && p.tty != Some(false)).cloned().collect());
    // обёртка, запустившая Claude из записи сессий, — тоже он
    let claudes: Vec<_> =
        claudes.into_iter().filter(|p| !live_pids.contains(&p.pid) && !procs.iter().any(|c| c.ppid == p.pid && live_pids.contains(&c.pid))).collect();
    for p in &claudes {
        let fresh = |s: &Session| p.start <= 0.0 || s.mtime >= p.start - 120.0;
        let idx = if !p.cwd.is_empty() { out.iter().position(|s| !s.live && s.cwd == p.cwd) } else { out.iter().position(|s| !s.live && fresh(s)) };
        if let Some(i) = idx {
            out[i].live = true;
        }
    }
    for (sid, cwd, st) in &live {
        if !out.iter().any(|s| s.id == *sid) {
            out.insert(
                0,
                Session {
                    id: sid.clone(),
                    cwd: cwd.clone().unwrap_or_else(home),
                    title: String::new(),
                    mtime: now_secs(),
                    live: true,
                    status: st.clone(),
                    path: String::new(),
                    agent: "claude",
                },
            );
        }
    }
    // живые сверху, внутри групп свежие первыми (сортировка устойчивая, как в Python)
    out.sort_by(|a, b| (!a.live).cmp(&(!b.live)).then(b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal)));
    out
}

/// Как shlex.quote: безопасные символы (ASCII \w и @%+=:,./-) без кавычек, иначе в '...'.
pub(crate) fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Команда для продолжения сессии. Путь в кавычках (в нём бывают пробелы: «Рабочий стол»);
/// если папки уже нет, то без cd: claude --resume сам найдёт сессию в любом проекте.
pub fn resume_cmd(s: &Session) -> String {
    if s.agent != "claude" {
        let a = crate::agents::AgentSession { agent: s.agent, id: s.id.clone(), cwd: s.cwd.clone(), ..Default::default() };
        return crate::agents::resume_command(&a);
    }
    if crate::agents::safe_id(&s.id).is_none() {
        return String::new();
    }
    if !s.cwd.is_empty() && std::path::Path::new(&s.cwd).is_dir() {
        if cfg!(windows) {
            // терминал Windows по умолчанию — PowerShell: там нет && (в версии 5) и кавычки свои
            return format!("Set-Location -LiteralPath {}; claude --resume {}", crate::agents::ps_quote(&s.cwd), s.id);
        }
        format!("cd {} && claude --resume {}", shell_quote(&s.cwd), s.id)
    } else {
        format!("claude --resume {}", s.id)
    }
}

/// Режим `--sessions`: список сессий и команды продолжения.
pub fn print_sessions() {
    let t = Instant::now();
    let all = claude_sessions(8);
    log(&format!("сессий: {}, за {:.0} мс", all.len(), t.elapsed().as_secs_f64() * 1000.0));
    let now = now_secs();
    for s in &all {
        let state = if s.live {
            if s.status.is_empty() {
                "live"
            } else {
                s.status.as_str()
            }
        } else {
            "-"
        };
        // всё, что пришло из чужих файлов, — без управляющих знаков терминала
        println!("{:<5} {:>8.0}{}  {}", state, (now - s.mtime).max(0.0), crate::i18n::pick("с назад", "s ago"), term_safe(&s.id));
        println!("      {} {}", crate::i18n::pick("название:", "title:   "), term_safe(&crate::footprint::mask_secrets(&s.title)));
        println!("      {} {}", crate::i18n::pick("папка:   ", "folder:  "), term_safe(&s.cwd));
        println!("      {} {}", crate::i18n::pick("файл:    ", "file:    "), term_safe(&s.path));
        println!("      {} {}", crate::i18n::pick("команда: ", "command: "), term_safe(&resume_cmd(s)));
    }
}

// ───────────────────────────── Тест скорости ─────────────────────────────

/// Заголовки строк результата (как SPEED_TITLES в Python).
#[allow(dead_code)]
pub const SPEED_TITLES: [&str; 3] = ["Пинг", "Скорость ↓ скачивание", "Скорость ↑ отдача"];
pub const SPEED_HOST: &str = "speed.cloudflare.com";

/// Этап теста скорости.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeedPhase {
    Ping,
    Down,
    Up,
}

/// Ход теста для живого прибора (в Python это progress(phase, value)).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub struct SpeedProgress {
    pub phase: SpeedPhase,
    /// Текущая скорость, Мбит/с (на этапе пинга 0).
    pub mbps: f64,
    /// Задержка, мс: только на этапе пинга, когда она измерена.
    pub ping_ms: Option<f64>,
    /// Доля текущего этапа 0..1: больший из долей по байтам и по времени.
    pub fraction: f64,
}

/// Адрес сервера замера; в тестах подменяется локальным.
fn speed_base() -> String {
    #[cfg(test)]
    if let Some(b) = TEST_BASE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return b;
    }
    format!("https://{SPEED_HOST}")
}
#[cfg(test)]
static TEST_BASE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `timeout_s` — на каждое чтение, `overall_s` — на весь запрос целиком (с заголовками и телом): иначе сервер,
/// отдающий по байту раз в несколько секунд (slow-loris), держал бы запрос сколько угодно.
fn speed_agent(timeout_s: u64, overall_s: f64) -> ureq::Agent {
    // прокси из окружения ureq не берёт: как ProxyHandler({}) в Python
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs_f64(overall_s))
        .timeout_connect(Duration::from_secs(timeout_s))
        .timeout_read(Duration::from_secs(timeout_s))
        .timeout_write(Duration::from_secs(timeout_s))
        .user_agent(concat!("aptechka/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Один пустой запрос по общему соединению; код ответа http не важен, как в http.client.
fn ping_once(agent: &ureq::Agent) -> Result<(), String> {
    let url = format!("{}/__down?bytes=0", speed_base());
    let resp = match agent.get(&url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => return Err(e.to_string()),
    };
    let mut sink = Vec::new();
    // ответ должен быть пустым: читаем не больше 64 КБ (чужой сервер не заставит качать гигабайты)
    resp.into_reader().take(64 << 10).read_to_end(&mut sink).map_err(|e| e.to_string())?;
    Ok(())
}

/// Задержка до настоящего сервера: время ответа на крошечный запрос по уже открытому
/// соединению. TCP-рукопожатие не годится: роутер с VPN отвечает на него сам за 0 мс.
/// Возвращает (медиана, джиттер) в мс или None, если удачных замеров меньше трёх.
fn ping_stats(n: usize) -> Option<(f64, f64)> {
    let agent = speed_agent(4, 10.0);
    let mut times: Vec<f64> = Vec::new();
    let mut run_all_pings = || -> Result<(), String> {
        // прогрев: рукопожатие и TLS. Первое соединение иногда не успевает открыться, когда все проверки
        // стартуют разом (свежая система, медленная сеть) — одна повторная попытка, прежде чем «не измерить»
        if ping_once(&agent).is_err() {
            thread::sleep(Duration::from_millis(500));
            ping_once(&agent)?;
        }
        for _ in 0..n {
            let t = Instant::now();
            ping_once(&agent)?;
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        Ok(())
    };
    if let Err(e) = run_all_pings() {
        log(&format!("пинг: {}", trunc(&e, 80)));
    }
    if times.len() < 3 {
        return None;
    }
    let mut s = times.clone();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = *s.get(s.len() / 2)?;
    let jit = times.windows(2).map(|w| (w[0] - w[1]).abs()).sum::<f64>() / (times.len() - 1) as f64;
    Some((med, jit))
}

fn mbps_of(bytes: u64, secs: f64) -> f64 {
    bytes as f64 * 8.0 / secs / 1e6
}

/// Замер в несколько потоков, как у speedtest: один поток TCP (особенно через VPN с большим пингом) не
/// заполняет быстрый канал. Каждый поток раз за разом качает (или отдаёт) кусок `chunk`, пока не выйдет время
/// или общий объём `cap`. Скорость — по установившемуся участку: первые ~1,5 с (рукопожатие, TLS, разгон
/// TCP) не считаются, иначе на коротком замере выходит вдвое меньше настоящей.
#[allow(clippy::too_many_arguments)]
/// Ответ сервера замера «слишком много запросов» (HTTP 429): замеров подряд было много, он временно ограничил адрес.
const BUSY: &str = "сервер замера просит подождать";

fn transfer(
    progress: &mut dyn FnMut(SpeedProgress),
    down: bool,
    streams: usize,
    limit_s: f64,
    cap: u64,
    chunk: u64,
    early: bool,
) -> Result<Option<f64>, String> {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    let total = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let block = Arc::new(if down { Vec::new() } else { random_block(256 * 1024) });
    let t0 = Instant::now();
    let mut workers = Vec::new();
    for _ in 0..streams.max(1) {
        let (total, stop, err, block) = (total.clone(), stop.clone(), err.clone(), block.clone());
        workers.push(thread::spawn(move || {
            let agent = speed_agent(8, limit_s + 3.0);
            // сбой соединения (медленная или неровная сеть, VPN) — ещё попытка, пока есть время: иначе одна
            // неудача в начале выбивала поток до конца замера, а неудачи у всех — «не измерить»
            let mut fails = 0;
            while !stop.load(Ordering::Relaxed) && total.load(Ordering::Relaxed) < cap {
                let r = if down { pull(&agent, chunk, &total, &stop) } else { push(&agent, chunk, &block, &total, &stop) };
                if let Err(e) = r {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    // сервер просит подождать — останавливаем все потоки сразу, не долбим его дальше
                    if e == BUSY {
                        err.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(e);
                        stop.store(true, Ordering::Relaxed);
                        break;
                    }
                    fails += 1;
                    if fails >= 3 || t0.elapsed().as_secs_f64() > limit_s - 1.0 {
                        err.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(e);
                        break;
                    }
                    thread::sleep(Duration::from_millis(300));
                }
            }
        }));
    }
    // главный поток: прогресс для окна и точки для расчёта установившейся скорости
    let mut samples: Vec<(f64, u64)> = vec![(0.0, 0)];
    loop {
        thread::sleep(Duration::from_millis(100));
        let el = t0.elapsed().as_secs_f64();
        let got = total.load(Ordering::Relaxed);
        samples.push((el, got));
        // текущая скорость — за последнюю секунду
        let back = samples.iter().rev().find(|(t, _)| el - t >= 1.0).copied().unwrap_or(samples[0]);
        let now_mbps = mbps_of(got.saturating_sub(back.1), (el - back.0).max(0.1));
        progress(SpeedProgress {
            phase: if down { SpeedPhase::Down } else { SpeedPhase::Up },
            mbps: now_mbps,
            ping_ms: None,
            fraction: (got as f64 / cap as f64).max(el / limit_s).min(1.0),
        });
        let all_done = workers.iter().all(|w| w.is_finished());
        // скорость уже установилась — дальше качать незачем (обычная сеть; на лимитной и так мало)
        let settled = early && early_stop_rate(&samples).is_some();
        if el > limit_s || got >= cap || all_done || settled {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    // ждём потоки недолго: застрявший (сервер тянет заголовки по байту) не должен держать замер. Его запрос
    // оборвётся сам по общему сроку; потоки владеют своими Arc, так что отпустить их безопасно
    let grace = Instant::now() + Duration::from_secs(2);
    while Instant::now() < grace && !workers.iter().all(|w| w.is_finished()) {
        thread::sleep(Duration::from_millis(20));
    }
    drop(workers);
    let end_b = samples.last().map_or(0, |s| s.1);
    if end_b <= 500_000 {
        return match err.lock().unwrap_or_else(|e| e.into_inner()).take() {
            Some(e) => Err(e),
            None => Ok(None),
        };
    }
    let steady = steady_rate(&samples);
    Ok(Some(steady))
}

/// Установившаяся скорость, Мбит/с: от точки, где пришли первые 10% данных, до конца ряда `(время, байты)`.
/// До этой точки — соединения, TLS и разгон TCP (на Windows открытие шести соединений занимает до 3 с).
fn steady_rate(samples: &[(f64, u64)]) -> f64 {
    let (end_t, end_b) = *samples.last().unwrap_or(&(0.0, 0));
    let (w_t, w_b) = samples.iter().copied().find(|(_, b)| *b >= end_b / 10).unwrap_or((0.0, 0));
    if end_t - w_t >= 0.5 && end_b > w_b {
        mbps_of(end_b - w_b, end_t - w_t)
    } else {
        mbps_of(end_b, end_t.max(0.01))
    }
}

/// Раньше срока замер не останавливаем, пока не прошло столько секунд.
const EARLY_MIN_S: f64 = 3.0;
/// Ширина окна для сравнения скоростей, секунд, и сколько таких окон подряд должны совпасть.
const EARLY_WINDOW_S: f64 = 1.0;
const EARLY_WINDOWS: usize = 3;
/// Допустимое расхождение окон со средним (±5%).
const EARLY_TOLERANCE: f64 = 0.05;
/// Медленную линию (меньше этого, Мбит/с) гоним до конца срока: трафика на ней мало, а замер шумнее.
const EARLY_MIN_MBPS: f64 = 20.0;

/// Пора ли остановить замер: возвращает установившуюся скорость, если она уже устоялась, иначе None.
/// Правило: прошло хотя бы 3 с; последние три окна по 1 с целиком лежат после точки разгона (первые 10%
/// данных); скорость в каждом окне отличается от средней не больше чем на 5%; и скорость не меньше
/// 20 Мбит/с. Три окна по ±5% отсекают разгон TCP (он меняет скорость на десятки процентов за секунду) и
/// неровную сеть, но не требуют идеально ровной линии — у шести потоков в сумме разброс обычно 1–3%.
fn early_stop_rate(samples: &[(f64, u64)]) -> Option<f64> {
    let &(end_t, end_b) = samples.last()?;
    if end_t < EARLY_MIN_S || end_b <= 500_000 {
        return None;
    }
    let (w_t, _) = samples.iter().copied().find(|(_, b)| *b >= end_b / 10)?;
    let first = end_t - EARLY_WINDOW_S * EARLY_WINDOWS as f64;
    if first < w_t {
        return None;
    }
    // байты к моменту t: последняя точка не позже t
    let at = |t: f64| samples.iter().rev().find(|(st, _)| *st <= t + 1e-9).map_or(0, |s| s.1);
    let rates: Vec<f64> = (0..EARLY_WINDOWS)
        .map(|i| {
            let (a, b) = (first + i as f64 * EARLY_WINDOW_S, first + (i + 1) as f64 * EARLY_WINDOW_S);
            mbps_of(at(b).saturating_sub(at(a)), EARLY_WINDOW_S)
        })
        .collect();
    let mean = rates.iter().sum::<f64>() / rates.len() as f64;
    if mean < EARLY_MIN_MBPS || rates.iter().any(|r| (r - mean).abs() > mean * EARLY_TOLERANCE) {
        return None;
    }
    Some(steady_rate(samples))
}

/// Один запрос скачивания: читает тело и сразу прибавляет к общему счёту.
fn pull(agent: &ureq::Agent, chunk: u64, total: &std::sync::atomic::AtomicU64, stop: &std::sync::atomic::AtomicBool) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let resp = match agent.get(&format!("{}/__down?bytes={chunk}", speed_base())).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(429, _)) => return Err(BUSY.into()),
        Err(e) => return Err(e.to_string()),
    };
    let mut r = resp.into_reader().take(chunk);
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        got += n as u64;
        total.fetch_add(n as u64, Ordering::Relaxed);
    }
    // пустой ответ 200 — не «скачала», а сбой: иначе поток крутил бы запросы вхолостую (счётчик сбоев в transfer)
    if got == 0 && !stop.load(Ordering::Relaxed) {
        return Err("пустой ответ сервера замера".into());
    }
    Ok(())
}

/// Тело для отдачи: блоки случайных байт; обрывается, когда замер закончен.
struct UpBody<'a> {
    block: &'a [u8],
    left: u64,
    total: &'a std::sync::atomic::AtomicU64,
    stop: &'a std::sync::atomic::AtomicBool,
    /// сколько этот запрос уже прибавил к `total` (при отказе сервера вычитается обратно)
    sent: &'a std::cell::Cell<u64>,
}

impl Read for UpBody<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        use std::sync::atomic::Ordering;
        if self.left == 0 {
            return Ok(0);
        }
        if self.stop.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("замер закончен"));
        }
        let n = out.len().min(self.block.len()).min(self.left as usize);
        out[..n].copy_from_slice(&self.block[..n]);
        self.left -= n as u64;
        self.total.fetch_add(n as u64, Ordering::Relaxed);
        self.sent.set(self.sent.get() + n as u64);
        Ok(n)
    }
}

/// Один запрос отдачи куска `chunk`.
fn push(
    agent: &ureq::Agent,
    chunk: u64,
    block: &[u8],
    total: &std::sync::atomic::AtomicU64,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<(), String> {
    let sent = std::cell::Cell::new(0u64);
    let body = UpBody { block, left: chunk, total, stop, sent: &sent };
    let res = match agent
        .post(&format!("{}/__up", speed_base()))
        .set("Content-Type", "application/octet-stream")
        .set("Content-Length", &chunk.to_string())
        .send(body)
    {
        // 429 — сервер просит остановиться: это не «отдала», иначе потоки тут же слали бы ещё
        Err(ureq::Error::Status(429, _)) => Err(BUSY.into()),
        // успех — только 2xx: заглушка captive portal, 403, 502 — не отдача
        Ok(r) if (200..300).contains(&r.status()) => Ok(()),
        Ok(r) => Err(format!("сервер замера ответил {}", r.status())),
        Err(ureq::Error::Status(code, _)) => Err(format!("сервер замера ответил {code}")),
        Err(e) => Err(e.to_string()),
    };
    // отказ или обрыв: эти байты сервер не принял (они лишь ушли в буфер сокета) — в скорость не идут
    if res.is_err() && !stop.load(std::sync::atomic::Ordering::Relaxed) {
        total.fetch_sub(sent.get(), std::sync::atomic::Ordering::Relaxed);
    }
    res
}

/// Источник случайных байт для отдачи: /dev/urandom, а если его нет, простой xorshift.
fn random_block(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut b).is_ok() {
            return b;
        }
    }
    let mut x = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
    for v in b.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *v = (x >> 24) as u8;
    }
    b
}

/// Пинг, скачивание и отдача через Cloudflare: около 20 секунд, в 6 потоков (на лимитной сети — один поток
/// и несколько мегабайт).
pub fn speed_test(mut progress: impl FnMut(SpeedProgress)) -> Vec<Check> {
    let mut out = Vec::new();
    let ping = |p: &mut dyn FnMut(SpeedProgress), ms: Option<f64>| {
        p(SpeedProgress { phase: SpeedPhase::Ping, mbps: 0.0, ping_ms: ms, fraction: if ms.is_some() { 1.0 } else { 0.0 } })
    };
    log("тест скорости: пинг");
    ping(&mut progress, None);
    // лимитная сеть (мобильный интернет, раздача с телефона): качаем мало — замер не съест трафик
    let metered = is_metered();
    let ping_r = ping_stats(8);
    let mut rec = SpeedRec { ping: ping_r.map(|p| p.0), ..Default::default() };
    match ping_r {
        // сервер замера молчит — это ещё не «нет интернета»: спросим другие адреса
        None if internet_up() => out.push(
            Check::new("net", SPEED_TITLES[0], WARN, "сервер замера не ответил")
                .hint("Интернет есть, но сервер замера скорости недоступен (бывает через VPN или в рабочей сети). Попробуй позже."),
        ),
        None => out.push(
            Check::new("net", SPEED_TITLES[0], BAD, "нет ответа").hint("Ни сервер замера, ни другие адреса не отвечают: похоже, интернета нет."),
        ),
        Some((med, jit)) => {
            let st = if med < 250.0 && jit < 60.0 {
                OK
            } else if med < 700.0 {
                WARN
            } else {
                BAD
            };
            let mut c = Check::new("net", SPEED_TITLES[0], st, &format!("{med:.0} мс · джиттер {jit:.0} мс"));
            if st != OK {
                c = c.hint("Большая задержка: видео и звонки будут подлагивать. Часто так бывает через дальний VPN-сервер.");
            }
            out.push(c);
            ping(&mut progress, Some(med));
        }
    }
    for (title, is_down, good) in [(SPEED_TITLES[1], true, 20.0), (SPEED_TITLES[2], false, 5.0)] {
        log(&format!("тест скорости: {}", if is_down { "скачивание" } else { "отдача" }));
        // обычная сеть — 6 потоков по времени (до 10 с скачивание, 8 с отдача, но раньше, если скорость устоялась; объём с запасом на гигабитную
        // линию); лимитная — один поток и совсем немного
        let res = match (is_down, metered) {
            (true, false) => transfer(&mut progress, true, 6, 10.0, 600_000_000, 50_000_000, true),
            (false, false) => transfer(&mut progress, false, 6, 8.0, 300_000_000, 20_000_000, true),
            (true, true) => transfer(&mut progress, true, 1, 10.0, 5_000_000, 5_000_000, false),
            (false, true) => transfer(&mut progress, false, 1, 8.0, 2_000_000, 2_000_000, false),
        };
        let (mbps, err) = match res {
            Ok(m) => (m, String::new()),
            Err(e) => (None, trunc(&e, 60)),
        };
        match mbps {
            // ограничение сервера — не поломка сети: «не проверено», с понятной подсказкой
            None if err == BUSY => out.push(
                Check::new("net", title, INFO, BUSY)
                    .hint("Замеров подряд было много, и сервер замера скорости временно ограничил твой адрес. Попробуй через несколько минут.")
                    .unknown(),
            ),
            None => {
                let hint = if err.is_empty() { "Сервер замера недоступен." } else { err.as_str() };
                out.push(Check::new("net", title, BAD, "не измерить").hint(hint));
            }
            Some(m) => {
                let st = if m >= good {
                    OK
                } else if m >= good / 4.0 {
                    WARN
                } else {
                    BAD
                };
                let mut c = Check::new("net", title, st, &format!("{m:.0} Мбит/с"));
                // сравнение с обычной скоростью этой сети (история замеров)
                let usual = speed_usual(is_down);
                if let Some(u) = usual.filter(|u| *u > 0.0) {
                    let d = (m - u) / u * 100.0;
                    if d <= -35.0 {
                        c = c.hint(&format!("На {:.0}% ниже обычного для этой сети (обычно {u:.0} Мбит/с). Проверь, не качает ли что-то в фоне, или смени VPN-сервер.", -d));
                        if st == OK {
                            c.status = WARN.into();
                        }
                    } else if st != OK {
                        c = c.hint(&format!("Для этой сети это обычная скорость (обычно {u:.0} Мбит/с)."));
                    }
                } else if st != OK {
                    c = c.hint("Медленно. Попробуй другой VPN-сервер или проверь, не качает ли что-то в фоне.");
                }
                if metered {
                    c.detail = format!(
                        "{}{}Сеть лимитная — мерила понемногу, чтобы не тратить трафик.",
                        c.detail,
                        if c.detail.is_empty() { "" } else { " " }
                    );
                }
                if is_down {
                    rec.down = Some(m)
                } else {
                    rec.up = Some(m)
                }
                out.push(c);
            }
        }
    }
    speed_remember(&rec);
    log("тест скорости завершён");
    out
}

/// Есть ли интернет вообще (TCP до двух независимых адресов) — чтобы «сервер замера молчит» не выдавать
/// за «интернета нет».
fn internet_up() -> bool {
    crate::reach::internet().is_some() || proxy_internet().is_some()
}

/// Лимитная ли сеть (мобильный интернет): Linux — NetworkManager (Metered: 1 да, 3 «скорее да»).
/// Windows и macOS пока не знаем — false.
pub(crate) fn is_metered() -> bool {
    #[cfg(target_os = "linux")]
    {
        let (code, v) = run(
            &[
                "busctl",
                "get-property",
                "org.freedesktop.NetworkManager",
                "/org/freedesktop/NetworkManager",
                "org.freedesktop.NetworkManager",
                "Metered",
            ],
            4,
        );
        return code == 0 && matches!(v.split_whitespace().nth(1), Some("1") | Some("3"));
    }
    #[allow(unreachable_code)]
    false
}

/// Запись замера: сеть (отпечаток адреса), пинг, скорость вниз/вверх.
#[derive(Default)]
struct SpeedRec {
    ping: Option<f64>,
    down: Option<f64>,
    up: Option<f64>,
}

/// История замеров текущего сеанса: только в памяти процесса (на диск не пишется), последние 20.
/// (сеть, ping, вниз, вверх)
static SPEED_HIST: std::sync::Mutex<std::collections::VecDeque<(String, SpeedRec)>> = std::sync::Mutex::new(std::collections::VecDeque::new());
const SPEED_HIST_MAX: usize = 20;

/// Отпечаток сети для сравнения замеров: адрес, с которого выходим в интернет (без отправки пакетов). Только у себя,
/// никуда не уходит; сами адреса не храним — только хэш.
fn net_tag() -> String {
    let ip = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("1.1.1.1:53")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_default();
    // подсеть /24 (адрес внутри одной сети меняется) и роутер: у многих домашних сетей одинаковая
    // 192.168.1.x, а роутер (его адрес и сетевой интерфейс) отличает дом от кафе
    let net = format!("{}|{}", ip.rsplit_once('.').map_or(ip.clone(), |(a, _)| a.to_string()), default_gateway().unwrap_or_default());
    let mut h: u64 = 0xcbf29ce484222325;
    for b in net.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", h as u32)
}

/// Роутер по умолчанию: «интерфейс адрес» (Linux, /proc/net/route; на других системах — None).
fn default_gateway() -> Option<String> {
    let t = std::fs::read_to_string("/proc/net/route").ok()?;
    t.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        // Destination 00000000 — маршрут по умолчанию; Gateway — в обратном порядке байт (hex)
        if f.len() < 3 || f[1] != "00000000" {
            return None;
        }
        let g = u32::from_str_radix(f[2], 16).ok()?.to_le_bytes();
        Some(format!("{} {}.{}.{}.{}", f[0], g[0], g[1], g[2], g[3]))
    })
}

/// Запомнить замер в памяти (для сравнения «ниже обычного» в этом же сеансе).
fn speed_remember(r: &SpeedRec) {
    if r.down.is_none() && r.up.is_none() {
        return;
    }
    let mut h = SPEED_HIST.lock().unwrap_or_else(|e| e.into_inner());
    if h.len() >= SPEED_HIST_MAX {
        h.pop_front();
    }
    h.push_back((net_tag(), SpeedRec { ping: r.ping, down: r.down, up: r.up }));
}

/// Обычная скорость этой сети в этом сеансе: медиана прошлых замеров (нужно хотя бы 3).
fn speed_usual(down: bool) -> Option<f64> {
    let tag = net_tag();
    let h = SPEED_HIST.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<f64> = h.iter().filter(|(n, _)| *n == tag).filter_map(|(_, r)| if down { r.down } else { r.up }).collect();
    drop(h);
    if v.len() < 3 {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

/// Задержка сети — лёгкий автоматический замер для расписания (несколько маленьких соединений, без скачивания).
pub(crate) fn check_latency() -> Vec<Check> {
    match ping_stats(5) {
        Some((med, jit)) => {
            let st = if med < 250.0 && jit < 60.0 {
                OK
            } else if med < 700.0 {
                WARN
            } else {
                BAD
            };
            let c = Check::new("net", "Задержка сети", st, &format!("{med:.0} мс · джиттер {jit:.0} мс"));
            vec![if st == OK {
                c
            } else {
                c.hint("Большая задержка: видео и звонки будут подлагивать. Часто так бывает через дальний VPN-сервер.")
            }]
        }
        // не ответил — значит, не проверено (в итоге «не всё проверено», а не зелёное «всё в порядке»)
        None => vec![Check::new("net", "Задержка сети", INFO, "не измерить").hint("Сервер замера не ответил.").unknown()],
    }
}

/// Режим `--speed`: тест с живым прогрессом в stderr и таблицей результатов в stdout.
pub fn print_speed() {
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut last_phase: Option<SpeedPhase> = None;
    let out = speed_test(|p| {
        let changed = last_phase != Some(p.phase);
        if !changed && last.elapsed() < Duration::from_millis(250) {
            return;
        }
        last = Instant::now();
        last_phase = Some(p.phase);
        let name = match p.phase {
            SpeedPhase::Ping => "пинг",
            SpeedPhase::Down => "скачивание",
            SpeedPhase::Up => "отдача",
        };
        let val = match (p.phase, p.ping_ms) {
            (SpeedPhase::Ping, Some(ms)) => format!("{ms:.0} мс"),
            (SpeedPhase::Ping, None) => "…".to_string(),
            _ => format!("{:.1} Мбит/с", p.mbps),
        };
        eprintln!("[диагностика] {name:<10} {val:<14} {:>3.0}%", p.fraction * 100.0);
    });
    let titles: Vec<String> = out.iter().map(|c| term_safe(&crate::i18n::tr(&c.title))).collect();
    let w = titles.iter().map(|t| t.chars().count()).max().unwrap_or(0);
    for (c, t) in out.iter().zip(&titles) {
        let pad = " ".repeat(w.saturating_sub(t.chars().count()));
        println!("{:<5} {}{}  {}", c.status, t, pad, term_safe(&crate::i18n::tr(&c.value)));
        if !c.detail.is_empty() {
            println!("      {}", term_safe(&crate::i18n::tr(&c.detail)));
        }
    }
}

/// Сессии всех агентов в терминале: Claude и остальные (agents.rs); живые сверху, потом свежие.
pub fn all_sessions(limit: usize) -> Vec<Session> {
    let mut v = claude_sessions(limit);
    for a in crate::agents::all_sessions(limit) {
        v.push(Session {
            id: a.id,
            cwd: a.cwd,
            title: crate::footprint::mask_secrets(&a.title),
            mtime: a.mtime,
            live: a.live,
            status: match (a.live, a.busy) {
                (true, Some(true)) => "busy".into(),
                (true, _) => "idle".into(),
                _ => String::new(),
            },
            path: a.path.to_string_lossy().to_string(),
            agent: a.agent,
        });
    }
    v.sort_by(|a, b| b.live.cmp(&a.live).then(b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal)));
    v.truncate(limit);
    v
}

/// Имя агента для подписи.
pub fn agent_name(id: &str) -> &'static str {
    if id == "claude" {
        "Claude"
    } else {
        crate::agents::find(id).map(|a| a.name).unwrap_or("ИИ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ряд (время, байты) раз в 100 мс для скорости `mbps(t)`, длиной `dur` секунд.
    fn series(dur: f64, mbps: impl Fn(f64) -> f64) -> Vec<(f64, u64)> {
        let mut v = vec![(0.0, 0u64)];
        let (mut t, mut b) = (0.0, 0.0f64);
        while t < dur - 1e-9 {
            t += 0.1;
            b += mbps(t) * 1e6 / 8.0 * 0.1;
            v.push((t, b as u64));
        }
        v
    }

    /// То, что делает transfer: по ряду шаг за шагом; (когда остановились, скорость, байт к остановке).
    fn simulate(full: &[(f64, u64)]) -> (f64, f64, u64) {
        for n in 2..=full.len() {
            if let Some(r) = early_stop_rate(&full[..n]) {
                return (full[n - 1].0, r, full[n - 1].1);
            }
        }
        (full.last().unwrap().0, steady_rate(full), full.last().unwrap().1)
    }

    #[test]
    fn early_stop_stable_line() {
        let (t, r, _) = simulate(&series(10.0, |_| 300.0));
        assert!((3.0..=4.0).contains(&t), "остановились в {t}");
        assert!((r - 300.0).abs() < 3.0, "скорость {r}");
    }

    #[test]
    fn early_stop_after_ramp_plateau() {
        // 2 с разгона до 300, дальше ровно
        let (t, r, _) = simulate(&series(10.0, |t| if t < 2.0 { 150.0 * t } else { 300.0 }));
        assert!(t >= 4.0, "остановились до плато: {t}");
        assert!(t <= 6.0, "слишком поздно: {t}");
        assert!(r > 250.0 && r <= 300.0, "скорость {r}");
    }

    #[test]
    fn early_stop_noisy_runs_to_limit() {
        let (t, _, _) = simulate(&series(10.0, |t| 200.0 + 60.0 * (t * 2.1).sin()));
        assert!(t >= 9.9, "остановились в {t}");
        // постоянный сползающий ряд тоже не «устоялся»
        let (t, _, _) = simulate(&series(10.0, |t| 400.0 - 20.0 * t));
        assert!(t >= 9.9, "остановились в {t}");
    }

    #[test]
    fn early_stop_slow_line_keeps_old_behaviour() {
        let (t, r, _) = simulate(&series(10.0, |_| 8.0));
        assert!(t >= 9.9, "остановились в {t}");
        assert!((r - 8.0).abs() < 0.3);
        assert!(early_stop_rate(&series(2.5, |_| 300.0)).is_none(), "раньше 3 с нельзя");
        assert!(early_stop_rate(&[(0.0, 0)]).is_none());
    }

    #[test]
    fn early_stop_saves_data_at_300_mbit() {
        let (_, _, b) = simulate(&series(10.0, |_| 300.0));
        let full = series(10.0, |_| 300.0).last().unwrap().1;
        eprintln!("стоп: {b} байт из {full}");
        assert!(b * 100 < full * 45, "сэкономили мало");
    }

    #[test]
    fn title_survives_spacing_and_broken_lines() {
        let title = |body: &str| {
            let p = std::env::temp_dir().join(format!("aptechka-title-{}-{}", std::process::id(), body.len()));
            std::fs::write(&p, body).unwrap();
            let t = session_title_read(p.to_str().unwrap(), 80);
            let _ = std::fs::remove_file(&p);
            t
        };
        assert_eq!(title("{\"type\": \"user\",\"message\":{\"content\":\"AUDIT_TITLE\"}}\n"), "AUDIT_TITLE");
        assert_eq!(title("{\"type\":\"user\",BROKEN}\n{\"type\":\"user\",\"message\":{\"content\":\"AUDIT_TITLE2\"}}\n"), "AUDIT_TITLE2");
    }

    #[test]
    fn clock_agreement() {
        assert_eq!(agreed_median(vec![0.4, 0.6, 3600.0]), Some(0.6));
        assert_eq!(agreed_median(vec![120.0]), None); // один источник — не верим
        assert_eq!(agreed_median(vec![0.0, 500.0]), None); // двое спорят
        assert_eq!(agreed_median(vec![]), None);
        assert_eq!(clock_row(Some(70.0), Some(true), vec![]).status, OK);
        assert_eq!(clock_row(Some(70.0), Some(false), vec![]).status, WARN);
        assert_eq!(clock_row(Some(-400.0), None, vec![]).value, "отстаёт на 6 мин");
    }

    /// Эталон пишется в формате Python (читается им) и читается обратно. HOME подменён на временную папку.
    #[test]
    fn snapshot_roundtrip() {
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("aptechka_test_{}", std::process::id()));
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", &dir);
        // вернуть HOME после теста (даже если тест упадёт)
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                match &self.0 {
                    Some(h) => std::env::set_var("HOME", h),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let _restore = Restore(old_home);
        let snap = take_snapshot().expect("запись эталона");
        let back = read_snapshot_file().expect("чтение эталона");
        let sorted = |mut v: Vec<(String, String)>| {
            v.sort();
            v
        };
        assert_eq!(sorted(snap.keys), sorted(back.keys));
        assert_eq!(snap.extensions, back.extensions);
        assert_eq!(snap.time, back.time);
        println!("{}", snapshot_path());
        // как в Python: не больше двух слов
        assert_eq!(ext_state("Name: x\n  State: OUT OF DATE\n"), "OUT OF");
        assert_eq!(ext_state("  State: ENABLED\nOther: 1"), "ENABLED");
    }

    #[test]
    fn unchecked_count_uses_marker_not_text() {
        let a = Check::new("sys", "Диск", INFO, "не проверить").unknown();
        let b = Check::new("sys", "Пояс", INFO, "не проверить"); // та же фраза, но без пометки
        assert_eq!(unchecked_count(&[a, b]), 1);
    }

    // ── замер скорости против локального сервера (в Cloudflare тесты не ходят) ──
    static SPEED_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Локальный сервер: `handler` получает сокет после чтения заголовков запроса (до пустой строки).
    fn local_server(handler: fn(std::net::TcpStream)) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            for c in l.incoming().flatten() {
                thread::spawn(move || handler(c));
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn read_head(c: &mut std::net::TcpStream) {
        let mut acc = Vec::new();
        let mut b = [0u8; 1];
        while !acc.ends_with(b"\r\n\r\n") {
            if c.read(&mut b).unwrap_or(0) == 0 {
                return;
            }
            acc.push(b[0]);
        }
    }

    fn run_transfer(base: String, down: bool, limit: f64) -> (Result<Option<f64>, String>, f64) {
        *TEST_BASE.lock().unwrap() = Some(base);
        let t = Instant::now();
        let r = transfer(&mut |_| {}, down, 2, limit, 1 << 30, 1 << 20, false);
        *TEST_BASE.lock().unwrap() = None;
        (r, t.elapsed().as_secs_f64())
    }

    /// Сервер отдаёт заголовок по байту раз в секунду: замер не должен висеть (общий срок запроса + ограниченное ожидание потоков).
    #[test]
    fn speed_slowloris_does_not_hang() {
        use std::io::Write;
        let _g = SPEED_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let base = local_server(|mut c| {
            read_head(&mut c);
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nX-A: ");
            for _ in 0..60 {
                thread::sleep(Duration::from_millis(1000));
                if c.write_all(b"a").is_err() {
                    break;
                }
            }
        });
        let (r, secs) = run_transfer(base, true, 2.0);
        assert!(secs < 8.0, "замер завис на {secs} с");
        assert!(matches!(r, Ok(None) | Err(_)));
    }

    /// Любой не-2xx ответ на отдачу — сбой, а не «отдала».
    #[test]
    fn speed_push_non_2xx_is_failure() {
        use std::io::Write;
        let _g = SPEED_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let base = local_server(|mut c| {
            read_head(&mut c);
            let _ = c.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            // дочитываем тело до конца, иначе закрытие сокета с непрочитанным даст RST вместо ответа
            let _ = c.set_read_timeout(Some(Duration::from_millis(500)));
            let mut sink = [0u8; 65536];
            while c.read(&mut sink).unwrap_or(0) > 0 {}
        });
        let (r, secs) = run_transfer(base, false, 6.0);
        assert!(secs < 8.0);
        let e = r.expect_err("403 не должен считаться успехом");
        assert!(e.contains("403"), "{e}");
    }

    /// Пустой ответ 200 — сбой (после нескольких попыток), а не бесконечные пустые запросы.
    #[test]
    fn speed_pull_empty_is_failure() {
        use std::io::Write;
        let _g = SPEED_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let base = local_server(|mut c| {
            read_head(&mut c);
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        });
        let (r, secs) = run_transfer(base, true, 6.0);
        assert!(secs < 8.0);
        assert!(r.is_err(), "{r:?}");
    }
}

/// История скорости живёт только в памяти: сравнение с «обычным» появляется после 3 замеров и ничего не пишет на диск.
#[cfg(test)]
#[test]
fn speed_history_is_memory_only() {
    let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("aptechka_sh_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let old = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &root);
    SPEED_HIST.lock().unwrap_or_else(|e| e.into_inner()).clear();
    assert!(speed_usual(true).is_none());
    for v in [10.0, 30.0, 20.0] {
        speed_remember(&SpeedRec { ping: None, down: Some(v), up: None });
    }
    assert_eq!(speed_usual(true), Some(20.0));
    assert_eq!(speed_usual(false), None);
    for _ in 0..50 {
        speed_remember(&SpeedRec { ping: None, down: Some(5.0), up: None });
    }
    assert_eq!(SPEED_HIST.lock().unwrap_or_else(|e| e.into_inner()).len(), SPEED_HIST_MAX);
    // ни одного файла истории нигде в папке настроек
    let found = std::fs::read_dir(&root).map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().contains("speed_history"))).unwrap_or(false);
    assert!(!found);
    SPEED_HIST.lock().unwrap_or_else(|e| e.into_inner()).clear();
    match old {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(test)]
mod ignore_tests {
    use super::*;

    /// Игнорируемая беда становится «для сведения», прочие строки и игнорируемая «в порядке» — как были.
    #[test]
    fn ignored_problems_become_info() {
        let ig: std::collections::HashSet<String> = ["Диск".to_string(), "Память".to_string()].into();
        let rows =
            [Check::new("sys", "Диск", "bad", "мало места"), Check::new("sys", "Память", "ok", "15%"), Check::new("net", "DNS", "warn", "медленно")];
        let out = ignore_in(&rows, &ig);
        let st: Vec<&str> = out.iter().map(|c| c.status.as_str()).collect();
        assert_eq!(st, ["info", "ok", "warn"]);
        assert_eq!(out[0].value, "мало места");
    }
}
