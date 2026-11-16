#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_imports))]
//! Частые поломки Linux у обычных людей (2026-10-31): звук, Bluetooth и Wi-Fi, печать, раскладка,
//! пакеты, /boot, место на диске, нехватка памяти, прожорливые процессы, Secure Boot и NVIDIA.
//! Все проверки только читают систему; починка — в fixes.rs, с подтверждением и командой «вручную».

use std::path::Path;
use std::time::Duration;

use crate::diag::{fx, run, Check, BAD, INFO, OK, WARN};

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_default().trim().to_string()
}

fn home() -> String {
    crate::store::home_str()
}

// ───────────────────────────── Звук ─────────────────────────────

/// «Volume: 0.40 [MUTED]» → (громкость 0…1.5, выключен ли).
fn parse_volume(s: &str) -> Option<(f64, bool)> {
    let v = s.split_whitespace().nth(1)?.parse::<f64>().ok()?;
    Some((v, s.contains("[MUTED]")))
}

pub(crate) fn check_sound() -> Vec<Check> {
    let mut out = Vec::new();
    if crate::procs::which("wpctl").is_none() {
        return out; // не PipeWire (PulseAudio или без звука) — не берёмся
    }
    let act = crate::diag::run_ex(&["systemctl", "--user", "is-active", "pipewire", "wireplumber"], 5);
    let st = act.text().to_string();
    // нет ни одного ответа по обеим службам (нет пользовательской шины, таймаут) — это «не проверить», а не «всё работает»
    if !act.answered() || st.lines().filter(|l| !l.trim().is_empty()).count() < 2 {
        out.push(
            Check::new("dev", "Звуковая служба", INFO, "не удалось проверить")
                .unknown()
                .hint("systemctl --user не ответил (нет пользовательской сессии systemd?)."),
        );
        return out;
    }
    let down: Vec<&str> = ["pipewire", "wireplumber"].into_iter().zip(st.lines()).filter(|(_, s)| *s != "active").map(|(n, _)| n).collect();
    if !down.is_empty() {
        out.push(
            Check::new("dev", "Звуковая служба", BAD, &format!("не работает: {}", down.join(", ")))
                .hint("Без неё звука не будет нигде.")
                .fixes(vec![fx("sound_restart", "Перезапустить звук", None)]),
        );
        return out;
    }
    let insp = crate::diag::run_ex(&["wpctl", "inspect", "@DEFAULT_AUDIO_SINK@"], 5);
    if !insp.answered() {
        out.push(Check::new("dev", "Звук: вывод", INFO, "не проверить").unknown().hint("Программа wpctl (PipeWire) не ответила."));
        return out;
    }
    let (code, inspect) = (insp.code(), insp.text().to_string());
    // звуковых карт нет совсем (виртуальная машина, сервер, системный блок без звука) — это не поломка
    let cards = read("/proc/asound/cards");
    if cards.trim().is_empty() || cards.contains("no soundcards") {
        out.push(
            Check::new("dev", "Звук: вывод", INFO, "звуковой карты нет")
                .hint("Система не нашла ни одной звуковой карты. Если она есть (встроенная, USB, HDMI) — не хватает драйвера (sof-firmware)."),
        );
        return out;
    }
    let name = inspect.lines().find(|l| l.contains("node.description")).and_then(|l| l.split('"').nth(1)).unwrap_or("").to_string();
    if code != 0 || name.is_empty() || name.to_lowercase().contains("dummy") {
        out.push(
            Check::new("dev", "Звук: вывод", BAD, "нет устройства (Dummy Output)")
                .hint("Система не видит звуковую карту. Часто помогает перезапуск звука; если нет — не хватает драйвера (sof-firmware).")
                .fixes(vec![fx("sound_restart", "Перезапустить звук", None)]),
        );
        return out;
    }
    let (_, vol) = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"], 5);
    match parse_volume(&vol) {
        Some((_, true)) => {
            out.push(Check::new("dev", "Звук: вывод", WARN, &format!("{name} — выключен")).hint("Звук выключен (беззвучный режим).").fixes(vec![fx(
                "sound_unmute",
                "Включить звук",
                None,
            )]))
        }
        Some((v, false)) if v < 0.02 => {
            out.push(Check::new("dev", "Звук: вывод", WARN, &format!("{name} — громкость 0")).hint("Громкость на нуле.").fixes(vec![fx(
                "sound_volume",
                "Сделать 50%",
                None,
            )]))
        }
        Some((v, false)) => out.push(Check::new("dev", "Звук: вывод", OK, &format!("{name} · {:.0}%", v * 100.0))),
        None => out.push(Check::new("dev", "Звук: вывод", INFO, &name)),
    }
    let (_, mic) = run(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SOURCE@"], 5);
    if let Some((v, muted)) = parse_volume(&mic) {
        if muted {
            out.push(Check::new("dev", "Микрофон", INFO, "выключен").hint("Если тебя не слышат в звонках — включи.").fixes(vec![fx(
                "mic_unmute",
                "Включить микрофон",
                None,
            )]));
        } else {
            out.push(Check::new("dev", "Микрофон", OK, &format!("{:.0}%", v * 100.0)));
        }
    }
    out
}

// ───────────────────────────── Bluetooth и Wi-Fi ─────────────────────────────

/// Блокировки радио: (тип, программная, аппаратная) из /sys/class/rfkill.
fn rfkill() -> Vec<(String, bool, bool)> {
    let Ok(rd) = std::fs::read_dir("/sys/class/rfkill") else { return Vec::new() };
    rd.flatten()
        .map(|e| {
            let p = e.path();
            let r = |n: &str| read(&p.join(n).to_string_lossy());
            (r("type"), r("soft") == "1", r("hard") == "1")
        })
        .collect()
}

pub(crate) fn check_radio() -> Vec<Check> {
    let mut out = Vec::new();
    let rf = rfkill();
    let has_bt = Path::new("/sys/class/bluetooth").read_dir().map(|mut d| d.next().is_some()).unwrap_or(false);
    if has_bt {
        let blocked = rf.iter().filter(|r| r.0 == "bluetooth");
        let (soft, hard) = blocked.fold((false, false), |a, r| (a.0 || r.1, a.1 || r.2));
        let (_, svc) = run(&["systemctl", "is-active", "bluetooth"], 5);
        if hard {
            out.push(
                Check::new("dev", "Bluetooth", WARN, "выключен кнопкой")
                    .hint("Выключен переключателем на ноутбуке (или Fn-клавишей). Программно не включить."),
            );
        } else if soft {
            out.push(
                Check::new("dev", "Bluetooth", WARN, "заблокирован")
                    .hint("Bluetooth выключен программно (режим «в самолёте» или кнопкой в меню).")
                    .fixes(vec![fx("bt_unblock", "Включить Bluetooth", None)]),
            );
        } else if svc.trim() != "active" {
            out.push(
                Check::new("dev", "Bluetooth", BAD, "служба не работает")
                    .hint("Служба bluetooth остановлена — устройства не подключатся.")
                    .fixes(vec![fx("svc_restart", "Перезапустить", Some("bluetooth"))]),
            );
        } else {
            let (_, show) = run(&["bluetoothctl", "show"], 4);
            if show.contains("Powered: no") {
                out.push(Check::new("dev", "Bluetooth", WARN, "адаптер выключен").fixes(vec![fx("bt_power", "Включить адаптер", None)]));
            } else {
                out.push(Check::new("dev", "Bluetooth", OK, "включён"));
            }
        }
    }
    if let Some(w) = rf.iter().find(|r| r.0 == "wlan") {
        if w.2 {
            out.push(Check::new("net", "Wi-Fi", INFO, "выключен кнопкой").hint("Выключен переключателем на ноутбуке."));
        } else if w.1 {
            // у многих Wi-Fi выключен нарочно (сеть по кабелю) — это не поломка
            out.push(
                Check::new("net", "Wi-Fi", INFO, "выключен").hint("Wi-Fi выключен программно. Если сеть по кабелю — так и надо.").fixes(vec![fx(
                    "wifi_on",
                    "Включить Wi-Fi",
                    None,
                )]),
            );
        }
    }
    out
}

// ───────────────────────────── Печать ─────────────────────────────

pub(crate) fn check_printing() -> Vec<Check> {
    let mut out = Vec::new();
    if crate::procs::which("lpstat").is_none() {
        return out;
    }
    let (_, p) = run(&["lpstat", "-p"], 5);
    if p.contains("No destinations") || p.trim().is_empty() {
        return out; // принтеров нет — и показывать нечего
    }
    let (_, cups) = run(&["systemctl", "is-active", "cups"], 5);
    if cups.trim() != "active" {
        out.push(Check::new("dev", "Печать", BAD, "служба CUPS не работает").fixes(vec![fx("svc_restart", "Перезапустить", Some("cups"))]));
        return out;
    }
    let (_, jobs) = run(&["lpstat", "-o"], 5);
    out.extend(printing_rows(&p, &jobs, "printer_enable", "print_clear"));
    out
}

/// Принтеры из `lpstat -p` (вывод CUPS одинаков на Linux и macOS, запускать с C-локалью): (имя, остановлен ли).
/// Строки «printer Имя is idle. enabled since …» и «printer Имя disabled since …».
pub(crate) fn parse_lpstat_printers(p: &str) -> Vec<(String, bool)> {
    p.lines()
        .filter(|l| l.starts_with("printer "))
        .map(|l| (l.split_whitespace().nth(1).unwrap_or("").to_string(), l.contains("disabled")))
        .filter(|(n, _)| !n.is_empty())
        .collect()
}

/// Число заданий в `lpstat -o`.
pub(crate) fn count_lpstat_jobs(jobs: &str) -> usize {
    jobs.lines().filter(|l| !l.trim().is_empty()).count()
}

/// Строки «Принтер …» и «Очередь печати» по выводу CUPS; `enable_id` и `clear_id` — действия этой системы.
pub(crate) fn printing_rows(lpstat_p: &str, lpstat_o: &str, enable_id: &str, clear_id: &str) -> Vec<Check> {
    let mut out = Vec::new();
    for (name, disabled) in parse_lpstat_printers(lpstat_p) {
        if disabled {
            out.push(
                Check::new("dev", &format!("Принтер {name}"), WARN, "остановлен")
                    .hint("Принтер поставлен на паузу (часто после ошибки бумаги или связи).")
                    .fixes(vec![fx(enable_id, "Возобновить", Some(&name))]),
            );
        } else {
            out.push(Check::new("dev", &format!("Принтер {name}"), OK, "готов"));
        }
    }
    let n = count_lpstat_jobs(lpstat_o);
    if n > 0 {
        out.push(
            Check::new("dev", "Очередь печати", WARN, &format!("{n} в очереди"))
                .hint("Если задания не печатаются долго — очередь застряла.")
                .fixes(vec![fx(clear_id, "Очистить мою очередь", None)]),
        );
    }
    out
}

/// macOS: печать через CUPS — то же, что на Linux, но служба CUPS — `lpstat -r` («scheduler is running»), а не systemd.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub(crate) fn check_printing_mac() -> Vec<Check> {
    if crate::procs::which("lpstat").is_none() {
        return Vec::new();
    }
    let (_, p) = run(&["lpstat", "-p"], 8);
    if p.contains("No destinations") || p.trim().is_empty() {
        return Vec::new(); // принтеров нет — и показывать нечего
    }
    let (_, sched) = run(&["lpstat", "-r"], 5);
    if sched.contains("not running") {
        return vec![Check::new("dev", "Печать", BAD, "служба печати (CUPS) не работает")
            .hint("Без неё ничего не печатается. Помогает перезагрузка компьютера.")];
    }
    let (_, jobs) = run(&["lpstat", "-o"], 8);
    printing_rows(&p, &jobs, "mac_printer_enable", "mac_print_clear")
}

