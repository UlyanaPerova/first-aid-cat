// часть разбора нужна только одной системе, остальные её не вызывают
#![allow(dead_code)]
//! Разбор вывода системных команд macOS и Windows — чистыми функциями, без обращения к системе: образцы настоящего
//! вывода лежат в тестах, и разбор проверяется на любой системе (на Linux тоже). Сами команды запускают mac.rs и win.rs.
//! Здесь же строки таблицы по разобранному: память, нагрузка, Wi-Fi, прокси в переменных Windows.

use crate::diag::{Check, BAD, INFO, OK, WARN};

/// Число с запятой или точкой («1,62» — так выводит sysctl на русской macOS).
fn num(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok()
}

/// macOS `sysctl -n vm.loadavg`: «{ 1,62 2,10 2,94 }» → нагрузка за минуту.
pub fn parse_loadavg(t: &str) -> Option<f64> {
    num(t.trim().trim_matches(|c| c == '{' || c == '}').split_whitespace().next()?)
}

/// Строка «Процессор» по средней нагрузке за минуту и числу ядер. Нагрузка выше числа ядер — программы ждут очереди.
pub fn load_row(l1: f64, cores: usize) -> Check {
    let cores = cores.max(1);
    let c = Check::new("sys", "Процессор", if l1 > cores as f64 * 1.5 { WARN } else { OK }, &format!("нагрузка {l1:.1} на {cores} ядер"));
    if l1 > cores as f64 * 1.5 {
        c.hint("Программы стоят в очереди за процессором: компьютер тормозит. Посмотри строки «Занят процессор» — там видно, кто грузит.")
    } else {
        c
    }
}

/// macOS `memory_pressure`: последняя строка «System-wide memory free percentage: 68%» → свободно, %.
pub fn parse_free_percent(t: &str) -> Option<u32> {
    let l = t.lines().find(|l| l.contains("free percentage"))?;
    l.rsplit(':').next()?.trim().trim_end_matches('%').trim().parse().ok()
}

/// macOS `sysctl kern.memorystatus_vm_pressure_level` (с названием или без): 1 — норма, 2 — предупреждение, 4 — критично.
pub fn parse_pressure_level(t: &str) -> Option<u32> {
    t.trim().rsplit([':', ' ']).next()?.trim().parse().ok()
}

/// macOS `sysctl vm.swapusage`: «total = 3072,00M  used = 1861,50M  free = 1210,50M  (encrypted)» → (всего, занято) в МБ.
pub fn parse_swapusage(t: &str) -> Option<(f64, f64)> {
    let get = |key: &str| {
        let r = t.split(key).nth(1)?.trim_start_matches([' ', '=']);
        let w = r.split_whitespace().next()?;
        let mul = match w.chars().last()? {
            'G' => 1024.0,
            'K' => 1.0 / 1024.0,
            _ => 1.0,
        };
        Some(num(w.trim_end_matches(['M', 'G', 'K']))? * mul)
    };
    Some((get("total")?, get("used")?))
}

/// Строка «Память» на macOS. Решает давление памяти системы (level), а не процент «занято»: macOS нарочно занимает почти
/// всю память кэшем. Нет давления — доля свободной памяти по порогам Linux (85 и 95 % занято).
pub fn mac_memory_row(total_bytes: u64, free_pct: Option<u32>, level: Option<u32>) -> Option<Check> {
    let used = 100.0 - free_pct? as f64;
    let st = match level {
        Some(4) => BAD,
        Some(2) => WARN,
        Some(_) => OK,
        None if used >= 95.0 => BAD,
        None if used >= 85.0 => WARN,
        None => OK,
    };
    let c = Check::new("sys", "Память", st, &format!("{used:.0}% из {:.0} ГБ", total_bytes as f64 / 1073741824.0));
    Some(if st == OK {
        c
    } else {
        c.hint("Системе не хватает свободной памяти: она сжимает данные и сбрасывает их на диск, из-за этого всё тормозит. Закрой лишние программы и вкладки браузера.")
    })
}

