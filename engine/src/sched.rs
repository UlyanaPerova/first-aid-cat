//! Расписание проверок: у каждой своя частота, проверка по событию, один общий результат на окно и виджет.
//!
//! Раньше всё проверялось сразу раз в минуту — и дважды (окно и виджет по отдельности): десятки обращений
//! в интернет в минуту, на Windows несколько PowerShell за раз (нагрев). Теперь:
//! - частота по смыслу (every): сеть и VPN — 30 с, устройства и ИИ — 2 мин, часы, пакеты, обновления — 30 мин;
//!   пока результат не меняется — реже (до ×4), поменялся — снова как обычно;
//! - событие: сменились адрес/маршрут — сетевые проверки сразу; компьютер проснулся — всё сразу;
//! - «как часто проверять» в настройках (ui.json check_freq: rare ×3 / normal / often ×0,5);
//! - проверяет один процесс (у кого замок checks.lock), результат — в checks.json; второй (окно или виджет)
//!   его читает; закрылся проверяющий — замок свободен, второй подхватывает;
//! - проверяющий раз в 5 с отмечается в checks.alive; завис (остановлен, но жив и держит замок) — второй
//!   через минуту начинает проверять сам, а когда тот оживёт — уступает ему;
//! - зависшая проверка через 5 минут запускается заново (не больше 3 раз), поздний ответ старой — не берём.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};

use crate::catalog::Unit;
use crate::diag::{Check, Fix};

/// Как часто проверять показатель (обычная частота, секунды).
fn base_every(id: &str) -> u64 {
    match id {
        // сеть, VPN, прокси, загрузка процессора и диск — то, что ломается «прямо сейчас»
        "network" | "proxy" | "vpn" | "hogs" | "system" | "dock" | "apps" | "disk" => 30,
        // лёгкий замер задержки — раз в 15 минут
        "latency" => 900,
        // часы, пакеты, обновления, очистка, журнал памяти, версия системы — меняются редко, проверять дорого
        "clock" | "repos" | "packages" | "cleanup" | "memory" | "updates" | "stability" | "win_os" | "mac_os" | "secureboot" | "dupes"
        | "diskhealth" | "footprint" => 1800,
        // устройства, видеокарта, темы, ИИ-помощники и прочие прокси
        _ => 120,
    }
}

/// Сетевые показатели: проверяются сразу, когда сменилась сеть.
fn is_net(u: &Unit) -> bool {
    u.cat == crate::catalog::Cat::Net
}

/// Почему включён режим экономии.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Eco {
    Battery,
    Metered,
}

/// Нужна ли экономия: на батарее или в лимитной сети — как «редко», если человек сам не выбрал «часто».
/// Выбрал «редко» — и так редко, экономить дальше нечем (None, чтобы не писать лишнего в журнал).
fn economy(on_battery: bool, metered: bool, choice: Option<&str>) -> Option<Eco> {
    if matches!(choice, Some("often") | Some("rare")) {
        return None;
    }
    if on_battery {
        Some(Eco::Battery)
    } else if metered {
        Some(Eco::Metered)
    } else {
        None
    }
}

/// Множитель частоты: выбор в настройках; на батарее и в лимитной сети (если не выбрано «часто») — как «редко».
fn factor_for(choice: Option<&str>, eco: Option<Eco>) -> f64 {
    match choice {
        Some("rare") => 3.0,
        Some("often") => 0.5,
        _ if eco.is_some() => 3.0,
        _ => 1.0,
    }
}

/// Батарея и лимитная сеть, кэш на минуту: опрос недорогой, но не каждый проход цикла.
fn power_state() -> (bool, bool) {
    static CACHE: std::sync::Mutex<Option<(Instant, bool, bool)>> = std::sync::Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, b, m)) = *c {
        if t.elapsed() < Duration::from_secs(60) {
            return (b, m);
        }
    }
    let (b, m) = (crate::netproxy::on_battery(), crate::diag::is_metered());
    *c = Some((Instant::now(), b, m));
    (b, m)
}

