//! Кнопки «полечить» — перенос aptechka/fixes.py. Каждая функция возвращает (успех, сообщение).
//!
//! DRY_RUN=1 — пробный режим: команды только пишутся в журнал, ничего не выполняется
//! (даже безобидные запросы). Именно так проверяем на рабочей машине.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static DRY: AtomicBool = AtomicBool::new(false);

pub fn set_dry_run(on: bool) {
    DRY.store(on, Ordering::Relaxed);
}

pub fn dry_run() -> bool {
    DRY.load(Ordering::Relaxed)
}

pub(crate) fn log(msg: &str) {
    // в журнал — без секретов (аргументы команд, адреса с токенами) и без управляющих знаков терминала
    eprintln!("[лечение] {}", crate::diag::term_safe(&crate::proxyaddr::redact(msg)));
}

/// Запустить команду и дождаться: (успех, последние 200 символов вывода).
fn sh(cmd: &[&str], timeout_s: u64) -> (bool, String) {
    sh_opts(cmd, timeout_s, crate::cmd::FIX)
}

/// Запуск программы для человека (её окно — на языке системы, а не по-английски).
fn sh_app(cmd: &[&str], timeout_s: u64) -> (bool, String) {
    sh_opts(cmd, timeout_s, crate::cmd::Opts { c_locale: false, stderr: true })
}

fn sh_opts(cmd: &[&str], timeout_s: u64, o: crate::cmd::Opts) -> (bool, String) {
    if dry_run() {
        log(&format!("DRY_RUN: выполнила бы: {}", cmd.join(" ")));
        return (true, String::new());
    }
    log(&format!("запускаю: {}", cmd.join(" ")));
    outcome_msg(cmd, crate::cmd::exec(cmd, Duration::from_secs(timeout_s), o))
}

/// Сначала без прав; с паролем администратора (pkexec) — только если команда есть и ответила отказом.
/// Раньше пароль спрашивали и когда программы просто нет в системе — пароль ничего бы не дал.
fn sh_or_admin(cmd: &[&str], timeout_s: u64, admin_timeout_s: u64) -> (bool, String) {
    if dry_run() {
        return sh(cmd, timeout_s);
    }
    log(&format!("запускаю: {}", cmd.join(" ")));
    match crate::cmd::exec(cmd, Duration::from_secs(timeout_s), crate::cmd::FIX) {
        crate::cmd::Outcome::Failed(_, ref out) if denied(out) => match cmd.split_first() {
            Some((prog, args)) => sh_admin(prog, args, admin_timeout_s, crate::elevate::Profile::Service),
            None => (false, "Пустая команда.".into()),
        },
        other => outcome_msg(cmd, other),
    }
}

/// Похоже ли, что команде отказали в правах (а не что она упала по другой причине): только тогда просим пароль.
fn denied(out: &str) -> bool {
    let l = out.to_lowercase();
    [
        "permission denied",
        "operation not permitted",
        "access is denied",
        "must be root",
        "authentication",
        "not authorized",
        "eacces",
        "eperm",
        "root privileges",
        "you need to be root",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// С правами администратора — только через elevate: доверенная системная программа по абсолютному пути,
/// в песочнице systemd (Linux). Аптечку саму с правами root не запускаем.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn sh_admin(prog: &str, args: &[&str], timeout_s: u64, p: crate::elevate::Profile) -> (bool, String) {
    #[cfg(target_os = "linux")]
    {
        let Some(c) = crate::elevate::linux_cmd(prog, args, p) else {
            return (false, format!("Не запускаю «{prog}» с правами администратора: нет такой системной программы (или она не принадлежит root)."));
        };
        let v: Vec<&str> = c.iter().map(String::as_str).collect();
        sh(&v, timeout_s)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (prog, args, timeout_s, p);
        (false, "Только для Linux.".into())
    }
}

fn outcome_msg(cmd: &[&str], r: crate::cmd::Outcome) -> (bool, String) {
    use crate::cmd::Outcome;
    let prog = cmd.first().copied().unwrap_or("");
    match r {
        Outcome::Ok(out) => (true, out),
        Outcome::Failed(code, out) => (false, explain(cmd, Some(code), &out)),
        Outcome::Missing => (false, format!("В системе нет программы {prog}.")),
        Outcome::Timeout { stopped: true } => (false, explain(cmd, None, "")),
        Outcome::Timeout { stopped: false } => (
            false,
            "Команда не ответила вовремя и не остановилась (она работает с правами администратора). \
             Если открыто окно пароля — закрой его; иначе дождись, пока команда закончит сама."
                .into(),
        ),
        Outcome::SpawnError(e) => (false, e),
    }
}

/// Понятная причина неудачи вместо сырого вывода команды.
fn explain(cmd: &[&str], code: Option<i32>, out: &str) -> String {
    let Some(code) = code else { return "Команда не ответила вовремя, я её остановила.".into() };
    if cmd.first().is_some_and(|c| c.ends_with("pkexec")) {
        // для подсказки — сама команда, без обёртки песочницы
        let rest = match cmd.iter().position(|a| *a == "--") {
            Some(i) => cmd[i + 1..].join(" "),
            None => cmd[1..].join(" "),
        };
        // 126 — окно пароля закрыли или пароль не подошёл; 127 — спросить пароль было нечем
        if code == 126 {
            return "Отменено: пароль администратора не введён.".into();
        }
        if code == 127 && (out.contains("authentication agent") || out.contains("Not authorized")) {
            return format!("Не получилось спросить пароль администратора. Выполни в терминале: sudo {rest}");
        }
    }
    if cmd.contains(&"systemctl") && out.contains("not found") {
        return format!("Служба {} не найдена.", cmd.last().unwrap_or(&""));
    }
    // последняя целая строка вывода, не длиннее 200 символов
    let line = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if line.is_empty() {
        return format!("Команда завершилась с ошибкой (код {code}).");
    }
    let chars: Vec<char> = line.chars().collect();
    if chars.len() > 200 {
        format!("{}…", chars[..200].iter().collect::<String>())
    } else {
        line.to_string()
    }
}

fn pause(ms: u64) {
    if !dry_run() {
        thread::sleep(Duration::from_millis(ms));
    }
}

/// Запустить команду и дождаться её (короткая обёртка «запусти в фоне и выйди»): успех — код 0.
fn sh_status(c: &mut std::process::Command) -> bool {
    crate::diag::quiet(c).status().is_ok_and(|s| s.success())
}

/// Закрыть VPN-клиент и открыть заново. Сначала убеждаемся, что открыть есть чем: запоминаем, как он был
/// запущен (его программа и аргументы — самый надёжный способ «тем же путём»), а без этого — ярлык
/// (gtk-launch). Нечем открыть — не закрываем вовсе (раньше VPN закрывался, а потом «не вышло»).
fn restart_app(procs: &[&str], desktop_id: &str) -> (bool, String) {
    let running: Vec<crate::procs::Proc> = crate::procs::list().into_iter().filter(|p| procs.iter().any(|n| p.name == *n)).collect();
    // как запущен: /proc/<pid>/exe и аргументы главного процесса (родитель которого — не такой же процесс)
    let main = running.iter().find(|p| !running.iter().any(|q| q.pid == p.ppid));
    let relaunch: Option<Vec<String>> = main.and_then(|p| {
        let exe = std::fs::read_link(format!("/proc/{}/exe", p.pid)).ok()?;
        let exe = exe.to_string_lossy().trim_end_matches(" (deleted)").to_string();
        std::path::Path::new(&exe).is_file().then(|| std::iter::once(exe).chain(p.args.iter().skip(1).cloned()).collect())
    });
    // ярлык: и программа gtk-launch, и сам файл ярлыка в папках приложений
    let desktop_file = std::iter::once(crate::store::home_dir().map(|h| h.join(".local/share")).unwrap_or_default())
        .chain(std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into()).split(':').map(std::path::PathBuf::from))
        .chain([std::path::PathBuf::from("/var/lib/flatpak/exports/share"), std::path::PathBuf::from("/var/lib/snapd/desktop")])
        .any(|d| d.join("applications").join(format!("{desktop_id}.desktop")).is_file());
    let launcher = crate::procs::which("gtk-launch").is_some() && desktop_file;
    if relaunch.is_none() && !launcher {
        return (false, "Не знаю, чем открыть клиент заново (нет gtk-launch), поэтому закрывать не стала. Перезапусти его сама.".into());
    }
    let old: Vec<i64> = running.iter().map(|p| p.pid).collect();
    for n in procs {
        sh(&["pkill", "-x", n], 30);
    }
    if dry_run() {
        return (true, "Открыла заново.".into());
    }
    // дождаться, что прежние процессы правда закрылись (до 5 с); не закрылись — второй копии не запускаем
    let mut gone = false;
    for _ in 0..25 {
        pause(200);
        if !crate::procs::list_fresh().iter().any(|p| old.contains(&p.pid)) {
            gone = true;
            break;
        }
    }
    if !gone {
        return (false, "Клиент не закрылся — второй его копии запускать не стала. Закрой его вручную.".into());
    }
    let started = match &relaunch {
        Some(cmd) => {
            // отдельно от Аптечки (не её дочерний процесс): клиент живёт дальше, когда Аптечка закроется.
            // Программа и аргументы — позиционными параметрами sh, не текстом команды
            let mut c = std::process::Command::new("/bin/sh");
            c.arg("-c").arg("\"$0\" \"$@\" </dev/null >/dev/null 2>&1 &").args(cmd);
            sh_status(&mut c)
        }
        None => sh_app(&["gtk-launch", desktop_id], 10).0,
    };
    // правда о результате: процесс снова есть?
    for _ in 0..10 {
        pause(500);
        // появился именно новый процесс (номер не из прежних)
        if crate::procs::list_fresh().iter().any(|p| procs.iter().any(|n| p.name == *n) && !old.contains(&p.pid)) {
            return (true, "Открыла заново, подключись в окне клиента. Связь через VPN вернётся после подключения.".into());
        }
    }
    (
        false,
        if started {
            "Запустила, но клиент так и не появился — открой его сама.".into()
        } else {
            "Не удалось открыть клиент заново — открой его сама.".into()
        },
    )
}

