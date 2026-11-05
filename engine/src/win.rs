//! Проверки Windows. Данные — из PowerShell в JSON (вывод команд в русской Windows переведён, а JSON нет).
//! Имена программ и служб VPN — с настоящей Windows (2026-10-31): Happ (FlyFrogLLC, HappService, интерфейс
//! happ-tun, SOCKS 127.0.0.1:10808), Outline (OutlineService, TAP tap0901), Proton VPN (ProtonVPN Service, wintun).
#![cfg(windows)]

use std::time::Duration;

use crate::diag::{fx, run, Check, BAD, INFO, OK, WARN};

/// Выполнить PowerShell и разобрать JSON (одиночный объект → массив из одного).
pub fn ps(script: &str, timeout_s: u64) -> Vec<serde_json::Value> {
    ps_try(script, timeout_s).unwrap_or_default()
}

/// То же, но None — PowerShell не ответил (таймаут, ошибка, непонятный вывод): это «не проверить»,
/// а не «ничего не нашлось». Пустой массив — честно ничего.
pub fn ps_try(script: &str, timeout_s: u64) -> Option<Vec<serde_json::Value>> {
    let full = format!(
        "$ProgressPreference='SilentlyContinue'; [Console]::OutputEncoding=[Text.Encoding]::UTF8; @({script}) | ConvertTo-Json -Depth 4 -Compress"
    );
    let r = crate::diag::run_ex(&["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &full], timeout_s);
    if r.code() != 0 {
        return None;
    }
    let out = r.text().trim();
    // ConvertTo-Json пустого @() печатает пустоту или «[]»
    if out.is_empty() {
        return Some(Vec::new());
    }
    match serde_json::from_str::<serde_json::Value>(out) {
        Ok(serde_json::Value::Array(a)) => Some(a),
        Ok(v) if !v.is_null() => Some(vec![v]),
        Ok(_) => Some(Vec::new()),
        Err(_) => None,
    }
}

fn s(v: &serde_json::Value, k: &str) -> String {
    match &v[k] {
        serde_json::Value::String(x) => x.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ───── прямые вызовы Windows вместо PowerShell (раньше на каждую проверку VPN — два PowerShell: нагрев) ─────

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Состояние службы: Some((работает, отключена)) или None — службы нет.
fn svc_state(name: &str) -> Option<(bool, bool)> {
    use core::ffi::c_void;
    #[repr(C)]
    struct Status {
        kind: u32,
        state: u32,
        accepted: u32,
        exit: u32,
        svc_exit: u32,
        checkpoint: u32,
        wait: u32,
    }
    #[link(name = "advapi32")]
    extern "system" {
        fn OpenSCManagerW(m: *const u16, d: *const u16, a: u32) -> *mut c_void;
        fn OpenServiceW(m: *mut c_void, n: *const u16, a: u32) -> *mut c_void;
        fn QueryServiceStatus(s: *mut c_void, st: *mut Status) -> i32;
        fn QueryServiceConfigW(s: *mut c_void, cfg: *mut u8, size: u32, need: *mut u32) -> i32;
        fn CloseServiceHandle(h: *mut c_void) -> i32;
    }
    unsafe {
        let m = OpenSCManagerW(std::ptr::null(), std::ptr::null(), 0x0001); // CONNECT
        if m.is_null() {
            return None;
        }
        let n = wide(name);
        let h = OpenServiceW(m, n.as_ptr(), 0x0004 | 0x0001); // QUERY_STATUS | QUERY_CONFIG
        if h.is_null() {
            CloseServiceHandle(m);
            return None;
        }
        let mut st = Status { kind: 0, state: 0, accepted: 0, exit: 0, svc_exit: 0, checkpoint: 0, wait: 0 };
        let ok = QueryServiceStatus(h, &mut st) != 0;
        // тип запуска: QUERY_SERVICE_CONFIGW.dwStartType — второе поле (4 = отключена)
        let mut buf = vec![0u8; 8192];
        let mut need = 0u32;
        let disabled =
            QueryServiceConfigW(h, buf.as_mut_ptr(), buf.len() as u32, &mut need) != 0 && u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) == 4;
        CloseServiceHandle(h);
        CloseServiceHandle(m);
        ok.then_some((st.state == 4, disabled)) // 4 = SERVICE_RUNNING
    }
}

/// Сетевые адаптеры (и скрытые — туннели VPN): (имя, описание, поднят).
/// Сетевой адаптер: имя, описание (модель/драйвер), включён ли, тип (IF_TYPE_*).
struct Adapter {
    name: String,
    desc: String,
    up: bool,
    if_type: u32,
}

impl Adapter {
    fn text(&self) -> String {
        format!("{} {}", self.name, self.desc).to_lowercase()
    }
    /// Похож на VPN-туннель по типу устройства: виртуальный (Wintun, 53), туннель (131), PPP (23),
    /// драйвер TAP-Windows (OpenVPN и др.). Служебные туннели Windows (Teredo, ISATAP, 6to4) и сети
    /// виртуальных машин — нет.
    fn vpn_like(&self) -> bool {
        let t = self.text();
        let system = ["teredo", "isatap", "6to4", "ip-https", "kernel debug", "hyper-v", "virtualbox", "vmware", "loopback"];
        if system.iter().any(|w| t.contains(w)) {
            return false;
        }
        matches!(self.if_type, 23 | 53 | 131) || t.contains("tap-windows") || t.contains("wintun")
    }
}

fn adapters() -> Vec<Adapter> {
    use core::ffi::c_void;
    // начало IP_ADAPTER_ADDRESSES (x64): нужны имя, описание и OperStatus
    #[repr(C)]
    struct Aa {
        length: u32,
        if_index: u32,
        next: *const Aa,
        adapter_name: *const u8,
        unicast: *const c_void,
        anycast: *const c_void,
        multicast: *const c_void,
        dns: *const c_void,
        dns_suffix: *const u16,
        description: *const u16,
        friendly: *const u16,
        phys: [u8; 8],
        phys_len: u32,
        flags: u32,
        mtu: u32,
        if_type: u32,
        oper: u32,
    }
    #[link(name = "iphlpapi")]
    extern "system" {
        fn GetAdaptersAddresses(family: u32, flags: u32, reserved: *mut c_void, a: *mut u8, size: *mut u32) -> u32;
    }
    let wstr = |p: *const u16| -> String {
        if p.is_null() {
            return String::new();
        }
        unsafe {
            let mut n = 0;
            while *p.add(n) != 0 && n < 512 {
                n += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
        }
    };
    // AF_UNSPEC; без адресов anycast/multicast/DNS, со всеми интерфейсами (и скрытыми)
    let flags = 0x2 | 0x4 | 0x8 | 0x100;
    let mut size: u32 = 64 * 1024;
    let mut out = Vec::new();
    for _ in 0..3 {
        // u64 — чтобы буфер был выровнен под структуру с указателями
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let r = unsafe { GetAdaptersAddresses(0, flags, std::ptr::null_mut(), buf.as_mut_ptr() as *mut u8, &mut size) };
        if r == 111 {
            continue; // ERROR_BUFFER_OVERFLOW — размер уже исправлен
        }
        if r != 0 {
            return out;
        }
        let mut p = buf.as_ptr() as *const Aa;
        while !p.is_null() {
            let a = unsafe { &*p };
            out.push(Adapter { name: wstr(a.friendly), desc: wstr(a.description), up: a.oper == 1, if_type: a.if_type });
            p = a.next;
        }
        return out;
    }
    out
}

/// Место на диске C: (свободно, всего), байты.
fn disk_c() -> Option<(f64, f64)> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    }
    let (mut av, mut tot, mut fr) = (0u64, 0u64, 0u64);
    let d = wide("C:\\");
    (unsafe { GetDiskFreeSpaceExW(d.as_ptr(), &mut av, &mut tot, &mut fr) } != 0).then_some((fr as f64, tot as f64))
}

// ───────────────────────────── Сеть: прокси, VPN ─────────────────────────────

/// Системный прокси Windows (WinINet, HKCU\…\Internet Settings) и WinHTTP.
pub(crate) fn check_proxy() -> Vec<Check> {
    let mut out = Vec::new();
    let Some(v) = ps_try(
        r"Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' | Select-Object ProxyEnable,ProxyServer,AutoConfigURL",
        15,
    ) else {
        // PowerShell не ответил — это не «прокси выключен»
        out.push(Check::new("net", "Системный прокси", INFO, "не удалось проверить").hint("PowerShell не ответил вовремя или вернул ошибку."));
        out.extend(crate::proxyaddr::env_row());
        return out;
    };
    let v = v.first().cloned().unwrap_or_default();
    let enabled = s(&v, "ProxyEnable") == "1";
    let server = s(&v, "ProxyServer");
    let pac = s(&v, "AutoConfigURL");
    let off = || vec![fx("win_proxy_off", "Выключить системный прокси", None)];
    if enabled && !server.is_empty() {
        match crate::proxyaddr::ProxyAddr::parse(&server) {
            Some(p) => out.push(crate::proxyaddr::system_row("Системный прокси", &p, off())),
            None => out.push(Check::new("net", "Системный прокси", INFO, &crate::proxyaddr::redact(&server))),
        }
    } else {
        out.push(Check::new("net", "Системный прокси", OK, "выключен (сеть напрямую)"));
    }
    if !pac.is_empty() {
        out.push(crate::proxyaddr::pac_row(&pac, off()));
    }
    out.extend(crate::proxyaddr::env_row());
    // WinHTTP — отдельное хранилище для служб Windows (обновления, Магазин)
    let (_, wh) = run(&["netsh", "winhttp", "show", "proxy"], 10);
    // «Proxy Server(s) :  127.0.0.1:10809» / «http=127.0.0.1:10809;https=…» (подписи зависят от языка Windows —
    // ищем слово, которое разбирается как адрес на этом компьютере)
    if let Some(p) =
        wh.split_whitespace().filter_map(crate::proxyaddr::ProxyAddr::parse).find(|p| p.is_local() && wh.contains(&format!(":{}", p.port)))
    {
        if !p.listening(Duration::from_millis(300)) {
            let addr = p.shown();
            out.push(
                Check::new("net", "Прокси WinHTTP", BAD, &format!("{addr} — там пусто"))
                    .hint("Службы Windows (обновления, Магазин) идут в пустой прокси.")
                    .fixes(vec![fx("win_winhttp_reset", "Сбросить (нужны права)", None)]),
            );
        }
    }
    out
}

struct VpnApp {
    name: &'static str,
    procs: &'static [&'static str],
    service: &'static str,
    /// Имена интерфейсов-туннелей (части имени или описания).
    tun: &'static [&'static str],
}