// ───────────────────────────── Раскладка ─────────────────────────────

pub(crate) fn check_keyboard() -> Vec<Check> {
    let mut out = Vec::new();
    if crate::procs::which("gsettings").is_none() {
        return out;
    }
    let (_, src) = run(&["gsettings", "get", "org.gnome.desktop.input-sources", "sources"], 5);
    let n = src.matches("('xkb'").count() + src.matches("('ibus'").count();
    let (_, keys) = run(&["gsettings", "get", "org.gnome.desktop.wm.keybindings", "switch-input-source"], 5);
    let (_, xkb) = run(&["gsettings", "get", "org.gnome.desktop.input-sources", "xkb-options"], 5);
    let has_grp = xkb.contains("grp:");
    let has_keys = keys.contains('<') || keys.contains("XF86");
    if n >= 2 && !has_keys && !has_grp {
        out.push(
            Check::new("dev", "Раскладка", WARN, &format!("{n} раскладки, но нечем переключать"))
                .hint("Сочетание для смены раскладки стёрлось.")
                .fixes(vec![fx("kb_switch", "Вернуть Super+Пробел", None)]),
        );
    } else if n >= 2 || has_grp {
        let how = if has_grp {
            let g = xkb.split('\'').find(|s| s.starts_with("grp:")).unwrap_or("").trim_start_matches("grp:");
            match g {
                "alt_shift_toggle" => "Alt+Shift".into(),
                "ctrl_shift_toggle" => "Ctrl+Shift".into(),
                "win_space_toggle" => "Super+Пробел".into(),
                "caps_toggle" => "Caps Lock".into(),
                "ctrl_alt_toggle" => "Ctrl+Alt".into(),
                "alt_space_toggle" => "Alt+Пробел".into(),
                other => other.trim_end_matches("_toggle").replace('_', "+"),
            }
        } else {
            "Super+Пробел".into()
        };
        // какая раскладка включена сейчас — полезнее, чем сочетание клавиш (его человек и так знает);
        // сочетание остаётся запасным ответом, когда система не говорит, что включено
        let value = match layouts_now(&src) {
            Some((cur, others)) if others.is_empty() => layout_name(&cur),
            Some((cur, others)) => format!("{} · ещё: {}", layout_name(&cur), others.iter().map(|c| layout_name(c)).collect::<Vec<_>>().join(", ")),
            None => format!("переключение: {how}"),
        };
        out.push(Check::new("dev", "Раскладка", OK, &value));
    }
    out
}