fn net_reconnect() -> (bool, String) {
    let (_, dev_out) = sh(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE", "device"], 30);
    let part = |l: &str, i: usize| l.split(':').nth(i).unwrap_or("").to_string();
    // то устройство, через которое сейчас идёт интернет (маршрут), а не первые два по списку
    let route_dev = crate::diag::run(&["ip", "route", "get", "1.1.1.1"], 6).1;
    let route_dev = route_dev.split_whitespace().skip_while(|w| *w != "dev").nth(1).unwrap_or("").to_string();
    let mut devs: Vec<String> = dev_out
        .lines()
        .filter(|l| matches!(part(l, 1).as_str(), "ethernet" | "wifi") && l.contains("connected") && !l.contains("disconnected"))
        .map(|l| part(l, 0))
        .collect();
    if let Some(i) = devs.iter().position(|d| *d == route_dev) {
        devs = vec![devs.remove(i)];
    }
    if devs.is_empty() {
        devs = dev_out.lines().filter(|l| l.contains(":ethernet:") || l.contains(":wifi:")).map(|l| part(l, 0)).collect();
    }
    if dry_run() && devs.is_empty() {
        devs.push("<устройство>".into()); // в пробном режиме запроса не было — покажем команду
    }
    let mut msgs = Vec::new();
    let mut all_ok = !devs.is_empty();
    for d in devs.iter().take(2) {
        let (ok, m) = sh(&["nmcli", "device", "reconnect", d], 40);
        all_ok &= ok;
        msgs.push(format!("{d}: {}", if ok { "ок".to_string() } else { m }));
    }
    let msg = if msgs.is_empty() { "не нашла сетевых устройств".to_string() } else { msgs.join("; ") };
    // успех — только если переподключились все, кого трогали (раньше «успех» был, раз устройства нашлись)
    (all_ok, msg)
}

fn dns_flush() -> (bool, String) {
    let (ok, m) = sh_or_admin(&["resolvectl", "flush-caches"], 30, 30);
    (ok, if ok { "Кэш DNS очищен.".into() } else { m })
}

/// Записать настройку GNOME и проверить, что записалось: без сессионной шины gsettings отвечает «успех»
/// (код 0), а настройка не меняется. Успех — только если прочиталось то, что записали.
fn gset(schema: &str, key: &str, val: &str) -> (bool, String) {
    let (ok, m) = sh(&["gsettings", "set", schema, key, val], 30);
    if !ok || dry_run() {
        return (ok, m);
    }
    let (_, now) = sh(&["gsettings", "get", schema, key], 10);
    // сравнение без кавычек и типа: «'none'» = «none», «@as []» = «[]»
    let norm = |s: &str| s.trim().trim_start_matches("@as ").trim_matches('\'').replace(['\'', '"'], "");
    if norm(&now) == norm(val) {
        (true, String::new())
    } else {
        (
            false,
            "Настройка не сохранилась: нет связи с сеансом рабочего стола (Аптечка запущена не из него?). Выполни команду в терминале своего сеанса."
                .into(),
        )
    }
}

/// То же для dconf.
fn dset(path: &str, val: &str) -> (bool, String) {
    let (ok, m) = sh(&["dconf", "write", path, val], 30);
    if !ok || dry_run() {
        return (ok, m);
    }
    let (_, now) = sh(&["dconf", "read", path], 10);
    if now.trim() == val.trim() {
        (true, String::new())
    } else {
        (false, "Настройка не сохранилась: нет связи с сеансом рабочего стола.".into())
    }
}

fn proxy_off() -> (bool, String) {
    let (ok, m) = gset("org.gnome.system.proxy", "mode", "'none'");
    (
        ok,
        if ok {
            "Системный прокси выключен. Новые терминалы тоже его не возьмут, а в уже открытых выполни unset http_proxy https_proxy.".into()
        } else {
            m
        },
    )
}

fn svc_restart(unit: &str) -> (bool, String) {
    // «cups» → «cups.service» (так зовёт и сам systemctl)
    let full = if unit.contains('.') { unit.to_string() } else { format!("{unit}.service") };
    let unit = full.as_str();
    // с паролем администратора перезапускаем не любую службу, а упавшую или ту, что Аптечка сама показывает
    // (VPN-клиенты, Bluetooth, печать): кнопка с чужим именем (например, display-manager) ничего не сделает
    const KNOWN: [&str; 5] =
        ["happd.service", "outline_proxy_controller.service", "me.proton.vpn.split_tunneling.service", "bluetooth.service", "cups.service"];
    let failed = crate::diag::run(&["systemctl", "is-failed", "--quiet", "--", unit], 6).0 == 0;
    if !failed && !KNOWN.contains(&unit) && !dry_run() {
        return (false, format!("Служба {unit} не упала — перезапускать её не стану."));
    }
    // имя службы — «имя.тип» из безопасных знаков; «--user» и подобное не превратится в ключ systemctl
    let valid = !unit.starts_with('-')
        && unit.len() <= 200
        && unit.chars().all(|c| c.is_ascii_alphanumeric() || ":_.@-\\".contains(c))
        && [".service", ".socket", ".timer", ".path", ".mount"].iter().any(|t| unit.ends_with(t));
    if !valid {
        return (false, format!("Непонятное имя службы: {unit}"));
    }
    let (ok, m) = sh_admin("systemctl", &["restart", "--", unit], 60, crate::elevate::Profile::Service);
    (
        ok,
        if ok {
            format!("Служба {unit} перезапущена.")
        } else if m.contains("not found") {
            format!("Служба {unit} не найдена.")
        } else {
            m
        },
    )
}

fn prime_nvidia() -> (bool, String) {
    let (ok, m) = sh_admin("prime-select", &["nvidia"], 120, crate::elevate::Profile::Packages);
    (
        ok,
        if ok {
            "Готово. Чтобы всё рисовала NVIDIA, выйди из системы и зайди снова (или перезагрузись).".into()
        } else {
            m
        },
    )
}

fn ext_restart(uuid: &str) -> (bool, String) {
    sh(&["gnome-extensions", "disable", "--", uuid], 30);
    pause(800);
    let (ok, m) = sh(&["gnome-extensions", "enable", "--", uuid], 30);
    (ok, if ok { "Расширение перезапущено.".into() } else { m })
}

fn ext_enable(uuid: &str) -> (bool, String) {
    let (ok, m) = sh(&["gnome-extensions", "enable", "--", uuid], 30);
    (ok, if ok { "Включено.".into() } else { m })
}

fn shell_restart() -> (bool, String) {
    // тип сеанса — у logind, а не по переменной окружения (она врёт при запуске из ssh, sudo, другого сеанса):
    // в Wayland этот перезапуск завершил бы весь сеанс
    let sid = std::env::var("XDG_SESSION_ID").unwrap_or_default();
    let ty = if sid.is_empty() {
        String::new()
    } else {
        crate::diag::run(&["loginctl", "show-session", &sid, "-p", "Type", "--value"], 5).1.trim().to_string()
    };
    if ty != "x11" && !dry_run() {
        return (false, "Перезапуск оболочки без выхода работает только в X11.".into());
    }
    // systemd поднимает оболочку заново, но после 3 падений за 15 с сдаётся и рушит сессию —
    // поэтому чаще раза в 20 с не перезапускаем
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    if let Ok(mut last) = LAST.lock() {
        if let Some(t) = *last {
            if t.elapsed() < Duration::from_secs(20) {
                let left = 20 - t.elapsed().as_secs();
                return (false, format!("Оболочка только что перезапускалась. Подожди {left} с и попробуй ещё раз."));
            }
        }
        *last = Some(Instant::now());
    }
    let (ok, m) = sh(&["pkill", "-QUIT", "-x", "gnome-shell"], 30);
    (ok, if ok { "Оболочка GNOME перезапускается, окна останутся на месте.".into() } else { m })
}

fn time_sync() -> (bool, String) {
    // уже включена — пароль не нужен; иначе включаем
    let on = crate::diag::run(&["timedatectl", "show", "-p", "NTP", "--value"], 6).1.trim() == "yes";
    if !on {
        let (ok, m) = sh_admin("timedatectl", &["set-ntp", "true"], 90, crate::elevate::Profile::Service);
        if !ok {
            return (false, m);
        }
    }
    if dry_run() {
        return (true, "Автосверка включена.".into());
    }
    // результат, а не «должно выровняться»: ждём до 30 с, пока система скажет «сверено»
    for _ in 0..15 {
        if crate::diag::run(&["timedatectl", "show", "-p", "NTPSynchronized", "--value"], 6).1.trim() == "yes" {
            return (true, "Часы сверены с интернетом.".into());
        }
        pause(2000);
    }
    (false, "Автосверка включена, но сверить часы не вышло за 30 с: сервер времени не отвечает (часто закрыт порт 123 — так бывает в рабочих и гостевых сетях). Попробуй в другой сети.".into())
}

/// Включить док заново, перезапустить «сторожа» полноэкранного режима, если он есть,
/// и вернуть аптечку в избранное дока.
/// Удалить свои задания печати: только этого пользователя, явно (cancel -a без прав и так не тронет чужие,
/// но с правами администратора удалил бы всё у всех).
fn print_clear() -> (bool, String) {
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    if user.is_empty() {
        return (false, "Не знаю имя пользователя — не трогаю очередь печати.".into());
    }
    done(sh(&["cancel", "-u", &user, "-x"], 20), "Мои задания печати удалены.")
}

/// Вернуть переключение раскладки: добавить Super+Пробел к тому, что задано, а не заменить всё.
fn kb_switch() -> (bool, String) {
    let (_, cur) = sh(&["gsettings", "get", "org.gnome.desktop.wm.keybindings", "switch-input-source"], 10);
    let cur = cur.trim();
    if cur.contains("<Super>space") {
        return (true, "Super+Пробел уже переключает раскладку.".into());
    }
    let items: Vec<&str> = cur
        .trim_start_matches("@as")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    let new = format!("[{}]", std::iter::once("'<Super>space'").chain(items).collect::<Vec<_>>().join(", "));
    done(gset("org.gnome.desktop.wm.keybindings", "switch-input-source", &new), "Раскладка снова переключается по Super+Пробел.")
}

fn dock_repair(uuid: Option<&str>) -> (bool, String) {
    // какой док — говорит проверка (включённое расширение дока); без него не угадываем
    let found = uuid.filter(|u| u.contains('@')).map(String::from).or_else(crate::diag::current_dock);
    let Some(uuid) = found else {
        return (false, "Не нашла расширение дока (Dash to Dock, Ubuntu Dock, Dash2Dock Lite).".into());
    };
    let uuid = uuid.as_str();
    let mut steps: Vec<String> = Vec::new();
    sh(&["gnome-extensions", "disable", "--", uuid], 30);
    pause(800);
    let (ok, m) = sh(&["gnome-extensions", "enable", "--", uuid], 30);
    steps.push(if ok { "расширение перезапущено".into() } else { format!("не включилось: {m}") });
    let (ok2, out) = sh(&["systemctl", "--user", "list-unit-files", "dock-fullscreen-guard.service"], 30);
    if ok2 && (out.contains("dock-fullscreen-guard") || dry_run()) {
        sh(&["systemctl", "--user", "restart", "dock-fullscreen-guard.service"], 30);
        steps.push("сторож полноэкранного режима перезапущен".into());
    }
    pause(1500);
    let (_, info) = sh(&["gnome-extensions", "info", uuid], 30);
    // «State: ACTIVE»/«ENABLED» или «Enabled: Yes» — словом целиком (contains("ACTIVE") ловил и INACTIVE)
    let good = dry_run()
        || info.lines().any(|l| {
            let l = l.trim();
            matches!(l.strip_prefix("State:").map(str::trim), Some("ACTIVE" | "ENABLED")) || l == "Enabled: Yes"
        });
    let head = if good { "Док починен: " } else { "Док не ожил: " };
    let tail = if good {
        ""
    } else {
        ". Если окно на весь экран — выйди из него (F11). Если не поможет — Alt+F2, r, Enter (перезапуск GNOME Shell)."
    };
    (good, format!("{head}{}{tail}", steps.join(", ")))
}

/// Вернуть одну настройку темы к значению из эталона (ключ вида «вид|схема|ключ»).
fn theme_restore(key: &str) -> (bool, String) {
    // эталон читаем без записи: если файла нет, берётся текущее состояние «в уме»
    let snap = crate::diag::load_snapshot();
    let Some(val) = snap.key(key) else { return (false, "В эталоне нет этого значения.".into()) };
    let mut parts = key.splitn(3, '|');
    let (Some(kind), Some(a), Some(b)) = (parts.next(), parts.next(), parts.next()) else {
        return (false, format!("Непонятный ключ: {key}"));
    };
    let (ok, m) = if kind == "gsettings" {
        // значение — как сохранено (запись GVariant: 'Yaru', '' или с кавычкой внутри); раньше кавычки
        // обрезались, и пустые значения и строки с апострофом не возвращались
        gset(a, b, val)
    } else {
        dset(a, val)
    };
    (ok, if ok { format!("Вернула как в эталоне: {b} = {val}.") } else { m })
}

/// Запомнить текущие темы и расширения как норму (пишет файл эталона).
fn snapshot() -> (bool, String) {
    if dry_run() {
        log(&format!("DRY_RUN: записала бы эталон тем в {}", crate::diag::snapshot_path()));
        return (true, "Запомнила текущие темы и расширения как норму.".into());
    }
    match crate::diag::take_snapshot() {
        Ok(_) => (true, "Запомнила текущие темы и расширения как норму.".into()),
        Err(e) => (false, format!("Не удалось записать эталон: {e}")),
    }
}

/// Нужно ли подтверждение перед запуском (и что сказать в окне). None — запускать сразу.
pub fn confirm_text(id: &str) -> Option<&'static str> {
    #[cfg(aptechka_custom)]
    if id.starts_with("my_") {
        return crate::custom::confirm(id);
    }
    match id {
        "sound_restart" => Some("Звук пропадёт на пару секунд."),
        "footprint_history" => Some("Строки с ключами удалятся из истории команд насовсем. Сами ключи лучше потом заменить новыми."),
        "win_winhttp_reset" | "win_svc_restart" | "win_time_sync" => Some("Windows спросит разрешение администратора."),
        "mac_dns_flush" | "mac_audio_restart" | "mac_time_sync" => Some("macOS спросит пароль администратора."),
        "mac_thin_snapshots" => Some("macOS спросит пароль. Локальные снимки Time Machine удалятся (резервные копии на внешнем диске останутся)."),
        "mac_finder_restart" => Some("Окна Finder закроются и откроются заново."),
        "mac_dock_restart" | "mac_menubar_restart" => Some("Dock и строка меню на секунду пропадут."),
        "mac_force_quit" => Some("Программа закроется. Несохранённое в ней может пропасть."),
        "win_update_reset" => Some("Windows спросит разрешение администратора. Остановлю службы обновления и начну их кэш заново."),
        "win_explorer_restart" => Some("Панель задач и окна папок моргнут; открытые программы останутся."),
        "win_kill" => Some("Программа закроется принудительно — несохранённое в ней пропадёт."),
        "dpkg_configure" | "apt_fix" => Some("Понадобится пароль. Может занять несколько минут — не выключай компьютер."),
        "apt_autoremove" => Some("Понадобится пароль. Удалю только старые ядра — работающее сейчас и по два самых новых каждого вида останутся; сначала проверю пробным прогоном, что apt не тронет ничего лишнего."),
        "trash_empty" => Some("Файлы из корзины удалятся насовсем — и из корзин на подключённых дисках."),
        "journal_vacuum" => Some("Понадобится пароль. Старые записи журнала удалятся."),
        "print_clear" => Some("Удалю твои задания из очереди печати."),
        "proc_term" => Some("Программа закроется. Несохранённое в ней может пропасть."),
        "dupes_trash" => Some("Лишние копии уйдут в Корзину, в каждой группе один файл останется. Вернуть можно из Корзины."),
        "net_reconnect" => Some("Сеть пропадёт на несколько секунд."),
        "happ_restart" => Some("Happ закроется и откроется заново."),
        "outline_restart" => Some("Outline закроется и откроется заново."),
        "proton_restart" => Some("Proton VPN закроется и откроется заново."),
        "svc_restart" => Some("Понадобится пароль администратора."),
        "prime_nvidia" => Some("Понадобится пароль, потом нужно перезайти в систему."),
        "snapshot" => Some("Текущее состояние тем станет эталоном."),
        "time_sync" => Some("Понадобится пароль администратора."),
        "dock_repair" => Some("Док на секунду пропадёт и появится снова."),
        "shell_restart" => Some("Панель и док моргнут, окна останутся."),
        _ => None,
    }
}