/// Ждущая запись в журнал окна о смене режима: 0 нет, 1 батарея, 2 лимитная сеть, 3 обычный режим.
static ECO_NOTE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// Прежний режим: 0 обычный, 1 батарея, 2 лимитная сеть.
static ECO_NOW: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Запись в журнал окна о смене режима экономии (забирает окно; None — нечего сказать).
pub fn take_eco_note() -> Option<&'static str> {
    use std::sync::atomic::Ordering;
    match ECO_NOTE.swap(0, Ordering::SeqCst) {
        1 => Some("На батарее — проверяю реже."),
        2 => Some("Сеть лимитная — проверяю реже."),
        3 => Some("Питание и сеть обычные — проверяю как обычно."),
        _ => None,
    }
}

/// Множитель частоты из настроек: rare ×3, often ×0,5; плюс автоматическая экономия.
fn freq_factor() -> f64 {
    use std::sync::atomic::Ordering;
    let prefs = crate::prefs::load();
    let choice = prefs.get("check_freq").and_then(|v| v.as_str());
    // опрашиваем батарею и сеть, только если выбор не «часто»/«редко»: иначе это не повлияет
    let eco = if matches!(choice, Some("often") | Some("rare")) {
        None
    } else {
        let (b, m) = power_state();
        economy(b, m, choice)
    };
    let code = match eco {
        None => 0,
        Some(Eco::Battery) => 1,
        Some(Eco::Metered) => 2,
    };
    let prev = ECO_NOW.swap(code, Ordering::SeqCst);
    if prev != code {
        ECO_NOTE.store(if code == 0 { 3 } else { code }, Ordering::SeqCst);
        match code {
            1 => crate::diag::log("проверки: на батарее — проверяю реже"),
            2 => crate::diag::log("проверки: сеть лимитная — проверяю реже"),
            _ => crate::diag::log("проверки: обычный режим"),
        }
    }
    factor_for(choice, eco)
}

/// Попросить проверить всё сейчас (кнопка «Проверить всё»): у себя или у проверяющего процесса.
pub fn check_now() {
    NOW.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = crate::store::write_private(&req_path(), crate::ui::now_secs().to_string().as_bytes());
}

static NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dir() -> std::path::PathBuf {
    crate::store::private_dir()
}
fn res_path() -> std::path::PathBuf {
    dir().join("checks.json")
}
fn alive_path() -> std::path::PathBuf {
    dir().join("checks.alive")
}
fn req_path() -> std::path::PathBuf {
    dir().join("check_now")
}

/// Запустить: результаты (все строки по порядку каталога) приходят в канал при каждом изменении.
pub fn start() -> Receiver<Vec<Check>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            let lock = crate::store::lock::try_take(&dir().join("checks.lock"));
            // перехват у зависшего — только одному: у кого замок перехвата (иначе две копии писали бы общий
            // файл наперегонки)
            let takeover = if lock.is_none() && producer_stale() { crate::store::lock::try_take(&dir().join("takeover.lock")) } else { None };
            let stale = takeover.is_some();
            if lock.is_some() || stale {
                crate::diag::log(if stale {
                    "проверки: проверяющий процесс завис — проверяю сам"
                } else {
                    "проверки: этот процесс проверяет (расписание)"
                });
                let r = produce(&tx, lock);
                drop(takeover);
                match r {
                    Produce::Closed => return,
                    Produce::Yield => continue, // зависший ожил — снова читаем его результат
                }
            }
            // проверяет другой процесс — читаем его результат, пока он жив
            if !consume(&tx) {
                return;
            }
        }
    });
    rx
}

// ───────────────────────────── читатель ─────────────────────────────

/// Последняя отметка проверяющего: (номер процесса, время). Пишется раз в 5 с.
fn heartbeat() -> Option<(u32, i64)> {
    let t = std::fs::read_to_string(alive_path()).ok()?;
    let (pid, at) = t.trim().split_once(' ')?;
    Some((pid.parse().ok()?, at.parse().ok()?))
}