/// Раскладка по коду xkb — словом («ru» → «русская»); незнакомый код остаётся кодом.
fn layout_name(code: &str) -> String {
    match code {
        "us" | "gb" | "en" => "английская",
        "ru" => "русская",
        "ua" => "украинская",
        "by" => "белорусская",
        "de" => "немецкая",
        "fr" => "французская",
        "es" => "испанская",
        "it" => "итальянская",
        "pl" => "польская",
        "kz" => "казахская",
        "tr" => "турецкая",
        _ => return code.to_uppercase(),
    }
    .to_string()
}

/// Коды раскладок из ответа gsettings: «[('xkb', 'us'), ('xkb', 'ru+phonetic')]» → us, ru.
fn layout_codes(sources: &str) -> Vec<String> {
    sources
        .split("('xkb', '")
        .skip(1)
        .filter_map(|p| p.split('\'').next())
        .map(|c| c.split('+').next().unwrap_or(c).to_string())
        .filter(|c| !c.is_empty() && c.len() <= 8 && c.chars().all(|x| x.is_ascii_alphanumeric()))
        .collect()
}

/// Включённая сейчас раскладка и остальные (без повторов). None — узнать не вышло.
fn layouts_now(sources: &str) -> Option<(String, Vec<String>)> {
    let pick = |cur: String, all: Vec<String>| {
        let mut others: Vec<String> = Vec::new();
        for c in all {
            if c != cur && !others.contains(&c) {
                others.push(c);
            }
        }
        Some((cur, others))
    };
    // GNOME: список источников, а последний использованный — первый в mru-sources
    let all = layout_codes(sources);
    if !all.is_empty() {
        let (_, mru) = run(&["gsettings", "get", "org.gnome.desktop.input-sources", "mru-sources"], 5);
        let cur = layout_codes(&mru).into_iter().next().or_else(|| all.first().cloned())?;
        return pick(cur, all);
    }
    // X11 без источников GNOME: список — у setxkbmap, а «включена не первая» — индикатор «Group 2» у xset
    if crate::procs::which("setxkbmap").is_none() || crate::procs::which("xset").is_none() {
        return None;
    }
    let (_, q) = run(&["setxkbmap", "-query"], 5);
    let all = x11_layouts(&q);
    let (ok, x) = run(&["xset", "-q"], 5);
    let second = x.split("Group 2:").nth(1).map(|t| t.trim_start().starts_with("on"));
    let cur = match (ok == 0, second) {
        (true, Some(false)) => all.first().cloned()?,
        // включена не первая: какая из остальных — индикатор не говорит; берём первую отличную от первой
        (true, Some(true)) => all.iter().find(|c| Some(*c) != all.first()).cloned().or_else(|| all.first().cloned())?,
        _ => return None,
    };
    pick(cur, all)
}