static RUNNING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Отметка «действие выполняется»; снимается сама, когда выполнение закончилось (даже с ошибкой).
/// Внутри процесса — по имени действия, между процессами (окно, виджет, --fix) — по ресурсу, который
/// действие меняет: две починки пакетов или «найти копии» и «убрать копии» не идут одновременно.
struct Busy(String, #[allow(dead_code)] Option<crate::store::lock::Held>);

/// Какой общий ресурс меняет действие (имя файла-замка).
fn resource(id: &str) -> &str {
    match id {
        "dupes_scan" | "dupes_trash" => "dupes",
        "dpkg_configure" | "apt_fix" | "apt_autoremove" | "prime_nvidia" => "packages",
        "net_reconnect" | "dns_flush" | "proxy_off" | "happ_restart" | "outline_restart" | "proton_restart" | "wifi_on" | "win_proxy_off"
        | "win_dns_flush" | "win_winhttp_reset" | "mac_proxy_off" | "mac_dns_flush" => "network",
        "env_proxy_off" | "footprint_history" | "footprint_lock" => "userfiles",
        "theme_restore" | "win_theme_restore" | "mac_theme_restore" | "snapshot" | "ext_restart" | "ext_enable" | "dock_repair" | "kb_switch" => {
            "desktop"
        }
        other => other,
    }
}

impl Busy {
    /// Err — текст для пользователя: «уже выполняется» только когда замок правда держит другой процесс;
    /// испорченный замок (каталог на месте файла, права) — отдельное внятное сообщение.
    fn take(id: &str) -> Result<Busy, String> {
        let busy = || "Это уже выполняется — дождись, пока закончится.".to_string();
        let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if r.iter().any(|x| x == id) {
            return Err(busy());
        }
        // замок ресурса между процессами (не ждём: занято — так и скажем)
        let path = crate::store::private_dir().join(format!("fix-{}.lock", resource(id)));
        let held = match crate::store::lock::take(&path) {
            crate::store::lock::Take::Got(h) => Some(h),
            crate::store::lock::Take::Busy => return Err(busy()),
            // папки настроек нет (работаем без сохранения) — внутри процесса защищает RUNNING
            crate::store::lock::Take::Broken(_) if !path.parent().is_some_and(|d| d.is_dir()) => None,
            crate::store::lock::Take::Broken(e) => {
                return Err(format!("Не могу занять замок действия: файл замка испорчен ({e}). Удали его вручную: {}", path.display()));
            }
        };
        r.push(id.to_string());
        Ok(Busy(id.to_string(), held))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        let mut r = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        r.retain(|x| *x != self.0);
    }
}

/// Выполнить починку (сбой внутри — строка «не получилось», программа не падает).
/// Действия без аргумента (если аргумент пришёл — он не используется).
const NO_ARG: [&str; 44] = [
    "net_reconnect",
    "dns_flush",
    "proxy_off",
    "happ_restart",
    "outline_restart",
    "proton_restart",
    "prime_nvidia",
    "time_sync",
    "shell_restart",
    "snapshot",
    "sound_restart",
    "sound_unmute",
    "sound_volume",
    "mic_unmute",
    "bt_unblock",
    "bt_power",
    "wifi_on",
    "print_clear",
    "kb_switch",
    "dpkg_configure",
    "apt_fix",
    "apt_autoremove",
    "trash_empty",
    "thumbs_clear",
    "journal_vacuum",
    "dupes_scan",
    "npm_proxy_off",
    "win_proxy_off",
    "win_dns_flush",
    "win_winhttp_reset",
    "win_update_reset",
    "win_explorer_restart",
    "win_time_sync",
    "win_cleanmgr",
    "mac_proxy_off",
    "mac_dns_flush",
    "mac_finder_restart",
    "mac_dock_restart",
    "mac_menubar_restart",
    "mac_audio_restart",
    "mac_time_sync",
    "mac_thin_snapshots",
    "footprint_lock",
    "footprint_history",
];

/// Проверка действия и аргумента — одна для всех путей (кнопка в окне и виджете, общий файл checks.json,
/// командная строка --fix, «ручная» команда). Неизвестное действие или аргумент не своей формы — отказ:
/// кнопку из подменённого файла с чужим аргументом не выполнить.
/// Время запуска процесса macOS (`ps -o lstart=`, «Thu Nov  5 21:59:32 2026») — короткой меткой без пробелов
/// и двоеточий для аргумента кнопки: «Thu-Nov-5-21-59-32-2026».
pub(crate) fn mac_start_key(lstart: &str) -> String {
    lstart.split(|c: char| c.is_whitespace() || c == ':').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-")
}

/// Аргумент кнопки «закрыть» на macOS: «номер@время запуска»; None — процесса уже нет.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn mac_quit_arg(pid: u32) -> Option<String> {
    let ps = pid.to_string();
    let l = crate::cmd::exec(&["ps", "-p", &ps, "-o", "lstart="], std::time::Duration::from_secs(5), crate::cmd::CHECK).text().trim().to_string();
    (!l.is_empty()).then(|| format!("{pid}@{}", mac_start_key(&l)))
}

pub fn validate(id: &str, arg: Option<&str>) -> Result<(), String> {
    let a = arg.unwrap_or("");
    let chars = |s: &str, extra: &str, max: usize| {
        !s.is_empty() && s.len() <= max && !s.starts_with('-') && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
    };
    let digits = |s: &str| !s.is_empty() && s.len() <= 12 && s.chars().all(|c| c.is_ascii_digit());
    let hex16 = |s: &str| s.len() == 16 && s.chars().all(|c| c.is_ascii_hexdigit());
    let ok = match id {
        _ if NO_ARG.contains(&id) => true,
        // имя службы systemd: «cups», «bluetooth.service», «user@1000.service»
        "svc_restart" => chars(a, ":_.@-\\", 200),
        "ext_restart" | "ext_enable" => chars(a, "._@-", 128) && a.contains('@'),
        "dock_repair" => a.is_empty() || chars(a, "._@-", 128) && a.contains('@'),
        "theme_restore" => crate::diag::theme_key_known(a),
        "win_theme_restore" | "mac_theme_restore" => look_known(a),
        "printer_enable" => chars(a, "_.@-", 127),
        "env_proxy_off" => crate::netproxy::env_spec_ok(a),
        "git_proxy_off" => matches!(a, "http.proxy" | "https.proxy"),
        "dupes_trash" => hex16(a) || a.strip_prefix("all:").is_some_and(hex16),
        "open_url" => {
            (a.starts_with("http://") || a.starts_with("https://"))
                && a.len() <= 2048
                && !a.chars().any(|c| c.is_whitespace() || c.is_control() || "\"'`<>".contains(c))
        }
        // номер процесса и время его запуска (без времени процесс под этим номером мог смениться)
        "proc_term" => a.split_once('@').is_some_and(|(p, t)| digits(p) && digits(t) && p.parse::<u32>().is_ok_and(|p| p > 1)),
        "win_kill" => {
            let (p, n) = a.split_once(':').unwrap_or((a, "x"));
            digits(p) && !n.is_empty() && n.len() <= 260 && !n.chars().any(|c| c.is_control())
        }
        "win_svc_restart" => !a.is_empty() && a.len() <= 80 && a.chars().all(|c| c.is_ascii_alphanumeric() || " -_.".contains(c)),
        // номер 0 и 1 — «вся группа» и система: ни кнопкой, ни ручной командой
        // «номер@время запуска» (как proc_term на Linux): время снято, когда строку показали, и сверяется перед
        // каждым сигналом — номер, доставшийся с тех пор другой программе, не тронем
        "mac_force_quit" => a.split_once('@').is_some_and(|(p, k)| {
            digits(p)
                && p.parse::<u32>().is_ok_and(|p| p > 1)
                && (8..=40).contains(&k.len())
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        }),
        // своя сборка владелицы (custom/): свои действия и своя проверка параметров
        #[cfg(aptechka_custom)]
        m if m.starts_with("my_") => crate::custom::valid(m, a),
        _ => return Err(format!("Неизвестное действие: {id}")),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("Странный параметр у действия «{id}» — не выполняю."))
    }
}