/// Проверяющий не отмечался больше минуты (остановлен, завис) — хотя замок держит.
fn producer_stale() -> bool {
    // отметка из «будущего» (часы перевели назад) — тоже не верим ей: иначе зависший казался бы живым часами
    heartbeat().is_none_or(|(_, at)| {
        let age = crate::ui::now_secs() - at;
        !(-60..=60).contains(&age)
    })
}

/// Читать checks.json, пока замок занят; false — получатель закрыт. Возвращается, когда замок можно взять
/// или проверяющий завис.
fn consume(tx: &Sender<Vec<Check>>) -> bool {
    let mut seen: Option<SystemTime> = None;
    let mut probe = Instant::now();
    loop {
        if let Ok(m) = std::fs::metadata(res_path()).and_then(|m| m.modified()) {
            if seen != Some(m) {
                seen = Some(m);
                if let Some(c) = read_results() {
                    if tx.send(c).is_err() {
                        return false;
                    }
                }
            }
        }
        // раз в 10 с: не освободился ли замок (проверяющий процесс закрылся)
        if probe.elapsed() > Duration::from_secs(10) {
            probe = Instant::now();
            if crate::store::lock::free(&dir().join("checks.lock")) || producer_stale() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

// ───────────────────────────── проверяющий ─────────────────────────────

struct Slot {
    rows: Vec<Check>,
    /// когда пора
    due: Instant,
    /// текущий интервал (растёт, пока ничего не меняется)
    every: f64,
    /// сводка состояний прошлого результата (для «ничего не изменилось»)
    sig: String,
    running: bool,
    started: Instant,
    /// номер запуска: ответ брошенной (зависшей) проверки приходит со старым номером и не берётся
    gen: u64,
    /// сколько раз бросали зависшую
    abandoned: u32,
}

enum Produce {
    /// окно/виджет закрылись
    Closed,
    /// проверяли без замка (прежний проверяющий завис), а он ожил — уступили
    Yield,
}

/// Через сколько бросить зависшую проверку и запустить её заново.
const STUCK: Duration = Duration::from_secs(300);

fn produce(tx: &Sender<Vec<Check>>, mut lock: Option<crate::store::lock::Held>) -> Produce {
    let (done_tx, done_rx) = mpsc::channel::<(&'static str, u64, Vec<Check>)>();
    let me = std::process::id();
    let mut beat_t = Instant::now() - Duration::from_secs(60);
    let begun = Instant::now();
    let mut dirty = false;
    let mut slots: HashMap<&'static str, Slot> = HashMap::new();
    let mut last_wall = SystemTime::now();
    let mut last_mono = Instant::now();
    let mut net_sig = net_signature();
    let mut dev_sig = dev_signature();
    let mut net_t = Instant::now();
    let mut req_seen = std::fs::read_to_string(req_path()).unwrap_or_default();
    let mut factor = freq_factor();
    let mut factor_t = Instant::now();
    loop {
        let now = Instant::now();
        // отметка «жив» раз в 5 с; без замка — следим, не ожил ли прежний проверяющий
        if beat_t.elapsed() >= Duration::from_secs(5) {
            beat_t = now;
            if lock.is_none() {
                lock = crate::store::lock::try_take(&dir().join("checks.lock"));
                if lock.is_none() && heartbeat().is_some_and(|(pid, at)| pid != me && crate::ui::now_secs() - at < 10) {
                    crate::diag::log("проверки: прежний проверяющий ожил — уступаю ему");
                    return Produce::Yield;
                }
            }
            let _ = crate::store::write_private_quick(&alive_path(), format!("{me} {}", crate::ui::now_secs()).as_bytes());
        }
        // компьютер спал: часы ушли вперёд больше, чем прошло у нас, — проверить всё (сеть — через 5 с, ей
        // нужно время подняться)
        let wall = SystemTime::now();
        let wall_gap = wall.duration_since(last_wall).unwrap_or_default();
        let mono_gap = now.duration_since(last_mono);
        if wall_gap > Duration::from_secs(20) || mono_gap > Duration::from_secs(20) {
            crate::diag::log("проверки: компьютер проснулся — проверяю всё");
            for (id, s) in slots.iter_mut() {
                s.due = now + if is_net_id(id) { Duration::from_secs(5) } else { Duration::ZERO };
                s.every = base_every(id) as f64 * factor;
            }
        }
        last_wall = wall;
        last_mono = now;
        // сеть сменилась (другой адрес или маршрут) — сетевые проверки сразу
        if net_t.elapsed() > Duration::from_secs(3) {
            net_t = Instant::now();
            let sig = net_signature();
            if sig != net_sig {
                crate::diag::log(&format!("проверки: сеть сменилась ({net_sig} → {sig}) — проверяю сеть"));
                net_sig = sig;
                for (id, s) in slots.iter_mut() {
                    if is_net_id(id) {
                        s.due = now + Duration::from_secs(2);
                        s.every = base_every(id) as f64 * factor;
                    }
                }
            }
        }
        // устройства: подключили/отключили звуковую карту, USB, питание — проверить устройства сразу
        if net_t.elapsed() < Duration::from_millis(600) {
            let sig = dev_signature();
            if sig != dev_sig {
                crate::diag::log("проверки: устройства поменялись — проверяю устройства и звук");
                dev_sig = sig;
                for (id, s) in slots.iter_mut() {
                    if is_dev_id(id) {
                        s.due = now + Duration::from_secs(2);
                        s.every = base_every(id) as f64 * factor;
                    }
                }
            }
        }
        // «Проверить всё» — из этого процесса или от другого (файл-запрос)
        let req = std::fs::read_to_string(req_path()).unwrap_or_default();
        if NOW.swap(false, std::sync::atomic::Ordering::SeqCst) || req != req_seen {
            req_seen = req;
            for (id, s) in slots.iter_mut() {
                s.due = now;
                s.every = base_every(id) as f64 * factor;
            }
        }
        // настройка частоты могла поменяться
        if factor_t.elapsed() > Duration::from_secs(10) {
            factor_t = Instant::now();
            factor = freq_factor();
        }
        // какие показатели включены (могли включить/выключить в «Показателях»)
        let units: Vec<&'static Unit> = crate::catalog::units().iter().filter(|u| crate::prefs::unit_enabled(u.id, u.default_on)).collect();
        // выключили показатель — его строки уходят из итога: это тоже изменение, о котором надо сообщить
        let before = slots.len();
        slots.retain(|id, _| units.iter().any(|u| u.id == *id));
        if slots.len() != before {
            dirty = true;
        }
        for u in &units {
            let s = slots.entry(u.id).or_insert_with(|| Slot {
                rows: vec![Check::new(u.cat.key(), u.title, "checking", "проверяю…")],
                due: now,
                every: base_every(u.id) as f64 * factor,
                sig: String::new(),
                running: false,
                started: now,
                // номер запуска — сквозной на весь процесс: после выключения и включения показателя поздний ответ
                // прежнего запуска не совпадёт с новым номером
                gen: next_gen(),
                abandoned: 0,
            });
            // зависла надолго — бросить (её поток так и останется ждать) и запустить заново; не больше 3 раз
            if s.running && s.started.elapsed() > STUCK && s.abandoned < 3 {
                crate::diag::log(&format!("проверки: «{}» зависла — запускаю заново", u.id));
                s.running = false;
                s.abandoned += 1;
                s.gen = next_gen();
                s.due = now;
            }
            if !s.running && now >= s.due {
                s.running = true;
                s.started = now;
                let (f, id, gen, tx2) = (u.run, u.id, s.gen, done_tx.clone());
                let (key, title) = (u.cat.key(), u.title);
                std::thread::spawn(move || {
                    // сбой внутри проверки — строка «не получилось», а не тишина до «зависла»
                    let rows = std::panic::catch_unwind(f).unwrap_or_else(|_| {
                        vec![Check::new(key, title, crate::diag::INFO, "не получилось проверить")
                            .hint("Внутренняя ошибка проверки — подробности в журнале.")]
                    });
                    let _ = tx2.send((id, gen, rows));
                });
            }
            // не уложилась в 60 с — показать, но не запускать заново, пока не вернётся
            if s.running && s.started.elapsed() > Duration::from_secs(60) && s.rows.first().is_none_or(|r| r.value != "не отвечает") {
                let hint = if s.abandoned >= 3 {
                    "Проверка зависает раз за разом — больше не перезапускаю."
                } else {
                    "Проверка не уложилась в минуту — жду её."
                };
                s.rows = vec![Check::new(u.cat.key(), u.title, crate::diag::WARN, "не отвечает").hint(hint)];
                dirty = true;
            }
        }
        // готовые результаты
        while let Ok((id, gen, mut rows)) = done_rx.try_recv() {
            let Some(u) = units.iter().find(|u| u.id == id) else { continue };
            if slots.get(id).is_some_and(|s| s.gen != gen) {
                continue; // ответ брошенной проверки
            }
            for c in &mut rows {
                c.key = u.cat.key();
                c.unit = u.id;
            }
            let sig: String = rows.iter().map(|c| format!("{}:{}|", c.title, c.status)).collect();
            if let Some(s) = slots.get_mut(id) {
                let base = base_every(id) as f64 * factor;
                // ничего не поменялось и всё спокойно — реже (до ×4); поменялось или беда — как обычно
                let calm = sig == s.sig && !rows.iter().any(|c| c.status == "bad");
                s.every = if calm { (s.every * 1.5).min(base * 4.0) } else { base };
                s.sig = sig;
                s.running = false;
                s.abandoned = 0;
                s.due = Instant::now() + Duration::from_secs_f64(s.every.max(5.0));
                // строки поменялись (не только порядок времени) — разослать
                if rows_key(&rows) != rows_key(&s.rows) {
                    dirty = true;
                }
                s.rows = rows;
            }
        }
        // изменения копятся (dirty), пока их нельзя отправить; первые 3 с ждём медленные проверки, потом
        // отправляем в любом случае — даже если новых ответов больше не придёт
        if dirty {
            let all: Vec<Check> = units.iter().filter_map(|u| slots.get(u.id)).flat_map(|s| s.rows.clone()).collect();
            let first_pass = all.iter().any(|c| c.status == "checking");
            if !first_pass || slots.values().all(|s| !s.running) || begun.elapsed() > Duration::from_secs(3) {
                dirty = false;
                write_results(&all);
                if tx.send(all).is_err() {
                    return Produce::Closed;
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn is_dev_id(id: &str) -> bool {
    crate::catalog::units().iter().find(|u| u.id == id).is_some_and(|u| u.cat == crate::catalog::Cat::Dev)
}

/// Отпечаток устройств (Linux, без запуска программ): звуковые карты, USB-устройства, питание от сети.
/// На Windows и macOS пока пусто — там устройства проверяются по расписанию.
fn dev_signature() -> String {
    let mut s = std::fs::read_to_string("/proc/asound/cards").unwrap_or_default();
    if let Ok(rd) = std::fs::read_dir("/sys/bus/usb/devices") {
        let mut v: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        v.sort();
        s.push_str(&v.join(","));
    }
    if let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") {
        for e in rd.flatten() {
            s.push_str(&std::fs::read_to_string(e.path().join("online")).unwrap_or_default());
        }
    }
    s
}

fn is_net_id(id: &str) -> bool {
    crate::catalog::units().iter().find(|u| u.id == id).is_some_and(is_net)
}

/// Отпечаток строк для «изменилось ли»: заголовок, состояние, значение.
/// Отпечаток строк — всё, что видно и что делают кнопки (подсказка, действия и их аргументы): кнопка с новой
/// целью при том же тексте строки — тоже изменение.
fn rows_key(r: &[Check]) -> String {
    r.iter()
        .map(|c| {
            let fx: String = c.fixes.iter().map(|f| format!("{}\u{3}{}\u{3}{}\u{4}", f.id, f.label, f.arg.as_deref().unwrap_or(""))).collect();
            format!("{}\u{1}{}\u{1}{}\u{1}{}\u{1}{fx}\u{2}", c.title, c.status, c.value, c.detail)
        })
        .collect()
}

/// Сквозной номер запуска проверки.
fn next_gen() -> u64 {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Отпечаток сети: адрес, с которого идём в интернет (без отправки пакетов: UDP «connect» только выбирает
/// маршрут), и на Linux — интерфейс маршрута по умолчанию.
fn net_signature() -> String {
    let local = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("1.1.1.1:53")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "нет".into());
    let route = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let dflt =
        route.lines().skip(1).find(|l| l.split_whitespace().nth(1) == Some("00000000")).and_then(|l| l.split_whitespace().next()).unwrap_or("");
    format!("{local} {dflt}")
}

// ───────────────────────────── общий файл ─────────────────────────────

fn write_results(all: &[Check]) {
    write_checks(&res_path(), all);
}

fn read_results() -> Option<Vec<Check>> {
    read_checks(&res_path()).map(|(_, c)| c)
}

/// Записать строки проверок в файл (общий формат для checks.json и speed.json).
fn write_checks(path: &std::path::Path, all: &[Check]) {
    let j = serde_json::json!({
        "time": crate::ui::now_secs(),
        "checks": all.iter().map(|c| serde_json::json!({
            "key": c.key, "unit": c.unit, "title": c.title, "status": c.status, "value": c.value, "detail": c.detail,
            "unchecked": c.unchecked,
            "fixes": c.fixes.iter().map(|f| serde_json::json!({"id": f.id, "label": f.label, "arg": f.arg})).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let _ = crate::store::write_private(path, j.to_string().as_bytes());
}

/// Прочитать строки проверок: (когда записаны, строки).
fn read_checks(path: &std::path::Path) -> Option<(i64, Vec<Check>)> {
    let j: serde_json::Value = serde_json::from_str(&crate::store::read_capped(path, 8 << 20).ok()?).ok()?;
    let s = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    // key и unit в Check — &'static str: берём из каталога (по коду) и из списка групп
    const KEYS: [&str; 6] = ["net", "ai", "gpu", "dev", "theme", "sys"];
    let out = j["checks"]
        .as_array()?
        .iter()
        .map(|c| {
            let key = KEYS.iter().find(|k| **k == c["key"].as_str().unwrap_or("")).copied().unwrap_or("sys");
            let unit = crate::catalog::units().iter().find(|u| u.id == c["unit"].as_str().unwrap_or("")).map_or("", |u| u.id);
            // кнопки из файла — только известные действия с аргументом своей формы (файл мог подменить кто-то
            // другой); подпись — короткая, одной строкой
            let fixes: Vec<Fix> = c["fixes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| {
                    let fix = Fix { id: s(f, "id"), label: s(f, "label"), arg: f["arg"].as_str().map(String::from) };
                    let label_ok = fix.label.chars().count() <= 60 && !fix.label.chars().any(|c| c.is_control());
                    (label_ok && crate::fixes::validate(&fix.id, fix.arg.as_deref()).is_ok()).then_some(fix)
                })
                .collect();
            let mut ch = Check::new(key, &s(c, "title"), &s(c, "status"), &s(c, "value"));
            ch.detail = s(c, "detail");
            ch.fix = fixes.first().map(|f| f.id.clone());
            ch.fixes = fixes;
            ch.unit = unit;
            // старые файлы без поля — как «проверено»
            ch.unchecked = c["unchecked"].as_bool().unwrap_or(false);
            ch
        })
        .collect();
    Some((j["time"].as_i64().unwrap_or(0), out))
}

/// Результат последнего полного замера скорости — общий для окна и виджета (замеряет окно, а итог
/// «внимание/беда» должен быть одинаковым там и там).
pub fn save_speed(rows: &[Check]) {
    // неудавшийся замер (сервер не ответил, сети не было) не храним: он рассказывает о минуте замера,
    // а не о сети вообще, и не должен краснить итог часами
    let measured = rows
        .iter()
        .any(|c| c.title.starts_with("Скорость") && c.value.split_whitespace().next().is_some_and(|v| v.replace(',', ".").parse::<f64>().is_ok()));
    if !measured {
        let _ = std::fs::remove_file(dir().join("speed.json"));
        let _ = std::fs::remove_file(dir().join("speed.net"));
        return;
    }
    write_checks(&dir().join("speed.json"), rows);
    // в какой сети мерили: другая сеть (дом → кафе, включили VPN) — замер уже не про неё
    let _ = crate::store::write_private(&dir().join("speed.net"), net_hash().as_bytes());
}

/// Отпечаток сети для файла (не сам адрес и маршрут): совпадает — та же сеть.
fn net_hash() -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in net_signature().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Последний замер скорости, если он не старше 6 часов (старый уже ничего не говорит о сети сейчас).
pub fn load_speed() -> Vec<Check> {
    let same_net = crate::store::read_capped(&dir().join("speed.net"), 4096).is_ok_and(|n| n == net_hash());
    match read_checks(&dir().join("speed.json")) {
        Some((t, rows)) if same_net && crate::ui::now_secs() - t < 6 * 3600 => rows,
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod unchecked_tests {
    use super::*;

    #[test]
    fn unchecked_flag_roundtrip_and_old_files() {
        let dir = std::env::temp_dir().join(format!("aptechka-unk-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("c.json");
        let rows = vec![Check::new("sys", "Диск", "info", "не проверить").unknown(), Check::new("sys", "Память", "ok", "1")];
        write_checks(&p, &rows);
        let (_, back) = read_checks(&p).unwrap();
        assert_eq!(back.iter().map(|c| c.unchecked).collect::<Vec<_>>(), [true, false]);
        // старый файл без поля
        std::fs::write(&p, r#"{"time":1,"checks":[{"key":"sys","title":"Диск","status":"info","value":"не проверить"}]}"#).unwrap();
        let (_, old) = read_checks(&p).unwrap();
        assert!(!old[0].unchecked);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod economy_tests {
    use super::*;

    #[test]
    fn economy_decision() {
        for choice in [None, Some("normal"), Some("rare"), Some("often")] {
            for battery in [false, true] {
                for metered in [false, true] {
                    let e = economy(battery, metered, choice);
                    let want = match (choice, battery, metered) {
                        (Some("often") | Some("rare"), _, _) => None,
                        (_, true, _) => Some(Eco::Battery),
                        (_, false, true) => Some(Eco::Metered),
                        _ => None,
                    };
                    assert_eq!(e, want, "{choice:?} батарея {battery} лимитная {metered}");
                }
            }
        }
    }

    #[test]
    fn factor_follows_choice_and_economy() {
        assert_eq!(factor_for(None, None), 1.0);
        assert_eq!(factor_for(Some("normal"), Some(Eco::Battery)), 3.0);
        assert_eq!(factor_for(None, Some(Eco::Metered)), 3.0);
        assert_eq!(factor_for(Some("often"), None), 0.5);
        assert_eq!(factor_for(Some("rare"), None), 3.0);
    }

    #[test]
    fn pmset_battery_text() {
        let bat = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t87%; discharging; 4:10 remaining";
        let ac = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t87%; charging; 0:40 remaining";
        assert!(crate::netproxy::pmset_on_battery(bat));
        assert!(!crate::netproxy::pmset_on_battery(ac));
        assert!(!crate::netproxy::pmset_on_battery("Now drawing from 'Battery Power'"));
    }
}