/// Подкачка macOS: строка только когда она в ходу. Много подкачки (больше 4 ГБ и больше половины) — признак нехватки памяти.
pub fn mac_swap_row(total_mb: f64, used_mb: f64) -> Option<Check> {
    if used_mb < 1.0 {
        return None;
    }
    let busy = used_mb > 4096.0 && used_mb > total_mb * 0.5;
    let c =
        Check::new("sys", "Подкачка", if busy { WARN } else { INFO }, &format!("занято {:.1} ГБ из {:.1} ГБ", used_mb / 1024.0, total_mb / 1024.0));
    Some(if busy {
        c.hint("Память вытесняется на диск — программ открыто больше, чем помещается. Закрой лишние.")
    } else {
        c
    })
}

/// Строка «Память» на Windows по GlobalMemoryStatusEx: физическая память и «лимит выделения» (оперативная + файл подкачки).
pub fn win_memory_rows(total_phys: u64, avail_phys: u64, commit_limit: u64, commit_avail: u64) -> Vec<Check> {
    let mut out = Vec::new();
    if total_phys == 0 {
        return out;
    }
    let used = 1.0 - avail_phys as f64 / total_phys as f64;
    let st = if used < 0.85 {
        OK
    } else if used < 0.95 {
        WARN
    } else {
        BAD
    };
    let c = Check::new("sys", "Память", st, &format!("{:.0}% из {:.0} ГБ", used * 100.0, total_phys as f64 / 1073741824.0));
    out.push(if st == OK {
        c
    } else {
        c.hint("Почти вся память занята: Windows сбрасывает данные на диск, и всё тормозит. Закрой лишние программы и вкладки; посмотреть, кто ест память, — Диспетчер задач.")
    });
    // лимит выделения почти исчерпан — программы вот-вот получат отказ в памяти, даже если «оперативной» ещё хватает
    if commit_limit > 0 && (commit_avail as f64) < commit_limit as f64 * 0.1 {
        out.push(Check::new("sys", "Лимит памяти", WARN, &format!("осталось {:.1} ГБ", commit_avail as f64 / 1073741824.0)).hint(
            "Оперативная память вместе с файлом подкачки почти исчерпана: программы начнут вылетать. Закрой лишние или увеличь файл подкачки.",
        ));
    }
    out
}

/// Приговор «занят долго»: `prev` — (секунды процессора, секунд назад) из прошлого замера, `cpu_now` — сейчас.
/// None — замера слишком давно нет (меньше 50 с) или не было: судить рано. Занят — больше половины ядра за всё окно.
pub fn hot_verdict(prev: Option<(f64, f64)>, cpu_now: f64) -> Option<bool> {
    let (c0, ago) = prev?;
    (ago >= 50.0).then(|| (cpu_now - c0).max(0.0) / ago > 0.5)
}

// ───────────────────────────── Wi-Fi ─────────────────────────────

/// macOS `networksetup -listallhardwareports`: устройство порта «Wi-Fi» (en0).
pub fn parse_wifi_device(t: &str) -> Option<String> {
    let mut lines = t.lines();
    while let Some(l) = lines.next() {
        if l.trim().strip_prefix("Hardware Port:").is_some_and(|p| matches!(p.trim(), "Wi-Fi" | "AirPort")) {
            let dev = lines.next()?.trim().strip_prefix("Device:")?.trim().to_string();
            // имя идёт в команды — только буквы и цифры
            return (!dev.is_empty() && dev.chars().all(|c| c.is_ascii_alphanumeric())).then_some(dev);
        }
    }
    None
}

/// macOS `networksetup -getairportpower en0`: «Wi-Fi Power (en0): On» → включён ли.
pub fn parse_wifi_power(t: &str) -> Option<bool> {
    let w = t.trim().rsplit(':').next()?.trim().to_lowercase();
    // по-английски «On»/«Off»; на русской системе, если вывод не по-английски, — «Вкл.»/«Выкл.»
    match w.trim_end_matches('.') {
        "on" | "вкл" => Some(true),
        "off" | "выкл" => Some(false),
        _ => None,
    }
}

/// Что известно о подключении к Wi-Fi.
#[derive(Debug, Default, PartialEq)]
pub struct WifiLink {
    pub ssid: Option<String>,
    /// сила сигнала: macOS — dBm (отрицательное), Windows — проценты
    pub signal: Option<i32>,
    pub channel: Option<String>,
}