pub fn apply(id: &str, arg: Option<&str>) -> (bool, String) {
    log(&format!("{}лечение «{id}»{}", if dry_run() { "ПРОБНЫЙ РЕЖИМ: " } else { "" }, arg.map(|a| format!(" ({a})")).unwrap_or_default()));
    if !for_this_os(id) {
        return (false, format!("Действие «{id}» — для другой системы, здесь его не выполняю."));
    }
    if let Err(e) = validate(id, arg) {
        log(&e);
        return (false, e);
    }
    // одно и то же действие — по одному разу (двойной щелчок не запустит вторую копию, второе окно пароля)
    let _busy = match Busy::take(id) {
        Ok(b) => b,
        Err(m) => return (false, m),
    };
    let (ok, msg) = apply_inner(id, arg);
    // в пробном режиме ничего не делали — так и пишем
    if dry_run() && ok {
        return (ok, format!("ПРОБНО (на деле ничего не менялось): {msg}"));
    }
    (ok, msg)
}

fn apply_inner(id: &str, arg: Option<&str>) -> (bool, String) {
    match id {
        "net_reconnect" => net_reconnect(),
        "dns_flush" => dns_flush(),
        "proxy_off" => proxy_off(),
        "happ_restart" => restart_app(&["Happ"], "Happ"),
        "outline_restart" => restart_app(&["Outline", "outline-client"], "outline"),
        "proton_restart" => restart_app(&["protonvpn-app"], "proton.vpn.app.gtk"),
        "svc_restart" => match arg {
            Some(u) if !u.is_empty() => svc_restart(u),
            _ => (false, "Не указана служба.".into()),
        },
        "prime_nvidia" => prime_nvidia(),
        "ext_restart" => ext_restart(arg.unwrap_or("")),
        "ext_enable" => ext_enable(arg.unwrap_or("")),
        "time_sync" => time_sync(),
        "dock_repair" => dock_repair(arg),
        "shell_restart" => shell_restart(),
        "theme_restore" => match arg {
            Some(k) if !k.is_empty() => theme_restore(k),
            _ => (false, "Не указан ключ темы.".into()),
        },
        #[cfg(aptechka_custom)]
        m if m.starts_with("my_") => crate::custom::apply(m, arg),
        "snapshot" => snapshot(),
        #[cfg(windows)]
        w if w.starts_with("win_") => win_apply(w, arg),
        #[cfg(target_os = "macos")]
        m if m.starts_with("mac_") => mac_apply(m, arg),
        // частые поломки (more.rs)
        "sound_restart" => {
            let (ok, m) = sh(&["systemctl", "--user", "restart", "pipewire", "pipewire-pulse", "wireplumber"], 30);
            (
                ok,
                if ok {
                    "Звук перезапущен. Если не появился — выбери устройство в настройках звука.".into()
                } else {
                    m
                },
            )
        }
        "sound_unmute" => done(sh(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "0"], 10), "Звук включён."),
        "sound_volume" => done(sh(&["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "0.5"], 10), "Громкость 50%."),
        "mic_unmute" => done(sh(&["wpctl", "set-mute", "@DEFAULT_AUDIO_SOURCE@", "0"], 10), "Микрофон включён."),
        "bt_unblock" => {
            let (ok, m) = sh_or_admin(&["rfkill", "unblock", "bluetooth"], 10, 60);
            (ok, if ok { "Bluetooth разблокирован.".into() } else { m })
        }
        "bt_power" => done(sh(&["bluetoothctl", "power", "on"], 15), "Адаптер Bluetooth включён."),
        "wifi_on" => done(sh(&["nmcli", "radio", "wifi", "on"], 15), "Wi-Fi включён."),
        "printer_enable" => match arg {
            Some(p) if !p.is_empty() => {
                let (ok, m) = sh_or_admin(&["cupsenable", p], 15, 60);
                (ok, if ok { format!("Принтер {p} снова печатает.") } else { m })
            }
            _ => (false, "Не указан принтер.".into()),
        },
        "print_clear" => print_clear(),
        "kb_switch" => kb_switch(),
        "dpkg_configure" => done(sh_admin("dpkg", &["--configure", "-a"], 1800, crate::elevate::Profile::Packages), "Установка доделана."),
        "apt_fix" => done(sh_admin("apt-get", &["-f", "install", "-y"], 1800, crate::elevate::Profile::Packages), "Зависимости починены."),
        "apt_autoremove" => remove_old_kernels(),
        "trash_empty" => done(sh(&["gio", "trash", "--empty"], 300), "Корзина очищена."),
        "thumbs_clear" => thumbs_clear(),
        "journal_vacuum" => done(sh_admin("journalctl", &["--vacuum-size=200M"], 300, crate::elevate::Profile::Service), "Журнал сжат до 200 МБ."),
        "dupes_scan" => crate::dupes::fix_scan(),
        "footprint_lock" => crate::footprint::fix_lock(),
        "footprint_history" => crate::footprint::fix_history(),
        "env_proxy_off" => crate::netproxy::comment_env_line(arg.unwrap_or("")),
        "git_proxy_off" => match arg {
            Some(k) if k == "http.proxy" || k == "https.proxy" => done(sh(&["git", "config", "--global", "--unset", k], 10), "Прокси у git убран."),
            _ => (false, "Не указан ключ git.".into()),
        },
        "npm_proxy_off" => {
            let (a, ma) = sh(&["npm", "config", "delete", "proxy"], 30);
            let (b, mb) = sh(&["npm", "config", "delete", "https-proxy"], 30);
            // обе настройки, а не одна из двух (иначе половина прокси оставалась, а писалось «убран»)
            match (a, b) {
                (true, true) => (true, "Прокси у npm убран.".into()),
                (true, false) => (false, format!("proxy убран, а https-proxy — нет: {mb}")),
                (false, true) => (false, format!("https-proxy убран, а proxy — нет: {ma}")),
                (false, false) => (false, ma),
            }
        }
        "dupes_trash" if arg.is_some_and(|a| a.starts_with("all:")) => crate::dupes::fix_trash_all(arg.unwrap_or("").trim_start_matches("all:")),
        "dupes_trash" if arg == Some("all") => (false, "Список копий устарел — найди копии заново.".into()),
        "dupes_trash" => match arg {
            Some(id) if !id.is_empty() => crate::dupes::fix_trash(id),
            _ => (false, "Не указана группа копий.".into()),
        },
        // страница входа в сеть (кафе, гостиница) — открыть в браузере
        "open_url" => open_url(arg.unwrap_or("")),
        // «PID» или «PID@время_запуска» (так строит кнопку check_hogs)
        "proc_term" => {
            let (p, t) = arg.map(|a| a.split_once('@').map_or((a, None), |(p, t)| (p, t.parse::<u64>().ok()))).unwrap_or(("", None));
            match p.parse::<i64>() {
                Ok(pid) => proc_term(pid, t),
                Err(_) => (false, "Не указан процесс.".into()),
            }
        }
        other => (false, format!("Неизвестное действие: {other}")),
    }
}

/// Действия, которые доступны всегда, а не только у конкретной проверки (меню «Лечение») — свои у каждой системы.
#[cfg(target_os = "linux")]
pub const GENERAL: &[(&str, &str)] = &[
    ("proxy_off", "Выключить системный прокси"),
    ("dns_flush", "Сбросить кэш DNS"),
    ("net_reconnect", "Переподключить сеть"),
    ("dock_repair", "Починить док"),
    ("shell_restart", "Перезапустить GNOME Shell"),
];
#[cfg(windows)]
pub const GENERAL: &[(&str, &str)] = &[
    ("win_proxy_off", "Выключить системный прокси"),
    ("win_dns_flush", "Сбросить кэш DNS"),
    ("win_explorer_restart", "Перезапустить Проводник и панель задач"),
];
#[cfg(target_os = "macos")]
pub const GENERAL: &[(&str, &str)] = &[
    ("mac_proxy_off", "Выключить системный прокси"),
    ("mac_dns_flush", "Сбросить кэш DNS"),
    ("mac_finder_restart", "Перезапустить Finder"),
    ("mac_dock_restart", "Перезапустить Dock"),
    ("mac_menubar_restart", "Перезапустить строку меню"),
];

// ───────────────────────────── Windows ─────────────────────────────

/// PowerShell одной командой (без окна).
#[cfg(windows)]
fn psh(cmd: &str, timeout: u64) -> (bool, String) {
    // любая ошибка — стоп и ненулевой код (иначе ошибка в середине терялась, а код был от последней команды)
    let full = format!("$ErrorActionPreference='Stop'; {cmd}");
    sh(&["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &full], timeout)
}

/// То же с правами администратора: обычный запрос UAC. Отказ — «Отменено».
#[cfg(windows)]
fn psh_admin(cmd: &str, timeout: u64) -> (bool, String) {
    // команда — закодированной (UTF-16LE в base64): в ней нет кавычек, которые можно было бы разорвать;
    // PowerShell — по полному системному пути, а не первым попавшимся в PATH
    let enc = crate::elevate::ps_encoded(&format!("$ErrorActionPreference='Stop'; {cmd}"));
    let wrap = format!(
        "try {{ $p = Start-Process \"$env:SystemRoot\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-EncodedCommand','{enc}'; exit $p.ExitCode }} catch {{ Write-Output 'CANCELLED'; exit 1223 }}"
    );
    let (ok, m) = psh(&wrap, timeout);
    if m.contains("CANCELLED") {
        return (false, "Отменено: права администратора не дали.".into());
    }
    (ok, m)
}

/// Выключить прокси WinINet и сразу сообщить системе (InternetSetOption 39 и 37) — без перезагрузки.
#[cfg(windows)]
const WIN_PROXY_OFF: &str = "$k='HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings'; Set-ItemProperty $k -Name ProxyEnable -Value 0; \
Remove-ItemProperty $k -Name AutoConfigURL -ErrorAction SilentlyContinue; Set-ItemProperty $k -Name AutoDetect -Value 0 -ErrorAction SilentlyContinue; \
$t = Add-Type -MemberDefinition '[DllImport(\"wininet.dll\")] public static extern bool InternetSetOption(IntPtr h, int o, IntPtr b, int l);' -Name W -Namespace Aptechka -PassThru; \
[void]$t::InternetSetOption(0, 39, 0, 0); [void]$t::InternetSetOption(0, 37, 0, 0)";

#[cfg(windows)]
fn win_apply(id: &str, arg: Option<&str>) -> (bool, String) {
    let a = arg.unwrap_or("");
    let ok_svc = |n: &str| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || " -_".contains(c));
    match id {
        "win_theme_restore" => crate::looksnap::restore(a),
        "win_proxy_off" => done(psh(WIN_PROXY_OFF, 30), "Системный прокси выключен. Браузер может понадобиться перезапустить."),
        "win_dns_flush" => done(sh(&["ipconfig", "/flushdns"], 20), "Кэш DNS очищен."),
        "win_winhttp_reset" => done(psh_admin("netsh winhttp reset proxy", 60), "Прокси WinHTTP сброшен."),
        "win_svc_restart" if ok_svc(a) => done(psh_admin(&format!("Restart-Service -Force -Name '{a}'"), 90), &format!("Служба {a} перезапущена.")),
        "win_svc_restart" => (false, "Не указана служба.".into()),
        // не трогаем обновления посреди установки: иначе установка оборвётся и сломает больше, чем починит
        "win_update_reset" if crate::procs::list().iter().any(|p| ["TiWorker.exe", "TrustedInstaller.exe", "wuauclt.exe", "MoUsoCoreWorker.exe", "UsoClient.exe"].iter().any(|n| p.name.eq_ignore_ascii_case(n))) => {
            (false, "Сейчас Windows устанавливает обновления — подожди, пока закончит, и попробуй снова.".into())
        }
        "win_update_reset" => done(
            psh_admin(
                // любая ошибка — стоп (иначе ошибка Rename-Item не меняла код выхода и писалось «сброшены»);
                // прежнюю копию не удаляем — новое имя с датой; службы запускаем в любом случае
                "$ErrorActionPreference='Stop'; try { Stop-Service wuauserv,bits,cryptsvc -Force; $d=\"$env:SystemRoot\\SoftwareDistribution\"; $n=\"SoftwareDistribution.$(Get-Date -Format yyyyMMdd-HHmmss)\"; Rename-Item -LiteralPath $d -NewName $n; if (Test-Path -LiteralPath $d) { throw 'not renamed' } } finally { Start-Service cryptsvc,bits,wuauserv -ErrorAction SilentlyContinue }",
                300,
            ),
            "Компоненты обновления сброшены (старые — в папке SoftwareDistribution с датой). Запусти проверку обновлений.",
        ),
        "win_explorer_restart" => done(psh("Stop-Process -Name explorer -Force; Start-Sleep 2; if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer }", 30), "Проводник перезапущен."),
        // a = «номер:имя» (имя — чтобы не закрыть другую программу, получившую тот же номер); просто номер — по-старому
        "win_kill" => {
            let (p, want) = a.split_once(':').map_or((a, None), |(p, n)| (p, Some(n)));
            match p.parse::<u32>() {
                Ok(pid) if pid > 4 && pid != std::process::id() => {
                    // открытый handle держит именно этот процесс: пока он открыт, номер не достанется другому, а
                    // имя и закрытие — у одного и того же объекта (раньше: tasklist по номеру, потом taskkill по номеру)
                    match win_proc::open(pid) {
                        Err(87) => (true, "Программа уже закрыта.".into()),
                        Err(5) => (false, "Windows не дала открыть эту программу (отказано в доступе) — закрыть её может только администратор.".into()),
                        Err(_) => (false, "Не удалось открыть программу, чтобы её закрыть — она могла остаться запущенной.".into()),
                        Ok(h) => {
                            let name = h.exe_name();
                            if name.is_empty() || crate::procs::is_protected(&name) {
                                return (false, format!("{} — часть системы, её закрывать нельзя.", if name.is_empty() { "Этот процесс" } else { &name }));
                            }
                            if want.is_some_and(|w| !w.eq_ignore_ascii_case(&name)) {
                                return (true, "Та программа уже закрылась (под этим номером теперь другая — её не трогаю).".into());
                            }
                            if dry_run() {
                                log_dry(&format!("закрыть {name} ({pid}) через его handle"));
                                return (true, "Программа закрыта.".into());
                            }
                            if h.terminate() { (true, "Программа закрыта.".into()) } else { (false, "Windows не дала закрыть программу (нет прав?).".into()) }
                        }
                    }
                }
                Ok(_) => (false, "Этот процесс закрывать нельзя.".into()),
                Err(_) => (false, "Не указан процесс.".into()),
            }
        }
        "win_time_sync" => clock_after(psh_admin("Start-Service w32time -ErrorAction SilentlyContinue; w32tm /resync /force", 90)),
        "win_cleanmgr" => {
            if dry_run() {
                log_dry("открыть «Очистку диска» (cleanmgr)");
                return (true, "Открыла «Очистку диска».".into());
            }
            // окно «Очистки диска» живёт своей жизнью — не ждём его
            let r = crate::diag::quiet(&mut std::process::Command::new("cleanmgr")).spawn();
            (r.is_ok(), if r.is_ok() { "Открыла «Очистку диска».".into() } else { "Не удалось открыть «Очистку диска».".into() })
        }
        other => (false, format!("Неизвестное действие: {other}")),
    }
}

// ───────────────────────────── macOS ─────────────────────────────

/// Команда с правами администратора: стандартное окно пароля macOS (osascript … with administrator privileges).
#[cfg(target_os = "macos")]
fn mac_admin(cmd: &str, timeout: u64) -> (bool, String) {
    // скрипт — аргументом AppleScript, а не вклеенным в его текст: кавычки и «\» в данных не станут кодом
    let (ok, m) = sh(
        &["/usr/bin/osascript", "-e", "on run argv", "-e", "do shell script (item 1 of argv) with administrator privileges", "-e", "end run", cmd],
        timeout,
    );
    if !ok && (m.contains("-128") || m.contains("canceled") || m.contains("отмен")) {
        return (false, "Отменено: пароль администратора не введён.".into());
    }
    (ok, m)
}

#[cfg(target_os = "macos")]
fn mac_apply(id: &str, arg: Option<&str>) -> (bool, String) {
    match id {
        "mac_theme_restore" => crate::looksnap::restore(arg.unwrap_or("")),
        "mac_proxy_off" => {
            let script = crate::mac::proxy_off_script();
            let (ok, m) = sh(&["sh", "-c", &script], 60);
            // обычному пользователю networksetup может отказать — тогда спросим пароль
            let (ok, m) = if ok && !m.to_lowercase().contains("privilege") { (ok, m) } else { mac_admin(&script, 120) };
            // результат — по факту: scutil не показывает ни одного включённого прокси
            if ok && !dry_run() && !crate::mac::proxy_all_off() {
                return (false, "Команды прошли, но система всё ещё показывает включённый прокси.".into());
            }
            done((ok, m), "Системный прокси выключен на всех сетевых службах.")
        }
        "mac_dns_flush" => done(mac_admin("/usr/bin/dscacheutil -flushcache && /usr/bin/killall -HUP mDNSResponder", 60), "Кэш DNS очищен."),
        "mac_finder_restart" => done(sh(&["killall", "Finder"], 15), "Finder перезапущен."),
        "mac_dock_restart" => done(sh(&["killall", "Dock"], 15), "Dock перезапущен."),
        "mac_menubar_restart" => {
            let (a, m) = sh(&["killall", "SystemUIServer"], 15);
            let (b, _) = sh(&["killall", "ControlCenter"], 15);
            (a || b, if a || b { "Строка меню перезапущена.".into() } else { m })
        }
        "mac_audio_restart" => done(mac_admin("killall coreaudiod", 60), "Звук перезапущен."),
        "mac_time_sync" => clock_after(mac_admin("/usr/bin/sntp -sS time.apple.com", 90)),
        "mac_thin_snapshots" => done(mac_admin("tmutil thinlocalsnapshots / 999999999999 4", 300), "Локальные снимки убраны."),
        "mac_force_quit" => match arg.and_then(|a| a.split_once('@')).and_then(|(p, k)| Some((p.parse::<u32>().ok()?, k))) {
            Some((pid, key)) if pid > 1 && pid != std::process::id() => {
                let ps = pid.to_string();
                // жив ли — по ps, а не kill -0: тот «не получается» и когда процесс чужой (нет прав), а не только когда его нет
                let alive =
                    || crate::cmd::exec(&["ps", "-p", &ps, "-o", "pid="], std::time::Duration::from_secs(5), crate::cmd::CHECK).text().trim() == ps;
                let info = crate::cmd::exec(&["ps", "-p", &ps, "-o", "uid=,comm="], std::time::Duration::from_secs(5), crate::cmd::CHECK);
                let line = info.text().trim().to_string();
                if line.is_empty() {
                    return (true, "Программа уже закрыта.".into());
                }
                let (uid, comm) = line.split_once(char::is_whitespace).unwrap_or((&line, ""));
                let comm = comm.trim();
                if crate::procs::is_protected(comm) {
                    return (false, "Это часть системы, её закрывать нельзя.".into());
                }
                let me = crate::cmd::exec(&["id", "-u"], std::time::Duration::from_secs(5), crate::cmd::CHECK);
                if me.text().trim() != uid.trim() {
                    return (false, "Это программа другого пользователя — закрыть её может только он или администратор.".into());
                }
                // время запуска: сверяем перед каждым сигналом — если номер за это время достался другому
                // процессу, он не получит ни TERM, ни KILL (у macOS нет pidfd; остаток гонки — доли секунды)
                let started = || {
                    crate::cmd::exec(&["ps", "-p", &ps, "-o", "lstart="], std::time::Duration::from_secs(5), crate::cmd::CHECK)
                        .text()
                        .trim()
                        .to_string()
                };
                let born = started();
                if born.is_empty() {
                    return (true, "Программа уже закрыта.".into());
                }
                if mac_start_key(&born) != key {
                    return (false, "Под этим номером теперь другая программа — её не трогаю. Проверь ещё раз.".into());
                }
                extern "C" {
                    fn kill(pid: i32, sig: i32) -> i32;
                }
                let send = |sig: i32| started() == born && unsafe { kill(pid as i32, sig) } == 0;
                if dry_run() {
                    log_dry(&format!("TERM процессу {pid} ({comm})"));
                    return (true, "Программа закрыта.".into());
                }
                if !send(15) {
                    return (true, "Программа уже закрылась (или под этим номером теперь другая — её не трогаю).".into());
                }
                for _ in 0..15 {
                    pause(200);
                    if !alive() || started() != born {
                        return (true, "Программа закрыта.".into());
                    }
                }
                if !send(9) {
                    return (true, "Программа закрыта.".into());
                }
                pause(300);
                if !alive() || started() != born {
                    (true, "Программа закрыта принудительно.".into())
                } else {
                    (false, "Программа не закрылась.".into())
                }
            }
            Some(_) => (false, "Этот процесс закрывать нельзя.".into()),
            None => (false, "Не указан процесс.".into()),
        },
        other => (false, format!("Неизвестное действие: {other}")),
    }
}

/// Процесс Windows по handle: имя программы и закрытие — одного и того же объекта.
#[cfg(windows)]
mod win_proc {
    use core::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn QueryFullProcessImageNameW(h: *mut c_void, flags: u32, buf: *mut u16, size: *mut u32) -> i32;
        fn TerminateProcess(h: *mut c_void, code: u32) -> i32;
        fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }
    pub struct H(*mut c_void);

    /// Err(код Windows): 87 (ERROR_INVALID_PARAMETER) — такого процесса нет; 5 (отказ в доступе) и прочее — процесс есть, но не открыть.
    pub fn open(pid: u32) -> Result<H, i32> {
        // PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE
        let h = unsafe { OpenProcess(0x0001 | 0x1000 | 0x0010_0000, 0, pid) };
        if h.is_null() {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1))
        } else {
            Ok(H(h))
        }
    }

    impl H {
        pub fn exe_name(&self) -> String {
            let mut buf = vec![0u16; 1024];
            let mut n = buf.len() as u32;
            if unsafe { QueryFullProcessImageNameW(self.0, 0, buf.as_mut_ptr(), &mut n) } == 0 {
                return String::new();
            }
            let full = String::from_utf16_lossy(&buf[..(n as usize).min(buf.len())]);
            full.rsplit('\\').next().unwrap_or("").to_string()
        }
        pub fn terminate(&self) -> bool {
            unsafe { TerminateProcess(self.0, 1) != 0 && WaitForSingleObject(self.0, 5000) == 0 }
        }
    }

    impl Drop for H {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Та же починка руками: команда для терминала этой системы (Linux и macOS — bash/zsh,
/// Windows — PowerShell). None — у действия нет терминального варианта.
pub fn manual(id: &str, arg: Option<&str>) -> Option<String> {
    // та же проверка, что у кнопки: команду со странным параметром человеку не показываем
    validate(id, arg).ok()?;
    #[cfg(aptechka_custom)]
    if id.starts_with("my_") {
        return crate::custom::manual(id, arg);
    }
    let a = arg.unwrap_or("");
    #[cfg(target_os = "linux")]
    {
        let q = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
        Some(match id {
            // то же, что кнопка: переподключить устройство, через которое идёт интернет (а не выключать всю сеть)
            "net_reconnect" => "nmcli device reconnect \"$(ip route get 1.1.1.1 | sed -n 's/.* dev \\([^ ]*\\).*/\\1/p')\"".into(),
            "dns_flush" => "resolvectl flush-caches".into(),
            "proxy_off" => "gsettings set org.gnome.system.proxy mode 'none'; unset http_proxy https_proxy all_proxy".into(),
            "happ_restart" => restart_by_name("Happ", "Happ"),
            "outline_restart" => restart_by_name("Outline", "outline"),
            "proton_restart" => restart_by_name("protonvpn-app", "proton.vpn.app.gtk"),
            "svc_restart" => format!("sudo systemctl restart {}", q(a)),
            "prime_nvidia" => "sudo prime-select nvidia".into(),
            "ext_restart" => format!("gnome-extensions disable -- {0} && sleep 1 && gnome-extensions enable -- {0}", q(a)),
            "ext_enable" => format!("gnome-extensions enable -- {}", q(a)),
            "time_sync" => "sudo timedatectl set-ntp true".into(),
            "dock_repair" => {
                let u = if a.is_empty() { crate::diag::current_dock()? } else { a.to_string() };
                format!("gnome-extensions disable {0} && sleep 1 && gnome-extensions enable {0}", q(&u))
            }
            "shell_restart" => "killall -3 gnome-shell   # только X11; на Wayland — выйти и войти".into(),
            "sound_restart" => "systemctl --user restart pipewire pipewire-pulse wireplumber".into(),
            "sound_unmute" => "wpctl set-mute @DEFAULT_AUDIO_SINK@ 0".into(),
            "sound_volume" => "wpctl set-volume @DEFAULT_AUDIO_SINK@ 0.5".into(),
            "mic_unmute" => "wpctl set-mute @DEFAULT_AUDIO_SOURCE@ 0".into(),
            "bt_unblock" => "rfkill unblock bluetooth".into(),
            "bt_power" => "bluetoothctl power on".into(),
            "wifi_on" => "nmcli radio wifi on".into(),
            "printer_enable" => format!("cupsenable {}", q(a)),
            "print_clear" => "cancel -u \"$USER\" -x".into(),
            // как кнопка: добавить Super+Пробел к тому, что есть (в терминале — вручную в Настройках проще)
            "kb_switch" => "gnome-control-center keyboard   # «Переключить на следующий источник ввода» → Super+Пробел".into(),
            "dpkg_configure" => "sudo dpkg --configure -a".into(),
            "apt_fix" => "sudo apt-get -f install".into(),
            "apt_autoremove" => {
                let k = old_kernel_packages();
                if k.is_empty() {
                    "# старых ядер нет".into()
                } else {
                    format!("sudo apt-get purge {}", k.join(" "))
                }
            }
            "trash_empty" => "gio trash --empty".into(),
            "thumbs_clear" => "rm -rf \"${XDG_CACHE_HOME:-$HOME/.cache}\"/thumbnails/*".into(),
            "journal_vacuum" => "sudo journalctl --vacuum-size=200M".into(),
            "proc_term" => confirm_kill(a.split('@').next().unwrap_or("").trim().parse::<u32>().ok()?, ""),
            "env_proxy_off" => {
                let (f, n, _) = crate::netproxy::split_env_spec(a)?;
                // --follow-symlinks: если файл — ссылка, sed правит настоящий файл, а не заменяет ссылку копией
                format!("sed --follow-symlinks -i.bak '{n}s/^/# /' {}", q(f))
            }
            "git_proxy_off" => format!("git config --global --unset {a}"),
            "npm_proxy_off" => "npm config delete proxy; npm config delete https-proxy".into(),
            "dupes_trash" => {
                let (groups, _, _) = crate::dupes::load()?;
                let picked: Vec<_> = if a.starts_with("all") { groups.iter().collect() } else { vec![groups.iter().find(|g| g.id() == a)?] };
                let files: Vec<String> = picked.iter().flat_map(|g| g.files[1..].iter()).map(|p| q(&p.to_string_lossy())).collect();
                format!("gio trash {}", files.join(" "))
            }
            "theme_restore" => {
                let snap = crate::diag::load_snapshot();
                let val = snap.key(a)?.to_string();
                let mut p = a.splitn(3, '|');
                let (kind, x, y) = (p.next()?, p.next()?, p.next()?);
                if kind == "gsettings" {
                    format!("gsettings set {x} {y} {}", q(val.trim_matches('\'')))
                } else {
                    format!("dconf write {x} {}", q(&val))
                }
            }
            _ => return None,
        })
    }
    #[cfg(windows)]
    {
        Some(match id {
            "win_proxy_off" => "Set-ItemProperty 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings' ProxyEnable 0".into(),
            "win_theme_restore" => crate::looksnap::restore_cmd(a, crate::diag::load_snapshot().key(a)?)?,
            "win_winhttp_reset" => "netsh winhttp reset proxy   # PowerShell от администратора".into(),
            "win_svc_restart" => format!("Restart-Service -Force {}   # от администратора", crate::agents::ps_quote(a)),
            "win_update_reset" => "Stop-Service wuauserv,bits -Force; Rename-Item $env:SystemRoot\\SoftwareDistribution SoftwareDistribution.old; Start-Service bits,wuauserv".into(),
            "win_explorer_restart" => "Stop-Process -Name explorer -Force; Start-Process explorer".into(),
            "win_kill" => {
                // как кнопка: закрыть только если под этим номером всё ещё та же программа (иначе номер мог достаться другой)
                let (p, name) = a.split_once(':').map_or((a, ""), |(p, n)| (p, n));
                let pid = p.trim().parse::<u32>().ok()?;
                let base = name.trim();
                let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
                if base.is_empty() {
                    format!("Get-Process -Id {pid} | Stop-Process -Confirm")
                } else {
                    format!("Get-Process -Id {pid} | Where-Object ProcessName -eq {} | Stop-Process -Confirm", crate::agents::ps_quote(base))
                }
            }
            "win_dns_flush" => "ipconfig /flushdns".into(),
            "win_time_sync" => "w32tm /resync /force   # от администратора".into(),
            "win_cleanmgr" => "cleanmgr".into(),
            _ => return None,
        })
    }
    #[cfg(target_os = "macos")]
    {
        Some(match id {
            "mac_proxy_off" => crate::mac::proxy_off_script(),
            "mac_theme_restore" => crate::looksnap::restore_cmd(a, crate::diag::load_snapshot().key(a)?)?,
            "mac_dns_flush" => "sudo dscacheutil -flushcache; sudo killall -HUP mDNSResponder".into(),
            "mac_finder_restart" => "killall Finder".into(),
            "mac_dock_restart" => "killall Dock".into(),
            "mac_menubar_restart" => "killall SystemUIServer; killall ControlCenter".into(),
            "mac_audio_restart" => "sudo killall coreaudiod".into(),
            "mac_time_sync" => "sudo sntp -sS time.apple.com".into(),
            "mac_thin_snapshots" => "tmutil thinlocalsnapshots / 999999999999 4".into(),
            "mac_force_quit" => {
                let p = a.split('@').next().unwrap_or("").trim().parse::<u32>().ok()?;
                confirm_kill(p, "   # still running? run this line again; if it stays, use Activity Monitor > Force Quit")
            }
            _ => return None,
        })
    }
    #[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
    {
        let _ = (id, a);
        None
    }
}

/// Для ручного закрытия по номеру: сначала показать, что за программа под этим номером, и спросить
/// (номер мог достаться другой программе). Текст без русских слов — как есть в любом языке интерфейса.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn confirm_kill(pid: u32, tail: &str) -> String {
    format!("ps -o pid=,comm= -p {pid} && printf 'Close this program? [y/N] ' && read a && [ \"$a\" = y ] && kill {pid}{tail}")
}

/// Ручной перезапуск клиента: закрыть по точному имени, открыть заново только если старая копия закрылась
/// (иначе получится две копии, а при неудаче закрытия — ничего не потеряно).
#[cfg(target_os = "linux")]
fn restart_by_name(proc_name: &str, desktop_id: &str) -> String {
    format!("pkill -x {proc_name}; sleep 2; if pgrep -x {proc_name} >/dev/null; then echo 'still running - close it by hand'; else gtk-launch {desktop_id}; fi")
}

/// Что произойдёт при починке — для окна подтверждения (у действий без особых последствий).
pub fn what_happens(id: &str) -> &'static str {
    #[cfg(aptechka_custom)]
    if id.starts_with("my_") {
        return crate::custom::what(id);
    }
    match id {
        "dns_flush" => "Забуду запомненные адреса сайтов, они узнаются заново.",
        "footprint_lock" => "Файлы с ключами и доступами станут видны только тебе (другим учётным записям — нет).",
        "footprint_history" => "Из истории команд пропадут строки, где были ключи. Вернуть их будет нельзя — так и задумано.",
        "proxy_off" => "Интернет пойдёт напрямую, без прокси. VPN-клиент сможет включить его снова.",
        "ext_enable" => "Включу расширение обратно.",
        "ext_restart" => "Выключу и сразу включу расширение.",
        "theme_restore" | "win_theme_restore" | "mac_theme_restore" => "Верну эту настройку как в эталоне.",
        "win_proxy_off" => "Выключу системный прокси Windows — интернет пойдёт напрямую.",
        "mac_proxy_off" => "Выключу прокси на всех сетевых службах — интернет пойдёт напрямую. Если macOS попросит — введи пароль.",
        "win_cleanmgr" => "Открою встроенную «Очистку диска».",
        "win_dns_flush" => "Забуду запомненные адреса сайтов, они узнаются заново.",
        "sound_unmute" => "Включу звук.",
        "sound_volume" => "Поставлю громкость 50%.",
        "mic_unmute" => "Включу микрофон.",
        "bt_unblock" => "Сниму программную блокировку Bluetooth.",
        "bt_power" => "Включу адаптер Bluetooth.",
        "wifi_on" => "Включу Wi-Fi.",
        "printer_enable" => "Сниму принтер с паузы.",
        "kb_switch" => "Верну переключение раскладки по Super+Пробел.",
        "thumbs_clear" => "Удалю картинки-превью файлов, они пересоздадутся сами.",
        "env_proxy_off" => "Закомментирую эту строку (рядом сохраню копию файла). Подействует в новых окнах терминала.",
        "git_proxy_off" => "Уберу прокси из настроек git.",
        "npm_proxy_off" => "Уберу прокси из настроек npm.",
        _ => "Сделаю это сама.",
    }
}

fn done((ok, m): (bool, String), msg: &str) -> (bool, String) {
    (ok, if ok { msg.to_string() } else { m })
}

/// Очистить кэш миниатюр (~/.cache/thumbnails) — только содержимое, сама папка остаётся.
fn thumbs_clear() -> (bool, String) {
    // без домашней папки путь стал бы «.cache/thumbnails» в текущей папке — не трогаем ничего
    // папка кэша — как у всех программ: $XDG_CACHE_HOME или ~/.cache; только абсолютный путь
    // (без него путь стал бы «.cache/thumbnails» в текущей папке — не трогаем ничего)
    let cache = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|p| p.starts_with('/'))
        .or_else(|| std::env::var("HOME").ok().filter(|h| h.starts_with('/')).map(|h| format!("{h}/.cache")));
    let Some(cache) = cache else { return (false, "Не нашла домашнюю папку.".into()) };
    let dir = std::path::PathBuf::from(cache).join("thumbnails");
    // сама папка миниатюр — ссылка (на другой диск или подменили) или не папка: не трогаем — иначе стёрлось бы
    // то, на что она ведёт, и без Корзины
    match std::fs::symlink_metadata(&dir) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => {
            return (false, format!("{} — ссылка, а не папка; очищать не стану.", dir.display()));
        }
        Err(_) => return (true, "Кэш миниатюр уже пуст.".into()),
        _ => {}
    }
    if dry_run() {
        log(&format!("DRY_RUN: очистила бы {}", dir.display()));
        return (true, "Кэш миниатюр очищен.".into());
    }
    let mut n = 0;
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => return (false, format!("Не открыть папку миниатюр: {e}")),
    };
    {
        for e in rd.flatten() {
            let p = e.path();
            // ссылку удаляем как ссылку, не заходя в то, на что она указывает
            let real_dir = std::fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir());
            let r = if real_dir { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
            if r.is_ok() {
                n += 1;
            }
        }
    }
    log(&format!("миниатюры: удалено {n} папок"));
    // правда о результате: что-то осталось (нет прав, занято) — так и говорим
    // ошибка чтения — не «пусто»
    let left = match std::fs::read_dir(&dir) {
        Ok(rd) => rd.count(),
        Err(e) => return (false, format!("Удалила {n}, а проверить, что осталось, не вышло: {e}")),
    };
    if left == 0 {
        (true, "Кэш миниатюр очищен.".into())
    } else {
        (false, format!("Удалила {n}, а {left} не удалось (нет прав или заняты) — посмотри папку {}.", dir.display()))
    }
}