const VPNS: &[VpnApp] = &[
    VpnApp { name: "Happ", procs: &["Happ.exe", "happd.exe", "sing-box-tun.exe"], service: "HappService", tun: &["happ-tun"] },
    VpnApp { name: "Outline", procs: &["Outline.exe", "OutlineService.exe"], service: "OutlineService", tun: &["outline-tap"] },
    VpnApp {
        name: "Proton VPN",
        procs: &["ProtonVPN.Client.exe", "ProtonVPN.exe", "ProtonVPNService.exe"],
        service: "ProtonVPN Service",
        tun: &["ProtonVPN"],
    },
    VpnApp { name: "AmneziaVPN", procs: &["AmneziaVPN.exe"], service: "AmneziaVPN-service", tun: &["Amnezia"] },
    VpnApp { name: "Hiddify", procs: &["Hiddify.exe", "HiddifyCli.exe"], service: "", tun: &["hiddify", "sing-tun"] },
    VpnApp { name: "v2rayN", procs: &["v2rayN.exe", "xray.exe"], service: "", tun: &["xray", "singbox_tun"] },
    VpnApp { name: "AdGuard VPN", procs: &["AdGuardVpn.exe", "AdGuardVpnSvc.exe"], service: "", tun: &["AdGuard"] },
    VpnApp { name: "WireGuard", procs: &["wireguard.exe"], service: "WireGuardManager", tun: &["WireGuard"] },
];