/// macOS `system_profiler SPAirPortDataType`: None — не подключён. Имя сети с macOS 14 скрыто («<redacted>») — тогда без имени.
pub fn parse_sp_airport(t: &str) -> Option<WifiLink> {
    let mut connected = false;
    let mut link = WifiLink::default();
    let mut lines = t.lines().map(str::trim);
    while let Some(l) = lines.next() {
        if l.starts_with("Other Local Wi-Fi Networks") {
            break;
        }
        if l == "Status: Connected" {
            connected = true;
        } else if l == "Current Network Information:" && connected {
            if let Some(n) = lines.next().and_then(|n| n.strip_suffix(':')) {
                link.ssid = Some(n.to_string()).filter(|n| !n.is_empty() && n != "<redacted>");
            }
        } else if connected {
            if let Some(v) = l.strip_prefix("Signal / Noise:") {
                link.signal = v.split_whitespace().next().and_then(|x| x.parse().ok());
            } else if let Some(v) = l.strip_prefix("Channel:") {
                link.channel = Some(v.trim().to_string()).filter(|c| !c.is_empty());
            }
        }
    }
    connected.then_some(link)
}

/// Один адаптер Wi-Fi из `netsh wlan show interfaces`.
#[derive(Debug, PartialEq)]
pub struct WlanIface {
    pub name: String,
    pub link: Option<WifiLink>,
}

/// Windows `netsh wlan show interfaces`. Названия строк зависят от языка («Signal» / «Сигнал», «SSID» / «Имя SSID»),
/// поэтому не по названиям: сеть — строка со словом SSID (но не BSSID), сигнал — единственное значение с «%», имя адаптера —
/// первая строка блока. Подключён — когда есть и сеть, и сигнал.
pub fn parse_netsh_wlan(t: &str) -> Vec<WlanIface> {
    let mut out: Vec<WlanIface> = Vec::new();
    let mut cur: Option<(String, Option<String>, Option<i32>, Option<String>)> = None;
    let mut blank_before = true;
    let finish = |c: Option<(String, Option<String>, Option<i32>, Option<String>)>, out: &mut Vec<WlanIface>| {
        if let Some((name, ssid, sig, ch)) = c {
            let link = (ssid.is_some() && sig.is_some()).then_some(WifiLink { ssid, signal: sig, channel: ch });
            out.push(WlanIface { name, link });
        }
    };
    for l in t.lines() {
        let (key, val) = match l.split_once(':') {
            Some((k, v)) if l.starts_with(' ') && !k.trim().is_empty() => (k.trim(), v.trim()),
            _ => {
                blank_before = l.trim().is_empty();
                continue;
            }
        };
        // новый адаптер: первая строка «Имя : …» после пустой (или в самом начале), если блок уже идёт
        if blank_before && cur.is_some() {
            finish(cur.take(), &mut out);
        }
        blank_before = false;
        match cur.as_mut() {
            None => cur = Some((val.to_string(), None, None, None)),
            Some(c) => {
                let up = key.to_ascii_uppercase();
                if up.contains("SSID") && !up.contains("BSSID") {
                    c.1 = Some(val.to_string()).filter(|v| !v.is_empty());
                } else if val.ends_with('%') && val.trim_end_matches('%').trim().parse::<i32>().is_ok() {
                    c.2 = val.trim_end_matches('%').trim().parse().ok();
                } else if (up == "CHANNEL" || key.starts_with("Канал")) && val.parse::<u32>().is_ok() {
                    c.3 = Some(val.to_string());
                }
            }
        }
    }
    finish(cur.take(), &mut out);
    out
}