/// Открыть адрес в браузере. Только http(s) и без пробелов и кавычек; адрес — отдельным аргументом.
fn open_url(url: &str) -> (bool, String) {
    let ok = (url.starts_with("http://") || url.starts_with("https://")) && !url.chars().any(|c| c.is_whitespace() || "\"'`<>|^".contains(c));
    if !ok {
        return (false, "Странный адрес — открывать не буду.".into());
    }
    let r = if cfg!(windows) {
        sh_app(&["rundll32", "url.dll,FileProtocolHandler", url], 10)
    } else if cfg!(target_os = "macos") {
        sh_app(&["open", url], 10)
    } else {
        sh_app(&["xdg-open", url], 10)
    };
    done(r, "Открыла страницу входа в браузере.")
}

/// Пакеты старых ядер: всё, что относится к версиям ядра, кроме работающей сейчас (uname -r) и самой новой.
/// Только пакеты с версией в имени (linux-image-6.8.0-45-generic и т.п.) — общие linux-generic не трогаем.
/// Раньше «Убрать старые ядра» запускала apt autoremove --purge -y: удаляла всё, что apt счёл лишним, не глядя.
/// Пакеты старых ядер, которые можно удалить. Оставляем: работающее ядро, и для каждого вида ядра
/// (generic, lowlatency, oem…) — самое новое и предыдущее (запасное, если новое не загрузится).
/// Версии сравнивает dpkg (как сам apt), а не разбор чисел.
fn old_kernel_packages() -> Vec<String> {
    let (_, running) = crate::diag::run(&["uname", "-r"], 5);
    let running = running.trim().to_string();
    let (code, list) = crate::diag::run(
        &[
            "dpkg-query",
            "-W",
            "-f=${Package} ${db:Status-Abbrev}\n",
            "linux-image-[0-9]*",
            "linux-headers-[0-9]*",
            "linux-modules-[0-9]*",
            "linux-modules-extra-[0-9]*",
        ],
        20,
    );
    if code != 0 || running.is_empty() {
        return Vec::new();
    }
    let installed: Vec<KPkg> = list
        .lines()
        .filter_map(|l| {
            let (pkg, st) = l.split_once(' ')?;
            st.trim().starts_with("ii").then(|| KPkg::parse(pkg)).flatten()
        })
        .collect();
    let keep = kernels_to_keep(&installed, &running, |a, b| {
        // dpkg --compare-versions a gt b: код 0 — да
        crate::diag::run(&["dpkg", "--compare-versions", a, "gt", b], 5).0 == 0
    });
    // заголовки без вида («linux-headers-6.8.0-45») нужны, пока жива любая сборка этой версии
    installed.into_iter().filter(|k| !keep.contains(&k.ver)).map(|k| k.pkg).collect()
}