/// Строка «layout:     us,ru» из ответа setxkbmap -query → us, ru.
fn x11_layouts(query: &str) -> Vec<String> {
    query
        .lines()
        .find_map(|l| l.strip_prefix("layout:"))
        .map(|v| {
            v.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty() && c.len() <= 8 && c.chars().all(|x| x.is_ascii_alphanumeric()))
                .collect()
        })
        .unwrap_or_default()
}

// ───────────────────────────── Пакеты, /boot ─────────────────────────────

pub(crate) fn check_packages() -> Vec<Check> {
    let mut out = Vec::new();
    if !Path::new("/var/lib/dpkg").is_dir() {
        return out; // не Debian/Ubuntu
    }
    // идёт установка прямо сейчас (apt, dpkg, обновления, Центр приложений) — файлы в updates/ это норма,
    // и «доделать установку» поверх идущей нельзя
    const INSTALLERS: [&str; 7] = ["dpkg", "apt", "apt-get", "aptitude", "unattended-upgr", "packagekitd", "synaptic"];
    let mut busy: Vec<String> = crate::procs::list().iter().filter(|p| INSTALLERS.iter().any(|i| p.name == *i)).map(|p| p.name.clone()).collect();
    // надёжнее имён: кто-то держит замок dpkg (lock-frontend или lock) — значит, установка идёт, кто бы её
    // ни вёл (apt.systemd.daily, Центр приложений, aptd). /proc/locks читается без прав
    if busy.is_empty() && dpkg_locked() {
        busy.push("установщик пакетов".into());
    }
    let pending = std::fs::read_dir("/var/lib/dpkg/updates").map(|d| d.flatten().next().is_some()).unwrap_or(false);
    let interrupted = pending && busy.is_empty();
    if pending && !busy.is_empty() {
        out.push(Check::new("sys", "Установка пакетов", INFO, "идёт сейчас").hint(&format!("Работает {}. Подожди, пока закончит.", busy[0])));
    } else if interrupted {
        out.push(
            Check::new("sys", "Установка пакетов", BAD, "прервана")
                .hint("Установка обновлений оборвалась на середине — пока не доделать, apt ничего не поставит.")
                .fixes(vec![fx("dpkg_configure", "Доделать установку", None)]),
        );
    } else {
        let aud = crate::diag::run_ex(&["dpkg", "--audit"], 20);
        let audit = aud.text().to_string();
        if !aud.answered() || (aud.code() != 0 && audit.trim().is_empty()) {
            out.push(Check::new("sys", "Пакеты", INFO, "не удалось проверить").unknown().hint("dpkg --audit не ответил или завершился с ошибкой."));
        } else if !audit.trim().is_empty() {
            out.push(
                Check::new("sys", "Пакеты", WARN, "есть недоустановленные")
                    .hint("dpkg --audit нашёл пакеты в непонятном состоянии.")
                    .fixes(vec![fx("apt_fix", "Починить зависимости", None)]),
            );
        } else {
            out.push(Check::new("sys", "Пакеты", OK, "в порядке"));
        }
    }
    if Path::new("/var/run/reboot-required").exists() {
        let pk = read("/var/run/reboot-required.pkgs");
        let what = pk.lines().take(3).collect::<Vec<_>>().join(", ");
        out.push(Check::new("sys", "Перезагрузка", INFO, "нужна после обновлений").hint(&if what.is_empty() {
            "Обновления вступят в силу после перезагрузки.".to_string()
        } else {
            format!("Ждут перезагрузки: {what}.")
        }));
    }
    // /boot отдельным разделом переполняется старыми ядрами
    if is_mountpoint("/boot") {
        if let Some((free, total)) = fs_space("/boot") {
            let used = 1.0 - free as f64 / total.max(1) as f64;
            let v = format!("занято {:.0}%, свободно {} МБ", used * 100.0, free / 1_048_576);
            if used > 0.9 {
                out.push(Check::new("sys", "Раздел /boot", BAD, &v).hint("Старые ядра заняли место — обновление ядра упадёт.").fixes(vec![fx(
                    "apt_autoremove",
                    "Убрать старые ядра",
                    None,
                )]));
            } else if used > 0.8 {
                out.push(Check::new("sys", "Раздел /boot", WARN, &v).fixes(vec![fx("apt_autoremove", "Убрать старые ядра", None)]));
            }
        }
    }
    out
}