pub(crate) fn check_vpn() -> Vec<Check> {
    let mut out = Vec::new();
    let procs: Vec<String> = crate::procs::list().into_iter().map(|p| p.name.to_lowercase()).collect();
    let adapters = adapters();
    let mut claimed: Vec<String> = Vec::new();
    for v in VPNS {
        let running = v.procs.iter().any(|p| procs.contains(&p.to_lowercase()));
        let svc = if v.service.is_empty() { None } else { svc_state(v.service) };
        if !running && svc.is_none() {
            continue; // не установлен
        }
        // туннель поднят, а программы нет — остаток: интернет может идти в никуда
        let mine = |a: &Adapter| v.tun.iter().any(|t| a.text().contains(&t.to_lowercase()));
        let tun_up = adapters.iter().any(|a| a.up && mine(a));
        claimed.extend(adapters.iter().filter(|a| mine(a)).map(|a| a.name.clone()));
        let title = format!("Канал {}", v.name);
        // «программа запущена» — не «VPN включён»: включён, когда поднят его туннель
        if running && tun_up {
            out.push(Check::new("net", &title, OK, "включён (туннель)"));
        } else if running {
            out.push(Check::new("net", &title, INFO, "запущен, не подключён"));
        } else if tun_up {
            out.push(Check::new("net", &title, BAD, "туннель остался без программы")
                .hint("Программа VPN закрыта, а её сетевой интерфейс включён — интернет может идти в никуда. Запусти её и отключись, или перезапусти службу.")
                .fixes(if v.service.is_empty() { Vec::new() } else { vec![fx("win_svc_restart", "Перезапустить службу", Some(v.service))] }));
        } else {
            out.push(Check::new("net", &title, INFO, "выключен"));
        }
    }
    // поднятые туннели без известного клиента (OpenVPN, Tailscale, Cloudflare WARP, свой VPN) — по типу устройства
    for a in adapters.iter().filter(|a| a.up && a.vpn_like() && !claimed.contains(&a.name)) {
        out.push(
            Check::new("net", &format!("Канал {}", a.name), OK, "включён (туннель)")
                .hint(&format!("VPN-туннель без известного Аптечке клиента: {}.", a.desc)),
        );
    }
    // итог — общий для всех систем (страна выхода против своей страны, «интернет падает из-за VPN»)
    let up: Vec<String> = out.iter().filter(|c| c.status == OK).map(|c| c.title.trim_start_matches("Канал ").to_string()).collect();
    // запущены, но без туннеля (режим прокси) — подсказка итогу
    let apps: Vec<String> =
        out.iter().filter(|c| c.value == "запущен, не подключён").map(|c| c.title.trim_start_matches("Канал ").to_string()).collect();
    let installed = !out.is_empty();
    let mut sum = crate::diag::vpn_summary_apps(&up, installed, &apps, Vec::new());
    sum.append(&mut out);
    sum
}