/// Пакет ядра: имя, версия («6.8.0-45»), вид («generic»; у общих заголовков — пусто).
#[derive(Debug, Clone)]
struct KPkg {
    pkg: String,
    ver: String,
    flavor: String,
}

impl KPkg {
    /// «linux-modules-extra-6.8.0-45-generic» → версия 6.8.0-45, вид generic.
    fn parse(pkg: &str) -> Option<KPkg> {
        let i = pkg.find(|c: char| c.is_ascii_digit())?;
        let mut parts = pkg[i..].splitn(3, '-');
        let (base, abi) = (parts.next()?, parts.next()?);
        if !abi.chars().all(|c| c.is_ascii_digit()) || base.is_empty() {
            return None;
        }
        Some(KPkg { pkg: pkg.to_string(), ver: format!("{base}-{abi}"), flavor: parts.next().unwrap_or("").to_string() })
    }
}

/// Какие версии оставить: работающая; для каждого вида — две самые новые. `newer(a, b)` — a новее b.
fn kernels_to_keep(installed: &[KPkg], running: &str, newer: impl Fn(&str, &str) -> bool) -> Vec<String> {
    let mut keep: Vec<String> = KPkg::parse(&format!("linux-image-{running}")).map(|k| vec![k.ver]).unwrap_or_default();
    let mut flavors: Vec<&str> = installed.iter().filter(|k| k.pkg.starts_with("linux-image-")).map(|k| k.flavor.as_str()).collect();
    flavors.sort();
    flavors.dedup();
    for f in flavors {
        let mut vers: Vec<&str> = installed.iter().filter(|k| k.pkg.starts_with("linux-image-") && k.flavor == f).map(|k| k.ver.as_str()).collect();
        vers.sort();
        vers.dedup();
        // простая сортировка вставками по сравнению dpkg (ядер — единицы)
        let mut sorted: Vec<&str> = Vec::new();
        for v in vers {
            let at = sorted.iter().position(|s| newer(v, s)).unwrap_or(sorted.len());
            sorted.insert(at, v);
        }
        keep.extend(sorted.into_iter().take(2).map(String::from));
    }
    keep.sort();
    keep.dedup();
    keep
}