fn is_mountpoint(p: &str) -> bool {
    read("/proc/self/mounts").lines().any(|l| l.split_whitespace().nth(1) == Some(p))
}

/// (свободно, всего) в байтах — у ядра (statvfs), с ограничением по времени.
fn fs_space(p: &str) -> Option<(u64, u64)> {
    let all = crate::disks::list();
    crate::disks::for_path(&all, p).map(|f| (f.avail as u64, f.size as u64))
}

// ───────────────────────────── Место: корзина, миниатюры, журнал ─────────────────────────────

/// Размер папки с ограничением по времени (большие папки не держат проверку).
fn dir_size(p: &Path, budget: std::time::Instant) -> u64 {
    let mut total = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        if budget.elapsed() > Duration::from_secs(2) {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            // ссылку не разворачиваем: папка-ссылка на весь диск посчиталась бы целиком (и по кругу)
            let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

fn gb(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.1} ГБ", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{} МБ", b >> 20)
    }
}

pub(crate) fn check_cleanup() -> Vec<Check> {
    let mut out = Vec::new();
    let h = home();
    // считаем то же, что очистит кнопка: gio trash --empty чистит Корзину в домашней папке и на всех
    // подключённых дисках (.Trash-<uid>, .Trash/<uid>)
    let uid = run(&["id", "-u"], 5).1.trim().to_string();
    let mut bins = vec![std::path::PathBuf::from(&h).join(".local/share/Trash")];
    for fs in crate::disks::list() {
        bins.push(Path::new(&fs.target).join(format!(".Trash-{uid}")));
        bins.push(Path::new(&fs.target).join(".Trash").join(&uid));
    }
    let t = std::time::Instant::now();
    let trash: u64 = bins.iter().filter(|b| b.is_dir()).map(|b| dir_size(b, t)).sum();
    let partial = t.elapsed() > Duration::from_secs(2);
    let disks_n = bins.iter().filter(|b| b.is_dir()).count();
    if trash > 1 << 30 {
        let v = if partial { format!("больше {}", gb(trash)) } else { gb(trash) };
        out.push(
            Check::new("sys", "Корзина", INFO, &v)
                .hint(&format!(
                    "Удалённые файлы всё ещё занимают место{}. Очистка удалит их насовсем{}.",
                    if partial { " (считал 2 секунды, на деле больше)" } else { "" },
                    if disks_n > 1 {
                        format!(" — с этого компьютера и подключённых дисков ({disks_n} корзины)")
                    } else {
                        String::new()
                    }
                ))
                .fixes(vec![fx("trash_empty", "Очистить корзину", None)]),
        );
    }
    let t = std::time::Instant::now();
    let thumbs_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| Path::new(&h).join(".cache"))
        .join("thumbnails");
    let thumbs = dir_size(&thumbs_dir, t);
    if thumbs > 500 << 20 {
        out.push(Check::new("sys", "Кэш миниатюр", INFO, &gb(thumbs)).hint("Картинки-превью файлов. Пересоздадутся сами.").fixes(vec![fx(
            "thumbs_clear",
            "Очистить",
            None,
        )]));
    }
    let (_, j) = run(&["journalctl", "--disk-usage"], 8);
    // «Archived and active journals take up 2.2G in the file system.»
    if let Some(sz) = j.split("take up ").nth(1).and_then(|r| r.split_whitespace().next()) {
        let bytes = parse_size(sz);
        if bytes > 1 << 30 {
            out.push(
                Check::new("sys", "Системный журнал", INFO, &gb(bytes))
                    .hint("Старые записи журнала. Можно оставить последние 200 МБ.")
                    .fixes(vec![fx("journal_vacuum", "Сжать до 200 МБ", None)]),
            );
        }
    }
    out
}