/// Интернет и DNS (страница проверки подключения Microsoft).
pub(crate) fn check_net() -> Vec<Check> {
    crate::netproxy::check_net_basic("win_dns_flush")
}

// ───────────────────────────── Система ─────────────────────────────

/// Что значит код ошибки Windows Update — по-человечески.
fn wu_meaning(code: &str) -> &'static str {
    match code.to_lowercase().as_str() {
        "0x80073d02" => "приложение из Магазина было открыто во время обновления — само пройдёт, когда оно закрыто",
        "0x80240016" => "другая установка уже шла — повторится сама",
        "0x80070005" => "нет доступа — часто мешает антивирус",
        "0x80070070" => "не хватило места на диске",
        "0x800f0922" => "не удалось связаться с серверами обновлений (VPN/прокси?) или мал раздел System Reserved",
        "0x80240fff" | "0x8024402c" | "0x80072efd" | "0x80072ee2" => "нет связи с серверами обновлений — проверь интернет и прокси",
        "0x8007000d" | "0x80073712" => "повреждены файлы обновлений — поможет сброс компонентов",
        _ => "",
    }
}

pub(crate) fn check_updates() -> Vec<Check> {
    let mut out = Vec::new();
    let Some(ev) = ps_try(
        "Get-WinEvent -FilterHashtable @{LogName='System';ProviderName='Microsoft-Windows-WindowsUpdateClient';Id=20;StartTime=(Get-Date).AddDays(-14)} -MaxEvents 200 -ErrorAction SilentlyContinue | ForEach-Object { $x=[xml]$_.ToXml(); [pscustomobject]@{t=$_.TimeCreated.ToString('dd.MM'); code=($x.Event.EventData.Data | Where-Object Name -eq 'errorCode').'#text'; title=($x.Event.EventData.Data | Where-Object Name -eq 'updateTitle').'#text'} }",
        40,
    ) else {
        return vec![Check::new("sys", "Обновления Windows", INFO, "не проверить").hint("Журнал Windows не прочитался вовремя.")];
    };
    if ev.is_empty() {
        out.push(Check::new("sys", "Обновления Windows", OK, "ошибок за 2 недели нет"));
        return out;
    }
    let mut codes: Vec<(String, usize)> = Vec::new();
    for e in &ev {
        let c = s(e, "code");
        match codes.iter_mut().find(|x| x.0 == c) {
            Some(x) => x.1 += 1,
            None => codes.push((c, 1)),
        }
    }
    codes.sort_by_key(|x| std::cmp::Reverse(x.1));
    let (top, n) = &codes[0];
    let mean = wu_meaning(top);
    let harmless = top.eq_ignore_ascii_case("0x80073d02") || top.eq_ignore_ascii_case("0x80240016");
    let c = Check::new(
        "sys",
        "Обновления Windows",
        if harmless { INFO } else { WARN },
        &format!("{} за 2 недели ({top} ×{n})", crate::diag::plural(ev.len(), "ошибка", "ошибки", "ошибок")),
    )
    .hint(&if mean.is_empty() { format!("Последнее: {}.", s(&ev[0], "title")) } else { format!("{top}: {mean}.") });
    out.push(if harmless { c } else { c.fixes(vec![fx("win_update_reset", "Сбросить компоненты обновления", None)]) });
    out
}