fn remove_old_kernels() -> (bool, String) {
    let pk = old_kernel_packages();
    if pk.is_empty() {
        return (true, "Старых ядер нет — удалять нечего.".into());
    }
    log(&format!("старые ядра: {}", pk.join(" ")));
    // пробный прогон apt: удалит ли он что-то кроме этих пакетов (например, метапакет linux-generic —
    // тогда обновления ядра перестанут приходить). Если да — не трогаем.
    let mut sim = vec!["apt-get", "-s", "purge"];
    sim.extend(pk.iter().map(String::as_str));
    let r = crate::cmd::exec(&sim, Duration::from_secs(120), crate::cmd::CHECK);
    if !r.answered() || r.code() != 0 {
        return (false, "Пробный прогон apt не удался — ничего не удаляю. Попробуй в терминале: sudo apt autoremove --purge".into());
    }
    let extra: Vec<&str> = r
        .text()
        .lines()
        .filter_map(|l| l.strip_prefix("Purg ").or_else(|| l.strip_prefix("Remv ")))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|p| !pk.iter().any(|k| k == p))
        .collect();
    if !extra.is_empty() {
        return (
            false,
            format!(
                "apt удалил бы заодно {} — это опасно, ничего не трогаю. Удали старые ядра в терминале: sudo apt autoremove --purge",
                extra.iter().take(5).copied().collect::<Vec<_>>().join(", ")
            ),
        );
    }
    // имена пакетов — только из букв, цифр и «.+-», не с «-» (иначе apt принял бы их за ключи)
    if !pk.iter().all(|p| !p.starts_with('-') && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".+-".contains(c))) {
        return (false, "Странные имена пакетов — ничего не удаляю.".into());
    }
    let mut args = vec!["purge", "-y"];
    args.extend(pk.iter().map(String::as_str));
    let n = pk.iter().filter(|p| p.starts_with("linux-image-")).count();
    done(
        sh_admin("apt-get", &args, 1800, crate::elevate::Profile::Packages),
        &format!("Убрала старые ядра ({n}): работающее и по два новых каждого вида остались."),
    )
}