/// «2.2G», «512.0M», «80K» → байты.
fn parse_size(s: &str) -> u64 {
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let n: f64 = num.parse().unwrap_or(0.0);
    let k: f64 = match unit.chars().next() {
        Some('K') => 1024.0,
        Some('M') => 1048576.0,
        Some('G') => 1073741824.0,
        Some('T') => 1099511627776.0,
        _ => 1.0,
    };
    (n * k) as u64
}

// ───────────────────────────── Память, процессы ─────────────────────────────

/// Держит ли кто-нибудь замок dpkg: номер файла (inode) замка есть в /proc/locks.
fn dpkg_locked() -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let locks = read("/proc/locks");
        ["/var/lib/dpkg/lock-frontend", "/var/lib/dpkg/lock"].iter().filter_map(|p| std::fs::metadata(p).ok()).any(|m| {
            // «1: POSIX ADVISORY WRITE 1234 08:02:1311 0 EOF» — устройство:inode в 6-м поле
            let ino = m.ino().to_string();
            locks.lines().any(|l| l.split_whitespace().nth(5).and_then(|f| f.rsplit(':').next()) == Some(ino.as_str()))
        })
    }
    #[cfg(not(unix))]
    false
}

pub(crate) fn check_memory() -> Vec<Check> {
    let mut out = Vec::new();
    // журнал ядра читают только root и группы adm, systemd-journal, wheel; без них journalctl молча покажет
    // пусто — и «нехватки памяти не было» было бы неправдой
    let groups = run(&["id", "-nG"], 5).1;
    let can_read = run(&["id", "-u"], 5).1.trim() == "0" || groups.split_whitespace().any(|g| matches!(g, "adm" | "systemd-journal" | "wheel"));
    if !can_read {
        out.push(
            Check::new("sys", "Нехватка памяти", INFO, "не проверить")
                .unknown()
                .hint("Журнал системы закрыт для этого пользователя (нужна группа adm или systemd-journal)."),
        );
    }
    // journalctl -g (поиск) есть не во всех сборках; ошибка — «не проверить», а не «нехватки не было»
    let kr = crate::diag::run_ex(&["journalctl", "-k", "--since", "-24h", "-q", "--no-pager", "-g", "Out of memory|oom-kill|Killed process"], 8);
    if !kr.answered() || kr.code() != 0 && kr.code() != 1 {
        out.push(Check::new("sys", "Нехватка памяти", INFO, "не проверить").unknown().hint("Журнал системы не прочитался."));
        return out;
    }
    let k = kr.text().to_string();
    // systemd-oomd закрывает программы раньше ядра и пишет в общий журнал
    let (_, u) = run(&["journalctl", "-u", "systemd-oomd", "--since", "-24h", "-q", "--no-pager", "-g", "Killed"], 8);
    let k = format!("{k}\n{u}");
    // «Killed process 1234 (firefox) total-vm…» — ядро; «Killed /…/app-firefox.scope …» — systemd-oomd.
    // Считаем события по номеру процесса (одна строка может повториться), имя — из скобок именно этой фразы
    let mut seen: Vec<String> = Vec::new();
    let mut who: Vec<String> = Vec::new();
    for l in k.lines() {
        let (id, name) = if let Some(r) = l.split("Killed process ").nth(1) {
            let pid = r.split_whitespace().next().unwrap_or("").to_string();
            let name = r.split_once('(').and_then(|(_, x)| x.split_once(')')).map(|(n, _)| n.to_string()).unwrap_or_default();
            (pid, name)
        } else if l.contains("systemd-oomd") && l.contains("Killed ") {
            let unit = l.split("Killed ").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("").to_string();
            let name = unit.rsplit('/').next().unwrap_or("").trim_end_matches(".scope").trim_start_matches("app-").to_string();
            (unit, name)
        } else {
            continue;
        };
        if id.is_empty() || seen.contains(&id) {
            continue;
        }
        seen.push(id);
        if !name.is_empty() && name != "null" && !who.contains(&name) && who.len() < 3 {
            who.push(name);
        }
    }
    let n = seen.len();
    if n > 0 {
        out.push(Check::new("sys", "Нехватка памяти", WARN, &format!("{} за сутки", crate::diag::plural(n, "раз", "раза", "раз"))).hint(&format!(
            "Система закрывала программы, чтобы не зависнуть{}.",
            if who.is_empty() { String::new() } else { format!(": {}", who.join(", ")) }
        )));
    }
    let swap = read("/proc/swaps").lines().count() > 1;
    if !swap {
        out.push(Check::new("sys", "Подкачка", INFO, "нет").hint("Без подкачки при нехватке памяти программы закрываются сразу."));
    }
    out
}