/// Строка «Wi-Fi» по состоянию подключения. `dbm` — сигнал в dBm (macOS) или `pct` — в процентах (Windows).
pub fn wifi_row(link: &WifiLink, is_dbm: bool) -> Check {
    let (weak, mid, sig) = match (link.signal, is_dbm) {
        (Some(s), true) => (s <= -75, s <= -67, format!(" ({s} dBm)")),
        (Some(s), false) => (s < 35, s < 60, format!(" ({s}%)")),
        (None, _) => (false, false, String::new()),
    };
    let quality = if link.signal.is_none() {
        "подключён"
    } else if weak {
        "сигнал слабый"
    } else if mid {
        "сигнал средний"
    } else {
        "сигнал хороший"
    };
    let net = link.ssid.as_deref().map(|s| format!("«{s}», ")).unwrap_or_default();
    let ch = link.channel.as_deref().map(|c| format!(", канал {c}")).unwrap_or_default();
    let c = Check::new("net", "Wi-Fi", if weak { WARN } else { OK }, &format!("{net}{quality}{sig}{ch}"));
    if weak {
        c.hint("Роутер далеко или мешают стены: интернет будет рваным. Подойди ближе к роутеру или подключись кабелем.")
    } else {
        c
    }
}

// ───────────────────────────── Переменные Windows ─────────────────────────────

/// Windows `reg query HKCU\Environment`: строки «    имя    REG_SZ    значение» → (имя, значение).
pub fn parse_reg_values(t: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for l in t.lines() {
        if !l.starts_with(' ') {
            continue;
        }
        let Some((name, rest)) = l.split_once("    REG_") else { continue };
        let Some((_ty, val)) = rest.split_once("    ") else { continue };
        out.push((name.trim().to_string(), val.trim().to_string()));
    }
    out
}

/// Прокси среди переменных окружения из реестра: (имя, значение). Без no_proxy; пустые значения не считаются.
pub fn proxy_vars(vals: &[(String, String)]) -> Vec<(String, String)> {
    vals.iter()
        .filter(|(n, v)| {
            !v.trim().is_empty()
                && matches!(n.to_ascii_lowercase().as_str(), "http_proxy" | "https_proxy" | "all_proxy" | "ftp_proxy" | "socks_proxy")
        })
        .cloned()
        .collect()
}

// ───────────────────────────── Перезапуск клиентов VPN ─────────────────────────────

/// Клиент VPN, который Аптечка умеет перезапускать на macOS и Windows (как Happ, Outline и Proton VPN на Linux).
pub struct VpnClient {
    /// код в аргументе кнопки
    pub id: &'static str,
    pub title: &'static str,
    /// Windows: имена процессов окна программы без «.exe» (службы клиента не трогаем)
    pub win_procs: &'static [&'static str],
    /// macOS: имя процесса и имя программы в /Applications
    pub mac_proc: &'static str,
    pub mac_app: &'static str,
}

pub const VPN_CLIENTS: &[VpnClient] = &[
    VpnClient { id: "happ", title: "Happ", win_procs: &["Happ"], mac_proc: "Happ", mac_app: "Happ" },
    VpnClient { id: "outline", title: "Outline", win_procs: &["Outline"], mac_proc: "Outline", mac_app: "Outline" },
    VpnClient { id: "proton", title: "Proton VPN", win_procs: &["ProtonVPN.Client", "ProtonVPN"], mac_proc: "ProtonVPN", mac_app: "ProtonVPN" },
];

pub fn vpn_client(id: &str) -> Option<&'static VpnClient> {
    VPN_CLIENTS.iter().find(|c| c.id == id)
}

/// Код клиента по названию из списка Windows/macOS (VPN-строка «Happ», «Proton VPN», «ProtonVPN», «Outline»).
pub fn vpn_client_by_name(name: &str) -> Option<&'static VpnClient> {
    let n = name.to_lowercase().replace(' ', "");
    VPN_CLIENTS.iter().find(|c| n.contains(&c.title.to_lowercase().replace(' ', "")) || n.contains(c.mac_app.to_lowercase().as_str()))
}