#[cfg(test)]
#[test]
fn kernel_keep() {
    let pk = |p: &str| KPkg::parse(p).unwrap();
    let inst: Vec<KPkg> = [
        "linux-image-6.8.0-40-generic",
        "linux-image-6.8.0-45-generic",
        "linux-image-6.8.0-49-generic",
        "linux-image-6.8.0-100-generic",
        "linux-image-6.8.0-1012-oem",
        "linux-headers-6.8.0-40",
    ]
    .iter()
    .map(|p| pk(p))
    .collect();
    // сравнение как у dpkg для этих простых версий: по числам
    let key = |v: &str| v.split(['.', '-']).map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let keep = kernels_to_keep(&inst, "6.8.0-40-generic", |a, b| key(a) > key(b));
    assert_eq!(keep, vec!["6.8.0-100", "6.8.0-1012", "6.8.0-40", "6.8.0-49"]);
    assert_eq!(pk("linux-headers-6.8.0-40").flavor, "");
    assert!(KPkg::parse("linux-image-generic").is_none());
}

/// Закрыть программу: сначала вежливо (TERM), через 5 с — если жива — сообщить.
/// Только обычную программу этого пользователя: не 0/−1 (kill −1 закрыл бы ВСЕ программы), не саму Аптечку,
/// не системные (procs::is_protected), не чужие и не тот процесс, что успел смениться (started — время запуска).
fn proc_term(pid: i64, started: Option<u64>) -> (bool, String) {
    if pid <= 1 || pid == std::process::id() as i64 {
        return (false, "Этот процесс закрывать нельзя.".into());
    }
    // без времени запуска не знаем, тот ли это процесс, — не трогаем (раньше отсутствие сведений пропускало сверку)
    let Some(want) = started else {
        return (false, "Не знаю, тот ли это процесс, — проверь заново и нажми кнопку из свежей строки.".into());
    };
    #[cfg(target_os = "linux")]
    {
        // дескриптор процесса (pidfd) держит именно этот процесс: номер, освободившись, достанется другому,
        // а сигнал по дескриптору уйдёт только «нашему» — гонки «проверила номер → послала по номеру» нет
        let fd = match pidfd::open(pid) {
            Ok(fd) => fd,
            Err(pidfd::ESRCH) => return (true, "Программа уже закрыта.".into()),
            Err(_) => return (false, "Не удалось получить доступ к этой программе (ядро не даёт или нет прав) — не трогаю её.".into()),
        };
        // сверка уже после того, как дескриптор взят: если время запуска совпало, дескриптор — тот самый процесс
        if crate::procs::start_ticks(pid) != Some(want) {
            return (true, "Та программа уже закрылась (под этим номером теперь другая — её не трогаю).".into());
        }
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return (true, "Программа уже закрыта.".into());
        };
        let name = status.lines().find_map(|l| l.strip_prefix("Name:")).unwrap_or("").trim().to_string();
        if crate::procs::is_protected(&name) {
            return (false, format!("{name} — часть системы, её закрывать нельзя."));
        }
        let uid = status.lines().find_map(|l| l.strip_prefix("Uid:")).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse::<u32>().ok());
        let me = std::fs::metadata("/proc/self").ok().map(|m| std::os::unix::fs::MetadataExt::uid(&m));
        if uid.is_none() || uid != me {
            return (false, "Это программа другого пользователя — закрыть её может только он или администратор.".into());
        }
        if dry_run() {
            log_dry(&format!("сигнал TERM процессу {pid} ({name}) по его дескриптору"));
            return (true, "Программа закрыта.".into());
        }
        if !fd.signal(15) {
            return (false, "Не удалось послать программе сигнал закрыться.".into());
        }
        if fd.wait_exit(Duration::from_secs(5)) {
            return (true, "Программа закрыта.".into());
        }
        // без «kill -9 <номер>»: номер мог достаться другой программе. Принудительно закрывают по имени и
        // подтверждению — в менеджере процессов (там видно, что именно закрываешь)
        (false, format!("{name} не закрылась сама. Закрой её принудительно в менеджере процессов («Завершить принудительно»): там видно, что именно закрываешь."))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = want;
        (false, "Только для Linux.".into())
    }
}

/// Дескриптор процесса Linux (pidfd, ядро 5.3+): сигнал и ожидание конца — именно этому процессу.
#[cfg(target_os = "linux")]
mod pidfd {
    pub struct Fd(i32);

    extern "C" {
        fn syscall(n: i64, ...) -> i64;
        fn poll(fds: *mut PollFd, n: u64, timeout_ms: i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    // номера системных вызовов одинаковы на x86_64 и arm64 (общая таблица новых вызовов)
    const SYS_PIDFD_OPEN: i64 = 434;
    const SYS_PIDFD_SEND_SIGNAL: i64 = 424;

    pub const ESRCH: i32 = 3;

    /// Err(errno): ESRCH — процесса нет; ENOSYS/EPERM/прочее — процесс мог остаться, «закрыт» говорить нельзя.
    pub fn open(pid: i64) -> Result<Fd, i32> {
        let fd = unsafe { syscall(SYS_PIDFD_OPEN, pid as i32, 0u32) };
        if fd >= 0 {
            Ok(Fd(fd as i32))
        } else {
            Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(-1))
        }
    }

    impl Fd {
        pub fn signal(&self, sig: i32) -> bool {
            unsafe { syscall(SYS_PIDFD_SEND_SIGNAL, self.0, sig, std::ptr::null::<u8>(), 0u32) == 0 }
        }

        /// Дождаться конца процесса: pidfd становится «читаемым», когда процесс завершился.
        pub fn wait_exit(&self, d: std::time::Duration) -> bool {
            let mut p = PollFd { fd: self.0, events: 1, revents: 0 };
            unsafe { poll(&mut p, 1, d.as_millis() as i32) > 0 }
        }
    }

    impl Drop for Fd {
        fn drop(&mut self) {
            unsafe {
                close(self.0);
            }
        }
    }
}

/// После «сверить часы» — перемерить: успех только если часы правда сходятся с интернетом.
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn clock_after(r: (bool, String)) -> (bool, String) {
    if !r.0 || dry_run() {
        return if r.0 { (true, "Часы сверены.".into()) } else { r };
    }
    match crate::diag::clock_offset() {
        Some(off) if off.abs() < 60.0 => (true, format!("Часы сверены (расхождение {:+.0} с).", off.round() + 0.0)),
        Some(off) => (false, format!("Команда прошла, но часы всё ещё расходятся на {:.0} с — сервер времени, похоже, недоступен.", off.abs())),
        // не «сверены»: команда прошла, а результат не проверить
        None => (false, "Команда сверки прошла, но проверить результат не с чем: сайты не ответили. Посмотри строку «Системное время» позже.".into()),
    }
}

/// Ключ темы Windows/macOS из нашего списка (looksnap.rs); на других системах таких нет.
fn look_known(a: &str) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        crate::looksnap::known(a)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = a;
        false
    }
}

/// Строка в журнал пробного режима (для модулей, которые сами выполняют действия).
pub fn log_dry(what: &str) {
    log(&format!("DRY_RUN: выполнила бы: {what}"));
}

/// Подходит ли действие этой системе: win_* — Windows, mac_* — macOS, общие — везде, остальные — Linux.
/// (Иначе на Mac с Homebrew-gsettings запускалось бы лечение GNOME.)
pub fn for_this_os(id: &str) -> bool {
    // «snapshot» — эталон тем: на Linux (GNOME), Windows и macOS свой набор ключей (diag::capture_snapshot)
    const COMMON: [&str; 8] =
        ["dupes_scan", "dupes_trash", "git_proxy_off", "npm_proxy_off", "open_url", "footprint_lock", "footprint_history", "snapshot"];
    // свои действия (папка custom/) сами решают, на какой системе работают
    if COMMON.contains(&id) || id.starts_with("my_") {
        true
    } else {
        os_of(id) == current_os()
    }
}

/// Для какой системы действие: по приставке (win_, mac_), без приставки — Linux.
/// Отдельной функцией, а не цепочкой `if cfg!(…)`: автоматическое «упрощение» Clippy на Linux посчитало
/// cfg! константой и свело правило к «mac_ — нельзя нигде», после чего на macOS отказывали все действия.
fn os_of(id: &str) -> &'static str {
    if id.starts_with("win_") {
        "windows"
    } else if id.starts_with("mac_") {
        "macos"
    } else {
        "linux"
    }
}

fn current_os() -> &'static str {
    std::env::consts::OS
}

#[cfg(test)]
#[test]
fn actions_for_their_os() {
    assert_eq!(os_of("mac_force_quit"), "macos");
    assert_eq!(os_of("win_kill"), "windows");
    assert_eq!(os_of("dns_flush"), "linux");
    // на любой системе: свои действия разрешены, чужие — нет, общие — везде
    let mine = match current_os() {
        "macos" => "mac_dns_flush",
        "windows" => "win_dns_flush",
        _ => "dns_flush",
    };
    assert!(for_this_os(mine));
    for other in ["mac_dns_flush", "win_dns_flush", "dns_flush"].iter().filter(|x| **x != mine) {
        assert!(!for_this_os(other), "{other}");
    }
    assert!(for_this_os("dupes_scan"));
}

#[cfg(test)]
#[test]
fn busy_once() {
    let a = Busy::take("тест_занято").unwrap();
    assert!(Busy::take("тест_занято").is_err());
    drop(a);
    assert!(Busy::take("тест_занято").is_ok());
}

#[cfg(test)]
#[test]
fn admin_only_on_denied() {
    for t in ["rfkill: Permission denied", "Operation not permitted", "Access is denied.", "must be root", "Interactive authentication required."] {
        assert!(denied(t), "{t}");
    }
    for t in ["", "Unknown operation", "Connection timed out", "no such device"] {
        assert!(!denied(t), "{t}");
    }
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn manual_kill_rechecks_name() {
    let m = manual("proc_term", Some("4242@100")).unwrap();
    assert!(m.starts_with("ps -o pid=,comm= -p 4242 &&") && m.contains("kill 4242"), "{m}");
    let m = manual("happ_restart", None).unwrap();
    assert!(m.contains("pgrep -x Happ") && m.contains("gtk-launch Happ"), "{m}");
}

#[cfg(test)]
mod mac_quit_tests {
    use super::*;

    #[test]
    fn mac_force_quit_needs_start_time() {
        assert_eq!(mac_start_key("Thu Nov  5 21:59:32 2026"), "Thu-Nov-5-21-59-32-2026");
        assert!(validate("mac_force_quit", Some("4242@Thu-Nov-5-21-59-32-2026")).is_ok());
        // без времени запуска, системные номера, лишние знаки — отказ
        for bad in ["4242", "1@Thu-Nov-5-21-59-32-2026", "0@Thu-Nov-5-21-59-32-2026", "4242@x", "4242@Thu Nov 5", "4242@a;b-cdefgh"] {
            assert!(validate("mac_force_quit", Some(bad)).is_err(), "{bad}");
        }
        // ручная команда для macOS собирается только на macOS
        #[cfg(target_os = "macos")]
        assert!(manual("mac_force_quit", Some("4242@Thu-Nov-5-21-59-32-2026")).is_some_and(|m| m.contains("4242")));
    }
}