pub(crate) fn check_stability() -> Vec<Check> {
    let mut out = Vec::new();
    // Kernel-Power 41 — компьютер выключился без нормального завершения (падение, питание, перегрев)
    let kp = ps("Get-WinEvent -FilterHashtable @{LogName='System';ProviderName='Microsoft-Windows-Kernel-Power';Id=41;StartTime=(Get-Date).AddDays(-30)} -ErrorAction SilentlyContinue | Select-Object @{n='t';e={$_.TimeCreated.ToString('dd.MM HH:mm')}}", 30);
    if !kp.is_empty() {
        out.push(Check::new("sys", "Внезапные выключения", if kp.len() >= 3 { WARN } else { INFO }, &format!("{} за месяц, последнее {}", kp.len(), s(&kp[0], "t")))
            .hint("Компьютер выключался, не завершив работу: падение системы, пропало питание или перегрев. Если часто — проверь блок питания и температуры."));
    }
    // падения и зависания программ за неделю
    let apps = ps("Get-WinEvent -FilterHashtable @{LogName='Application';Id=1000,1002;StartTime=(Get-Date).AddDays(-7)} -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{id=$_.Id; app=$_.Properties[0].Value} }", 30);
    let mut by: Vec<(String, u32, usize)> = Vec::new();
    for a in &apps {
        let (app, id) = (s(a, "app"), a["id"].as_u64().unwrap_or(0) as u32);
        match by.iter_mut().find(|x| x.0 == app && x.1 == id) {
            Some(x) => x.2 += 1,
            None => by.push((app, id, 1)),
        }
    }
    by.sort_by_key(|x| std::cmp::Reverse(x.2));
    for (app, id, n) in by.into_iter().take(3) {
        let what = if id == 1002 { "зависала" } else { "падала" };
        let app = app.rsplit(['\\', '/']).next().unwrap_or(&app).to_string();
        out.push(
            Check::new("sys", &app, if n >= 3 { WARN } else { INFO }, &format!("{what} {} за неделю", crate::diag::plural(n, "раз", "раза", "раз")))
                .hint("Если программа мешает работать — переустанови или обнови её; часто падают оверлеи, подсветка и игры."),
        );
    }
    // зависшие прямо сейчас
    let (_, hung) = run(&["tasklist", "/fi", "STATUS eq NOT RESPONDING", "/fo", "csv", "/nh"], 15);
    for l in hung.lines().filter(|l| l.starts_with('"')) {
        let f: Vec<&str> = l.split("\",\"").map(|x| x.trim_matches('"')).collect();
        let (Some(name), Some(pid)) = (f.first(), f.get(1)) else { continue };
        if name.eq_ignore_ascii_case("explorer.exe") {
            out.push(Check::new("sys", "Проводник", BAD, "не отвечает").hint("Зависли панель задач и окна папок.").fixes(vec![fx(
                "win_explorer_restart",
                "Перезапустить проводник",
                None,
            )]));
        } else {
            out.push(
                Check::new("sys", &format!("Зависла: {name}"), WARN, "не отвечает")
                    .hint("Программа не отвечает. Закрыть её — несохранённое пропадёт.")
                    .fixes(vec![fx("win_kill", "Закрыть принудительно", Some(&format!("{pid}:{name}")))]),
            );
        }
    }
    out
}