/// Windows: PowerShell-скрипт перезапуска клиента по пути его запущенной программы. Окно просим закрыться, не закрылось за
/// 4 секунды — Stop-Process; запускаем заново по тому же пути, только когда прежняя копия закрылась (иначе — отказ).
/// Отказы — метки NOT_RUNNING (клиент не запущен — путь неизвестен) и STILL_RUNNING (не закрылся): fixes.rs переводит их в слова.
pub fn win_vpn_restart_script(procs: &[&str]) -> String {
    let names = procs.iter().map(|n| format!("'{}'", n.replace('\'', "''"))).collect::<Vec<_>>().join(",");
    format!(
        "$n = @({names}); \
         $p = @(Get-Process -Name $n -ErrorAction SilentlyContinue | Where-Object {{ $_.Path }}); \
         if (-not $p) {{ throw 'NOT_RUNNING' }}; \
         $exe = $p[0].Path; \
         $p | ForEach-Object {{ [void]$_.CloseMainWindow() }}; Start-Sleep 4; \
         Get-Process -Name $n -ErrorAction SilentlyContinue | Stop-Process -Force; Start-Sleep 2; \
         if (Get-Process -Name $n -ErrorAction SilentlyContinue) {{ throw 'STILL_RUNNING' }}; \
         Start-Process -FilePath $exe"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_load_and_memory() {
        // настоящий вывод macOS 26 (русская локаль — запятая в числах)
        assert_eq!(parse_loadavg("{ 1,62 2,10 2,94 }"), Some(1.62));
        assert_eq!(parse_loadavg("{ 0.50 0.40 0.30 }\n"), Some(0.5));
        assert_eq!(parse_loadavg(""), None);
        assert_eq!(load_row(1.6, 8).status, "ok");
        assert_eq!(load_row(20.0, 8).status, "warn");
        let mp = "The system has 17179869184 (1048576 pages with a page size of 16384).\n\nStats: \nPages free: 54617 \n\nSystem-wide memory free percentage: 68%\n";
        assert_eq!(parse_free_percent(mp), Some(68));
        assert_eq!(parse_free_percent("нет"), None);
        assert_eq!(parse_pressure_level("kern.memorystatus_vm_pressure_level: 1\n"), Some(1));
        assert_eq!(parse_pressure_level("4"), Some(4));
        assert_eq!(parse_swapusage("vm.swapusage: total = 3072,00M  used = 1861,50M  free = 1210,50M  (encrypted)"), Some((3072.0, 1861.5)));
        assert_eq!(parse_swapusage("vm.swapusage: total = 2.00G  used = 512.00M  free = 1536.00M"), Some((2048.0, 512.0)));
        assert_eq!(parse_swapusage("мусор"), None);
    }

    #[test]
    fn mac_memory_rows() {
        let g16 = 16u64 << 30;
        let c = mac_memory_row(g16, Some(68), Some(1)).unwrap();
        assert_eq!((c.status.as_str(), c.value.as_str()), ("ok", "32% из 16 ГБ"));
        // давление важнее процента: занято 97%, но система спокойна — не тревога
        assert_eq!(mac_memory_row(g16, Some(3), Some(1)).unwrap().status, "ok");
        assert_eq!(mac_memory_row(g16, Some(30), Some(2)).unwrap().status, "warn");
        assert_eq!(mac_memory_row(g16, Some(30), Some(4)).unwrap().status, "bad");
        assert_eq!(mac_memory_row(g16, Some(10), None).unwrap().status, "warn");
        assert!(mac_memory_row(g16, None, Some(1)).is_none());
        assert!(mac_swap_row(3072.0, 0.0).is_none());
        assert_eq!(mac_swap_row(3072.0, 1861.5).unwrap().status, "info");
        assert_eq!(mac_swap_row(8192.0, 6000.0).unwrap().status, "warn");
    }

    #[test]
    fn windows_memory_rows() {
        let g = 1u64 << 30;
        let r = win_memory_rows(16 * g, 8 * g, 30 * g, 20 * g);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].status.as_str(), r[0].value.as_str()), ("ok", "50% из 16 ГБ"));
        assert_eq!(win_memory_rows(16 * g, g, 30 * g, 20 * g)[0].status, "warn");
        assert_eq!(win_memory_rows(16 * g, g / 4, 30 * g, 20 * g)[0].status, "bad");
        let r = win_memory_rows(16 * g, 8 * g, 30 * g, g);
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].title, "Лимит памяти");
        assert!(win_memory_rows(0, 0, 0, 0).is_empty());
    }

    #[test]
    fn mac_wifi() {
        let hw =
            "Hardware Port: Ethernet Adapter (en3)\nDevice: en3\nEthernet Address: aa\n\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: bb\n";
        assert_eq!(parse_wifi_device(hw).as_deref(), Some("en0"));
        assert_eq!(parse_wifi_device("Hardware Port: Thunderbolt 1\nDevice: en1\n"), None);
        assert_eq!(parse_wifi_device("Hardware Port: Wi-Fi\nDevice: en0; rm\n"), None);
        assert_eq!(parse_wifi_power("Wi-Fi Power (en0): On\n"), Some(true));
        assert_eq!(parse_wifi_power("Wi-Fi Power (en0): Off"), Some(false));
        assert_eq!(parse_wifi_power("Питание Wi-Fi (en0): Выкл."), Some(false));
        assert_eq!(parse_wifi_power("Error: not a Wi-Fi interface"), None);
        // настоящий вывод macOS 26 (имена сетей скрыты самой системой)
        let sp = "Wi-Fi:\n\n      Interfaces:\n        en0:\n          Card Type: Wi-Fi  (0x14E4, 0x4388)\n          Status: Connected\n          Current Network Information:\n            <redacted>:\n              PHY Mode: 802.11ax\n              Channel: 36 (5GHz, 80MHz)\n              Signal / Noise: -61 dBm / -92 dBm\n              Transmit Rate: 576\n          Other Local Wi-Fi Networks:\n            <redacted>:\n              Channel: 11 (2GHz, 20MHz)\n              Signal / Noise: -80 dBm / -92 dBm\n";
        let l = parse_sp_airport(sp).unwrap();
        assert_eq!(l, WifiLink { ssid: None, signal: Some(-61), channel: Some("36 (5GHz, 80MHz)".into()) });
        let r = wifi_row(&l, true);
        assert_eq!((r.status.as_str(), r.value.as_str()), ("ok", "сигнал хороший (-61 dBm), канал 36 (5GHz, 80MHz)"));
        assert_eq!(wifi_row(&WifiLink { signal: Some(-82), ..Default::default() }, true).status, "warn");
        assert!(wifi_row(&WifiLink { signal: Some(-70), ..Default::default() }, true).value.starts_with("сигнал средний"));
        // не подключён
        assert_eq!(parse_sp_airport("        en0:\n          Status: Not Connected\n"), None);
        // с видимым именем сети
        let named = "          Status: Connected\n          Current Network Information:\n            Home:\n              Signal / Noise: -50 dBm / -90 dBm\n";
        assert_eq!(parse_sp_airport(named).unwrap().ssid.as_deref(), Some("Home"));
    }

    #[test]
    fn windows_netsh_wlan_both_languages() {
        let en = "\nThere is 1 interface on the system:\n\n    Name                   : Wi-Fi\n    Description            : Intel(R) Wi-Fi 6 AX201 160MHz\n    GUID                   : 11111111-2222-3333-4444-555555555555\n    Physical address       : aa:bb:cc:dd:ee:ff\n    State                  : connected\n    SSID                   : HomeNet\n    BSSID                  : 11:22:33:44:55:66\n    Network type           : Infrastructure\n    Radio type             : 802.11ac\n    Authentication         : WPA2-Personal\n    Channel                : 36\n    Receive rate (Mbps)    : 866.7\n    Signal                 : 91%\n    Profile                : HomeNet\n";
        let v = parse_netsh_wlan(en);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Wi-Fi");
        assert_eq!(v[0].link, Some(WifiLink { ssid: Some("HomeNet".into()), signal: Some(91), channel: Some("36".into()) }));
        let r = wifi_row(v[0].link.as_ref().unwrap(), false);
        assert_eq!(r.value, "«HomeNet», сигнал хороший (91%), канал 36");
        let ru = "\nВ системе 1 интерфейс:\n\n    Имя                    : Беспроводная сеть\n    Описание               : Realtek 8822CE Wireless LAN\n    GUID                   : 11111111-2222-3333-4444-555555555555\n    Физический адрес       : aa:bb:cc:dd:ee:ff\n    Состояние              : подключено\n    Имя SSID               : Дом\n    BSSID                  : 11:22:33:44:55:66\n    Тип сети               : Инфраструктура\n    Тип радио              : 802.11n\n    Проверка подлинности   : WPA2-Personal\n    Канал                  : 6\n    Сигнал                 : 28%\n    Профиль                : Дом\n";
        let v = parse_netsh_wlan(ru);
        assert_eq!(v[0].name, "Беспроводная сеть");
        let l = v[0].link.as_ref().unwrap();
        assert_eq!((l.ssid.as_deref(), l.signal, l.channel.as_deref()), (Some("Дом"), Some(28), Some("6")));
        assert_eq!(wifi_row(l, false).status, "warn");
        // не подключён: ни сети, ни сигнала
        let off = "\n    Name                   : Wi-Fi\n    Description            : Intel\n    State                  : disconnected\n";
        let v = parse_netsh_wlan(off);
        assert_eq!((v.len(), v[0].link.is_none()), (1, true));
        // два адаптера, второй подключён
        let two = format!("{off}\n{}", en.trim_start_matches('\n').split_once("\n\n").unwrap().1);
        let v = parse_netsh_wlan(&two);
        assert_eq!(v.len(), 2);
        assert!(v[0].link.is_none() && v[1].link.is_some());
        // служба Wi-Fi не запущена — адаптеров нет
        assert!(parse_netsh_wlan("The Wireless AutoConfig Service (wlansvc) is not running.\n").is_empty());
    }

    #[test]
    fn hot_process_verdict() {
        assert_eq!(hot_verdict(None, 100.0), None);
        assert_eq!(hot_verdict(Some((10.0, 20.0)), 40.0), None); // замер свежий — рано судить
        assert_eq!(hot_verdict(Some((10.0, 60.0)), 70.0), Some(true)); // 60 с процессора за 60 с
        assert_eq!(hot_verdict(Some((10.0, 60.0)), 20.0), Some(false)); // секундный всплеск
        assert_eq!(hot_verdict(Some((10.0, 60.0)), 5.0), Some(false)); // счётчик сбросился
    }

    #[test]
    fn vpn_clients() {
        assert_eq!(vpn_client("happ").unwrap().title, "Happ");
        assert!(vpn_client("happ; rm").is_none() && vpn_client("").is_none());
        assert_eq!(vpn_client_by_name("Proton VPN").unwrap().id, "proton");
        assert_eq!(vpn_client_by_name("VPN ProtonVPN").unwrap().id, "proton");
        assert_eq!(vpn_client_by_name("Outline").unwrap().id, "outline");
        assert!(vpn_client_by_name("AdGuard VPN").is_none());
        let s = win_vpn_restart_script(vpn_client("proton").unwrap().win_procs);
        assert!(s.contains("@('ProtonVPN.Client','ProtonVPN')") && s.contains("Start-Process -FilePath $exe") && s.contains("Stop-Process -Force"));
    }

    #[test]
    fn windows_registry_proxy_variables() {
        let t = "\nHKEY_CURRENT_USER\\Environment\n    OneDrive    REG_SZ    C:\\Users\\x\\OneDrive\n    HTTP_PROXY    REG_SZ    http://127.0.0.1:10809\n    Path    REG_EXPAND_SZ    %USERPROFILE%\\bin;C:\\Program Files\\x\n    no_proxy    REG_SZ    localhost\n    https_proxy    REG_SZ    \n    ALL_PROXY    REG_SZ    socks5://127.0.0.1:10808\n\n";
        let v = parse_reg_values(t);
        assert_eq!(v.len(), 6);
        assert_eq!(v[2], ("Path".to_string(), "%USERPROFILE%\\bin;C:\\Program Files\\x".to_string()));
        let p = proxy_vars(&v);
        assert_eq!(
            p,
            vec![("HTTP_PROXY".to_string(), "http://127.0.0.1:10809".to_string()), ("ALL_PROXY".to_string(), "socks5://127.0.0.1:10808".to_string())]
        );
        // ключа нет — «ERROR: The system was unable to find the specified registry key or value.»
        assert!(parse_reg_values("ERROR: The system was unable to find the specified registry key or value.\n").is_empty());
    }
}