/// Время процессора процесса (utime+stime, тики) из /proc/<pid>/stat.
fn cpu_ticks(pid: i64) -> Option<u64> {
    let s = read(&format!("/proc/{pid}/stat"));
    let rest = s.rsplit(')').next()?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

/// Замеры «горячих» процессов между проверками: (номер, время запуска) → (тики процессора, когда).
static HOT_SEEN: std::sync::Mutex<Vec<((i64, u64), (u64, std::time::Instant))>> = std::sync::Mutex::new(Vec::new());

/// Процесс занят долго, а не секунду: по замеру из прошлой проверки (≥ 50 с назад) — в среднем за это время
/// больше половины ядра. Средняя за всю жизнь не годится: браузер, открытый сутки, завис на час — его средняя
/// всё равно 5%. Впервые замеченный горячий процесс запоминаем и судим в следующий раз.
fn sustained(pid: i64, hz: f64) -> bool {
    let (Some(start), Some(cpu)) = (crate::procs::start_ticks(pid), cpu_ticks(pid)) else { return false };
    let key = (pid, start);
    let now = std::time::Instant::now();
    let mut seen = HOT_SEEN.lock().unwrap_or_else(|e| e.into_inner());
    // старые замеры (больше 15 минут) — забыть
    seen.retain(|(_, (_, t))| t.elapsed() < Duration::from_secs(900));
    let verdict = match seen.iter().find(|(k, _)| *k == key) {
        Some((_, (c0, t0))) if t0.elapsed() >= Duration::from_secs(50) => {
            Some(cpu.saturating_sub(*c0) as f64 / hz / t0.elapsed().as_secs_f64() > 0.5)
        }
        Some(_) => None, // слишком недавно — ждём следующей проверки
        None => {
            seen.push((key, (cpu, now)));
            None
        }
    };
    if verdict.is_some() {
        // новая точка отсчёта для следующего окна
        seen.retain(|(k, _)| *k != key);
        seen.push((key, (cpu, now)));
    }
    verdict.unwrap_or(false)
}

pub(crate) fn check_hogs() -> Vec<Check> {
    let mut out = Vec::new();
    let me = std::process::id() as i64;
    let all = crate::procs::list();
    let a: Vec<(crate::procs::Proc, u64)> = all.iter().filter(|p| p.pid != me).filter_map(|p| cpu_ticks(p.pid).map(|t| (p.clone(), t))).collect();
    std::thread::sleep(Duration::from_millis(1500));
    let hz = crate::procs::clk_tck(); // тиков в секунду — у системы
    let mut hot: Vec<(crate::procs::Proc, f64)> = a
        .into_iter()
        .filter_map(|(p, t0)| cpu_ticks(p.pid).map(|t1| (p, (t1.saturating_sub(t0)) as f64 / hz / 1.5 * 100.0)))
        .filter(|(_, pct)| *pct > 80.0)
        // не секундный всплеск (браузер открыл страницу, сборка запустилась), а долгая работа: процесс живёт
        // больше 2 минут и за всю жизнь в среднем занимал больше половины ядра
        .filter(|(p, _)| sustained(p.pid, hz))
        .collect();
    hot.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    // индексаторы — отдельно: их не убивают, а объясняют
    const INDEXERS: [&str; 3] = ["tracker-miner-f", "baloo_file", "localsearch-3"];
    for (p, pct) in hot.iter().take(3) {
        // номер процесса и время его запуска: кнопка не закроет другой процесс, получивший тот же номер
        let pid = match crate::procs::start_ticks(p.pid) {
            Some(t) => format!("{}@{t}", p.pid),
            None => p.pid.to_string(),
        };
        if crate::procs::is_protected(&p.name) {
            // системный процесс (оболочка рабочего стола, звук, сеть…): не закрываем, только говорим
            out.push(Check::new("sys", &format!("Занят процессор: {}", p.name), WARN, &format!("{pct:.0}% ядра"))
                .hint("Это часть системы, её закрывать нельзя. Обычно её нагружает какая-то программа или расширение; если тормозит долго — перезайди в систему."));
        } else if INDEXERS.iter().any(|i| p.name.starts_with(i)) {
            out.push(
                Check::new("sys", &format!("Индексатор {}", p.name), INFO, &format!("{pct:.0}% процессора"))
                    .hint("Составляет поиск по файлам. Обычно затихает сам через полчаса."),
            );
        } else {
            out.push(hog_check(&p.name, *pct, crate::procs::hog_kind(p, &all), Some(fx("proc_term", "Закрыть программу", Some(&pid)))));
        }
    }
    out
}

/// Строка «кто грузит процессор» — спокойно и по делу: виртуальная машина и работа ИИ-помощника —
/// не беда и без кнопки «Закрыть» (новичок закрыл бы то, что ему же нужно); сборка и скрипты — пояснение;
/// прочее — «если нужна, всё нормально; если зависла — можно закрыть».
pub(crate) fn hog_check(name: &str, pct: f64, kind: crate::procs::Hog, close: Option<crate::diag::Fix>) -> Check {
    use crate::procs::Hog;
    let load = format!("{pct:.0}% ядра");
    match kind {
        Hog::Vm => Check::new("sys", "Работает виртуальная машина", INFO, &load).hint(&format!(
            "Это виртуальный компьютер ({name}). Он занят тем, что в нём запущено. Если его включил ты или твой ИИ-помощник — всё в порядке, закрывать не нужно."
        )),
        Hog::Agent(who) => Check::new("sys", &format!("{who} работает: {name}"), INFO, &load).hint(&format!(
            "Эту программу запустил {who} — он что-то проверяет, собирает или считает. Это нормально и обычно скоро заканчивается. Закрывать не нужно."
        )),
        Hog::Build => Check::new("sys", &format!("Занят процессор: {name}"), INFO, &load).hint(
            "Похоже, идёт сборка программы или работает скрипт. Если это запустил ты или твой ИИ-помощник — подожди, это пройдёт. Если не знаешь, что это, и всё тормозит — можно закрыть.",
        ).fixes(close.into_iter().collect()),
        Hog::Other => Check::new("sys", &format!("Занят процессор: {name}"), WARN, &load).hint(
            "Программа сейчас много считает. Если она тебе нужна (игра, видео, сборка) — всё нормально. Если она зависла или ты её не запускал — можно закрыть.",
        ).fixes(close.into_iter().collect()),
    }
}

// ───────────────────────────── Secure Boot и NVIDIA ─────────────────────────────

pub(crate) fn check_secureboot() -> Vec<Check> {
    let mut out = Vec::new();
    // драйвер NVIDIA установлен (есть модуль в системе), но не загружен
    // карты NVIDIA нет (драйвер остался от прошлой) или режим «всегда встроенная» (prime-select intel —
    // драйвер не загружен нарочно) — сказать нечего; «не загружен» без Secure Boot говорит проверка видеокарты
    if !crate::diag::pci_gpus().iter().any(|(v, _)| v == "10de") {
        return out;
    }
    let (code, _) = run(&["modinfo", "-F", "version", "nvidia"], 5);
    if code != 0 || Path::new("/sys/module/nvidia").exists() {
        return out;
    }
    if run(&["prime-select", "query"], 5).1.trim() == "intel" {
        return out;
    }
    let (_, sb) = run(&["mokutil", "--sb-state"], 5);
    if sb.contains("SecureBoot enabled") {
        out.push(Check::new("gpu", "Secure Boot и NVIDIA", BAD, "драйвер не загружен")
            .hint("Secure Boot не пускает неподписанный драйвер NVIDIA — экран рисует процессор. Нужно зарегистрировать ключ (MOK) с перезагрузкой или выключить Secure Boot в BIOS."));
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn layout_codes_names_and_x11_list() {
        assert_eq!(super::layout_codes("[('xkb', 'us'), ('xkb', 'ru+phonetic'), ('ibus', 'mozc')]"), ["us", "ru"]);
        assert!(super::layout_codes("@a(ss) []").is_empty());
        assert!(super::layout_codes("[('xkb', 'u$(rm)')]").is_empty());
        assert_eq!(super::x11_layouts("rules:      evdev\nlayout:     us,ru,ru\nvariant:    ,,\n"), ["us", "ru", "ru"]);
        assert_eq!(
            (super::layout_name("ru"), super::layout_name("us"), super::layout_name("zz")),
            ("русская".into(), "английская".into(), "ZZ".into())
        );
    }

    use super::*;

    #[test]
    fn volume_parse() {
        assert_eq!(parse_volume("Volume: 0.40 [MUTED]"), Some((0.40, true)));
        assert_eq!(parse_volume("Volume: 1.00"), Some((1.0, false)));
        assert_eq!(parse_volume("ничего"), None);
    }

    #[test]
    fn cups_output_parse() {
        // вывод lpstat на C-локали (Linux и macOS одинаковый)
        let p = "printer HP_M125 is idle.  enabled since Mon Nov  9 10:00:00 2026\nprinter Brother_X disabled since Tue Nov 10 09:00:00 2026 -\n\treason unknown\n";
        assert_eq!(parse_lpstat_printers(p), vec![("HP_M125".to_string(), false), ("Brother_X".to_string(), true)]);
        assert!(parse_lpstat_printers("lpstat: No destinations added.\n").is_empty());
        assert_eq!(count_lpstat_jobs("HP_M125-12 ulyana 1024 Mon Nov  9 10:05:00 2026\n\n"), 1);
        let rows = printing_rows(p, "HP_M125-12 u 1 Mon\nHP_M125-13 u 1 Mon\n", "mac_printer_enable", "mac_print_clear");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].fixes[0].id, "mac_printer_enable");
        assert_eq!(rows[1].fixes[0].arg.as_deref(), Some("Brother_X"));
        assert_eq!(
            (rows[2].title.as_str(), rows[2].value.as_str(), rows[2].fixes[0].id.as_str()),
            ("Очередь печати", "2 в очереди", "mac_print_clear")
        );
    }

    #[test]
    fn size_parse() {
        assert_eq!(parse_size("2.0G"), 2 << 30);
        assert_eq!(parse_size("512M"), 512 << 20);
        assert_eq!(parse_size("80K"), 80 << 10);
    }
}