pub(crate) fn check_time() -> Vec<Check> {
    vec![crate::diag::clock_row(crate::diag::clock_offset(), None, vec![fx("win_time_sync", "Сверить часы", None)])]
}

pub(crate) fn check_services() -> Vec<Check> {
    let mut out = Vec::new();
    // служба Bluetooth запускается сама, когда есть адаптер; без адаптера её «остановлена» — норма.
    // Адаптер Bluetooth — по сетевым адаптерам («Bluetooth Device (Personal Area Network)») или его службе
    // поддержки, без PowerShell
    // (драйвер BTHPORT работает, только когда есть сам радиомодуль; служба BTAGService есть в любой Windows)
    let has_bt = adapters().iter().any(|a| a.text().contains("bluetooth")) || svc_state("BTHPORT").is_some_and(|(running, _)| running);
    let names = [("Audiosrv", "Звук Windows", "dev"), ("Spooler", "Диспетчер печати", "dev"), ("bthserv", "Служба Bluetooth", "dev")];
    for (n, title, key) in names {
        if n == "bthserv" && !has_bt {
            continue;
        }
        let Some((running, disabled)) = svc_state(n) else { continue };
        if running {
            out.push(Check::new(key, title, OK, "работает"));
        } else if !disabled {
            out.push(Check::new(key, title, BAD, "остановлена").fixes(vec![fx("win_svc_restart", "Запустить", Some(n))]));
        }
    }
    // свободное место на системном диске
    if let Some((free, total)) = disk_c() {
        let pct = free / total.max(1.0);
        let gb = free / 1073741824.0;
        let st = if gb < 5.0 || pct < 0.05 {
            BAD
        } else if gb < 15.0 || pct < 0.10 {
            WARN
        } else {
            OK
        };
        let c = Check::new("sys", "Диск C:", st, &format!("свободно {gb:.0} ГБ ({:.0}%)", pct * 100.0));
        out.push(if st == OK {
            c
        } else {
            c.hint("Мало места: обновления не встанут, система тормозит.").fixes(vec![fx("win_cleanmgr", "Очистка диска", None)])
        });
    }
    out
}

/// Подробности для журнала: версия Windows (одной строкой).
pub(crate) fn check_os() -> Vec<Check> {
    let v = ps(r"Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' | Select-Object ProductName,DisplayVersion,CurrentBuild", 15);
    match v.first() {
        Some(x) => {
            // в реестре Windows 11 до сих пор пишется «Windows 10» — смотрим на номер сборки
            let build: u32 = s(x, "CurrentBuild").parse().unwrap_or(0);
            let name = s(x, "ProductName");
            let name = if build >= 22000 { name.replacen("Windows 10", "Windows 11", 1) } else { name };
            vec![Check::new("sys", "Windows", INFO, &format!("{name} {} (сборка {build})", s(x, "DisplayVersion")))]
        }
        None => Vec::new(),
    }
}
