// Windows: окно без консоли; для --diag/--fix подключаемся к консоли, из которой запустили (attach_console)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Аптечка: окно с живым котом и проверками, виджет на рабочем столе, команды для терминала.
// Стиль, который в проекте принят сознательно: длинные типы каналов и кортежей, функции рисования с многими
// параметрами, заполнение структур по полям после Default — понятнее здесь, чем обёртки ради правила.
#![allow(clippy::type_complexity, clippy::too_many_arguments, clippy::field_reassign_with_default)]
//!
//! Команды: --diag (проверки таблицей в терминал), --sessions, --speed, --fix ИД [ПАРАМЕТР], --widget.
//!
//! Переменные окружения — для разработки и проверки (в обычной работе не нужны; описаны в TESTING.md):
//!   DRY_RUN=1        — исправления только пишут команды в журнал, ничего не выполняют
//!   UI_K=2           — масштаб интерфейса (по умолчанию — масштаб монитора)
//!   UI_SHOT=файл.png — без окна нарисовать всё окно в PNG; UI_H=высота окна, T=секунды (момент анимации),
//!                      UI_FAKE=ok|fix (образцы данных вместо проверок), UI_DO="fix0;speed;menu2;sessions;heal;about;tab1"
//!   WIDGET_SHOT=файл.png — то же для виджета; WIDGET_HOVER=1 — как под мышью; WIDGET_MAC=папка — widget.json
//!                      для виджета macOS; WIDGET_CONCEPT=1|warn|bad — данные как на концептах
//!   WIDGET_ANIM_LOG=1 — виджет пишет в журнал, какие кадры анимации рисует
//!   FRAMES=папка N=… — без окна записать N кадров PNG; CLICK=1.5:84,146 — тычки (секунда:клетка x,y; через «;»)
//!   ART=папка        — брать слои с диска, а не вшитые (картинки должны быть целы: для художника)
//!   SCALE=4, FPS=20, SEED=1 — клетка, кадры в секунду, зерно случайностей (одинаковые прогоны)
//!   TROUBLE=1        — кот беспокоится всё время; APP_ANIM=0 — без анимации окна
//!   SCHED=0          — не трогать планировщик (CORES=8-15 — привязать к ядрам); NODIAG=1 — без проверок; DIAG_WAIT=сек — подождать их при записи кадров
//!
//! Мышь: ухо — смущение, голова — возмущение, хвост — беспокойство, растение — качается.

mod clip;
mod cmd;
mod cover;
// своя сборка владелицы: папка custom/ рядом с engine/ (в git её нет), build.rs включает cfg aptechka_custom
#[cfg(aptechka_custom)]
mod custom {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/../custom/mod.rs"));
}
mod sched;
mod tray;
/// Тесты, которые читают или меняют переменные окружения (HOME): переменные общие на весь процесс, а тесты
/// идут параллельно — без общего замка один тест подменял HOME посреди другого (cursor_layout падал через раз).
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
mod agents;
mod art;
mod catalog;
mod colorman;
mod diag;
mod disks;
mod dupes;
mod elevate;
mod fixes;
mod footprint;
mod gfx;
mod i18n;
mod licenses;
mod life;
mod limits;
mod looksnap;
#[cfg(target_os = "macos")]
mod mac;
mod more;
mod netproxy;
mod notify;
mod prefs;
mod procs;
mod proxyaddr;
mod quips;
mod reach;
mod scene;
mod screens;
mod shellcfg;
mod single;
mod spravka;
mod store;
mod ui;
mod widget;
mod widget_anim;
mod widget_look;
mod widget_px;
mod widget_tpl;
#[cfg(windows)]
mod win;

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
#[cfg(target_os = "linux")]
use winit::platform::x11::WindowAttributesExtX11;
use winit::window::{CursorIcon, ResizeDirection, Window, WindowId};

use gfx::{Canvas, Rect};
use life::Life;
use scene::{compose, Draw, Scene};
use screens::{ScreenState, Screens};
use ui::{Cmd, Ui};

/// Открыть терминал этой системы и выполнить в нём команду; терминал остаётся открытым.
/// Linux — первый найденный из привычных (GNOME, KDE, Xfce, kitty, Alacritty, xterm…), Windows — PowerShell,
/// macOS — Terminal (команда передаётся аргументом AppleScript, а не вклеивается в его текст).
fn open_terminal_with(script: &str) -> Result<&'static str, String> {
    use std::process::Command;
    #[cfg(windows)]
    {
        // своё окно консоли (CREATE_NEW_CONSOLE) — без «cmd /c start»: cmd заново разбирал бы строку,
        // и «&» в имени папки стал бы командой
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("powershell");
        c.args(["-NoExit", "-Command", script]).creation_flags(0x0000_0010);
        return procs::spawn_detached(&mut c).then_some("PowerShell").ok_or_else(|| "PowerShell не запустился".into());
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = Command::new("osascript");
        c.args([
            "-e",
            "on run argv",
            "-e",
            "tell application \"Terminal\" to do script (item 1 of argv)",
            "-e",
            "tell application \"Terminal\" to activate",
            "-e",
            "end run",
            script,
        ]);
        return procs::spawn_detached(&mut c).then_some("Terminal").ok_or_else(|| "Terminal не запустился".into());
    }
    #[allow(unreachable_code)]
    {
        // после команды — обычная оболочка, чтобы окно не закрылось
        let keep = format!("{script}; exec bash");
        // (терминал, как передать команду)
        const TERMS: [(&str, &[&str]); 8] = [
            ("gnome-terminal", &["--"]),
            ("ptyxis", &["--"]),
            ("konsole", &["-e"]),
            ("xfce4-terminal", &["-x"]),
            ("kitty", &[]),
            ("alacritty", &["-e"]),
            ("x-terminal-emulator", &["-e"]),
            ("xterm", &["-e"]),
        ];
        for (t, how) in TERMS {
            if procs::which(t).is_none() {
                continue;
            }
            // окружение — полное (не scrub_env): в терминале человек работает дальше, ему нужны его PATH и ключи
            let mut c = Command::new(t);
            c.args(how).args(["bash", "-lc", &keep]).stdin(std::process::Stdio::null());
            // spawn_detached дожидается терминала в своём потоке — иначе на каждое «открыть» оставался «зомби»
            if procs::spawn_detached(&mut c) {
                return Ok(t);
            }
        }
        Err("не нашёл программу-терминал (gnome-terminal, konsole, xterm…)".into())
    }
}

/// Аргументы командной строки; не в UTF-8 — с заменой непонятных байтов (env::args() на них паникует).
fn cli_args() -> Vec<String> {
    std::env::args_os().map(|a| a.to_string_lossy().into_owned()).collect()
}

fn log(msg: &str) {
    eprintln!("[aptechka] {msg}");
}

fn wlog(msg: &str) {
    eprintln!("[окно] {msg}");
}

fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

/// Сообщения из фоновых потоков в окно.
enum Bg {
    Sessions(Vec<diag::Session>),
    /// Прогон проверок по кнопке (true — писать каждую проверку в журнал).
    Checks(Vec<diag::Check>, bool),
    Speed(diag::SpeedProgress),
    SpeedDone(Vec<diag::Check>),
    FixDone(bool, String),
    Analytics(spravka::Analytics),
    /// «Проверить всё»: можно ли мерить скорость (false — сеть лимитная)
    SpeedAuto(bool),
}

struct App {
    sc: Scene,
    /// потолок с проводами над картинкой (дорисован ChatGPT, своё разрешение) — для окна выше картинки
    ceiling: (usize, usize, Vec<u32>),
    /// он же под текущий масштаб клетки: (масштаб, ширина, высота, пиксели)
    ceiling_px: (usize, usize, usize, Vec<u32>),
    /// Своя копия кадра: там, где система каждый раз отдаёт пустой буфер (macOS, Windows),
    /// рисуем только изменившееся сюда и копируем целиком — а не рисуем всё окно заново.
    back: Vec<u32>,
    /// Окно не видно (закрыто другими окнами, свёрнуто): не рисуем.
    hidden: bool,
    /// окно целиком закрыто другими (Linux — спрашиваем оконный менеджер, Windows — точки окна); проверяется
    /// раз в секунду, только без фокуса (в фокусе окно не закрыто); закрыто — не рисуем, как скрытое
    covered: bool,
    cover_t: Instant,
    #[cfg(all(unix, not(target_os = "macos")))]
    cover: Option<Option<crate::cover::Cover>>,
    /// Окно в фокусе. Без фокуса, но на виду — рисуем втрое реже (живые показания в картинке всё равно идут).
    focused: bool,
    /// файл программы при запуске (время изменения) и сказали ли уже, что её обновили
    exe_stamp: Option<std::time::SystemTime>,
    /// секунда последней проверки «программу обновили» (раз в 5 с)
    update_sec: i64,
    told_update: bool,
    /// сколько починок идёт сейчас (пока идут — не перезапускаемся новой версией)
    fixes_running: u32,
    /// пора перезапуститься новой версией (делает about_to_wait — у него есть цикл событий)
    restart_now: bool,
    /// когда последний раз показывали кадр (macOS и Windows показывают окно целиком — не чаще 10 раз в секунду)
    last_present: Instant,
    /// масштаб клетки до «развернуть» (Some — окно развёрнуто)
    base_scale: Option<usize>,
    life: Life,
    /// реплики кота на тычки
    quips: quips::Quips,
    draws: Vec<Draw>,
    frame: Vec<u32>, // кадр в клетках (w × h)
    owner: Vec<u16>, // кто сверху в каждой клетке
    cursor: (f64, f64),
    clicks: Vec<(f64, usize, usize)>, // CLICK для записи кадров: (время, x, y)
    prev: Vec<u32>,
    scale: usize,
    fps: f64,
    /// анимация картинки (prefs::app_anim): выключена — кот замирает, кадр раз в секунду
    anim: bool,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    t0: Instant,
    last_t: f64,
    next: Instant,
    need_full: bool,
    stat: (u32, Duration, Instant),
    art: String,
    screens: Option<Screens>,
    sstate: ScreenState,
    screens_t: f64,      // когда экраны рисовались последний раз
    screens_dirty: bool, // экраны перерисованы — наложить заново
    diag_rx: Option<Receiver<Vec<diag::Check>>>,
    trouble_env: bool,
    // окно «Аптечки»
    ui: Option<Ui>,
    bg_tx: Sender<Bg>,
    bg_rx: Receiver<Bg>,
    checks: Vec<diag::Check>,       // последний прогон проверок (игнорируемые — уже «для сведения»)
    raw_checks: Vec<diag::Check>,   // он же как есть (чтобы «Не игнорировать» вернуло строке её статус)
    speed_checks: Vec<diag::Check>, // замер скорости (отдельно от прогона)
    sessions: Vec<diag::Session>,
    first_checks: bool,
    checks_running: bool,
    recheck_at: Option<Instant>,
    last_sec: i64,
    avatar_done: bool,
    alt: bool, // зажат Alt (для Alt+буква в меню)
    /// край окна под мышью (для курсора-стрелки и перетаскивания)
    edge: Option<ResizeDirection>,
    /// macOS: окно тянут за край сами — (край, где нажали, размер тогда)
    manual_resize: Option<(ResizeDirection, (f64, f64), (f64, f64))>,
    /// левая кнопка мыши сейчас нажата (отпускание за краем окна может не дойти — тогда сверяемся)
    left_down: bool,
    /// специальные возможности (AccessKit: AT-SPI на Linux, UI Automation на Windows, NSAccessibility на macOS) —
    /// экранный диктор и программы управления (agent-desktop) видят кнопки, строки и меню окна
    a11y: Option<accesskit_winit::Adapter>,
    proxy: Option<EventLoopProxy<AppEvent>>,
    /// подпись последнего отданного дерева (чтобы не слать одно и то же) и когда сверяли
    a11y_sig: u64,
    a11y_t: Instant,
    /// экранный диктор подключён (InitialTreeRequested … AccessibilityDeactivated): без него дерево не строим
    a11y_on: bool,
}

/// События от других потоков: вторая копия попросила показать окно; специальные возможности.
enum AppEvent {
    ShowWindow,
    A11y(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for AppEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        AppEvent::A11y(e)
    }
}

/// Результаты проверок и сессии Claude → что показать на экранах (как _screen_state в Python-версии).
fn screen_state(checks: &[diag::Check], sessions: &[diag::Session], speed: Option<(f64, f64, f64)>, prev: &ScreenState, t: f64) -> ScreenState {
    let g = |title: &str| checks.iter().find(|c| c.title == title);
    // идущий замер скорости — до его конца (Bg::SpeedDone его снимает)
    let mut st = ScreenState { message_t: prev.message_t, speed_live: prev.speed_live, ..ScreenState::default() };
    st.speed = speed;
    if let Some(v) = g("VPN (итог)") {
        let short = if v.status == "ok" { v.value.replace("мир видит ", "вкл · ") } else { v.value.clone() };
        // перевод до обрезки (обрезанную фразу в таблице i18n не найти)
        st.vpn = (v.status.clone(), i18n::t(&short).chars().take(22).collect());
    }
    // Claude: по живым сессиям Claude Code (как в gui.py _screen_state)
    let live: Vec<&diag::Session> = sessions.iter().filter(|s| s.live).collect();
    st.claude = if live.is_empty() {
        ("bad".into(), "выключен".into())
    } else {
        let busy = live.iter().any(|s| s.status == "busy");
        ("ok".into(), format!("вкл · {} · {}", live.len(), if busy { "думает" } else { "ждёт" }))
    };
    let mut net = Vec::new();
    if let Some(r) = g("Мой внешний IP") {
        net.push(("info".into(), "IP".into(), r.value.split_whitespace().next().unwrap_or("").to_string()));
    }
    if let Some(r) = g("Системный прокси") {
        let val = if r.value.contains("выключен") {
            "выкл"
        } else if r.status == "bad" {
            "ПУСТО!"
        } else {
            "вкл"
        };
        net.push((r.status.clone(), "Прокси".into(), val.into()));
    }
    for name in ["Happ", "Outline", "Proton VPN"] {
        if let Some(r) = g(&format!("Канал {name}")) {
            let val = if r.status == "ok" {
                "вкл"
            } else if r.status == "bad" {
                "СБОЙ"
            } else {
                "выкл"
            };
            net.push((r.status.clone(), name.split(' ').next().unwrap_or(name).into(), val.into()));
        }
    }
    if let Some(r) = g("DNS") {
        net.push((r.status.clone(), "DNS".into(), r.value.clone()));
    }
    st.net_rows = net;
    let mut sys = Vec::new();
    if let Some(cpu) = g("Процессор") {
        // «занят на 12% (4 потока)» — нужна часть с процентами, а не второе слово
        let load = cpu.value.split_whitespace().find(|w| w.ends_with('%')).unwrap_or("").to_string();
        let tc = g("Температура CPU");
        let s = if tc.is_some_and(|t| t.status == "bad") || cpu.status == "bad" {
            "bad"
        } else if tc.is_some_and(|t| t.status == "warn") || cpu.status == "warn" {
            "warn"
        } else {
            "ok"
        };
        sys.push((s.into(), "CPU".into(), format!("{load} {}", tc.map(|t| t.value.replace("°C", "°")).unwrap_or_default())));
    }
    if let Some(gpu) = g("Видеокарта") {
        let parts: Vec<&str> = gpu.value.split('·').map(|x| x.trim()).collect();
        let temp = parts.first().unwrap_or(&"").replace("°C", "°");
        let load = parts.iter().find(|x| x.starts_with("загрузка")).and_then(|x| x.split_whitespace().last()).unwrap_or("");
        sys.push((gpu.status.clone(), "GPU".into(), format!("{load} {temp}").trim().to_string()));
    }
    if let Some(m) = g("Память") {
        sys.push((m.status.clone(), "RAM".into(), m.value.split_whitespace().next().unwrap_or("").into()));
    }
    if let Some(d) = g("Диск") {
        sys.push((d.status.clone(), "Диск".into(), format!("{}Г", d.value.split_whitespace().nth(1).unwrap_or(""))));
    }
    st.sys_rows = sys;
    // настроение и сообщение кота
    let worst = diag::worst(checks).to_string();
    let first = |s: &str| checks.iter().find(|c| c.status == s);
    st.message = match worst.as_str() {
        "bad" => first("bad").map(|c| format!("Ой! {}: {}", i18n::t(&c.title), i18n::t(&c.value))).unwrap_or_default(),
        "warn" => first("warn").map(|c| format!("Хм, {}: {}", i18n::t(&c.title), i18n::t(&c.value))).unwrap_or_default(),
        _ if diag::unchecked_count(checks) > 0 => format!("Проблем не вижу, но не всё получилось проверить: {}", diag::unchecked_count(checks)),
        _ => "Всё спокойно, мур! :3".into(),
    };
    st.mood = worst;
    if st.message != prev.message {
        st.message_t = t; // новое сообщение печатается заново
    }
    st
}

/// Масштаб картинки: по масштабу экрана (≈2,67 «логических» на клетку), но чтобы окно влезало по высоте.
fn pick_scale(k: i32, cell_h: usize, mon_h: Option<u32>) -> usize {
    let want = ((2.67 * k as f64).round() as usize).max(1);
    match mon_h {
        Some(mh) => {
            let room = (mh as usize * 85 / 100).saturating_sub(94 * k as usize);
            want.min((room / cell_h.max(1)).max(1))
        }
        None => want,
    }
}

/// Круглая «аватарка» кота для облачка: вырезка лица из кадра картинки (в клетках).
fn make_avatar(frame: &[u32], w: usize, h: usize) -> Option<tiny_skia::Pixmap> {
    let (cx, cy, half) = (env::<i32>("FACE_X").unwrap_or(165), env::<i32>("FACE_Y").unwrap_or(141), 35i32); // как в Python-версии: кусок B1 (700,560)–(1070,930)
    let f = 3i32; // клетка → 3×3 пикселя, дальше окно уменьшает усреднением
    let size = (half * 2 * f) as u32;
    let mut pm = tiny_skia::Pixmap::new(size, size)?;
    let r = size as f32 / 2.0;
    for (i, p) in pm.data_mut().chunks_mut(4).enumerate() {
        let (x, y) = ((i as u32 % size) as i32, (i as u32 / size) as i32);
        let (dx, dy) = (x as f32 + 0.5 - r, y as f32 + 0.5 - r);
        let d = (dx * dx + dy * dy).sqrt();
        let a = (r - 1.0 - d).clamp(0.0, 1.0);
        let (sx, sy) = (cx - half + x / f, cy - half + y / f);
        let c = if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
            frame.get(sy as usize * w + sx as usize).copied().unwrap_or(0)
        } else {
            0
        };
        let al = (a * 255.0) as u32;
        p[0] = ((c >> 16 & 255) * al / 255) as u8;
        p[1] = ((c >> 8 & 255) * al / 255) as u8;
        p[2] = ((c & 255) * al / 255) as u8;
        p[3] = al as u8;
    }
    Some(pm)
}

/// Увеличить строки клеток (redo) в буфер окна: картинка стоит в точке (ox, oy), строка буфера — stride.
fn upscale(frame: &[u32], w: usize, s: usize, buf: &mut [u32], stride: usize, ox: usize, oy: usize, redo: &[bool]) {
    for (y, &r) in redo.iter().enumerate() {
        if !r {
            continue;
        }
        let Some(row) = frame.get(y * w..(y + 1) * w) else { continue };
        for sy in 0..s {
            let o = (oy + y * s + sy) * stride + ox;
            let Some(dst) = buf.get_mut(o..o + w * s) else { continue };
            for (x, &p) in row.iter().enumerate() {
                dst[x * s..x * s + s].fill(p);
            }
        }
    }
}

impl App {
    /// Дерево специальных возможностей: окно и в нём — всё, на что можно нажать (кнопки, меню, вкладки,
    /// строки проверок с их состоянием), плюс текст облачка кота. Координаты — в точках окна.
    fn a11y_tree(&mut self) -> accesskit::TreeUpdate {
        use accesskit::{Action, Node, NodeId, Rect as ARect, Role, TreeId, TreeInfo, TreeUpdate};
        let (nodes, w, h) = match self.ui.as_mut() {
            Some(u) => (u.a11y_nodes(), u.w, u.h),
            None => (Vec::new(), 0, 0),
        };
        let mut root = Node::new(Role::Window);
        root.set_label(i18n::t("Аптечка"));
        root.set_bounds(ARect::new(0.0, 0.0, w as f64, h as f64));
        let mut out = Vec::with_capacity(nodes.len() + 1);
        let mut seen = std::collections::HashSet::new();
        for n in nodes {
            // один и тот же элемент мог попасть дважды (нарисован в двух местах) — оставляем первый
            if !seen.insert(n.id) {
                continue;
            }
            let role = match n.role {
                ui::A11yRole::Button => Role::Button,
                ui::A11yRole::MenuItem => Role::MenuItem,
                ui::A11yRole::Tab => Role::Tab,
                ui::A11yRole::Row => Role::ListItem,
                ui::A11yRole::Label => Role::Label,
            };
            let mut node = Node::new(role);
            // у надписи текст читается как её значение (AT-SPI показывал пустое имя) — ставим и то и другое
            if role == Role::Label {
                node.set_value(i18n::t(&n.name));
            }
            node.set_label(i18n::t(&n.name));
            if !n.value.is_empty() {
                node.set_value(i18n::t(&n.value));
            }
            let r = n.rect;
            node.set_bounds(ARect::new(r.x as f64, r.y as f64, (r.x + r.w) as f64, (r.y + r.h) as f64));
            if role == Role::Tab {
                node.set_selected(n.selected);
            }
            if role != Role::Label {
                node.add_action(Action::Click);
            }
            root.push_child(NodeId(n.id));
            out.push((NodeId(n.id), node));
        }
        out.push((NodeId(0), root));
        TreeUpdate { nodes: out, tree: Some(TreeInfo::new(NodeId(0))), tree_id: TreeId::ROOT, focus: NodeId(0) }
    }

    /// Обновить дерево специальных возможностей, если интерфейс поменялся (не чаще 4 раз в секунду;
    /// force — сразу, после нажатия). Если ими никто не пользуется, адаптер ничего не делает.
    fn a11y_refresh(&mut self, force: bool) {
        if self.a11y.is_none() || !self.a11y_on || (!force && self.a11y_t.elapsed() < Duration::from_millis(250)) {
            return;
        }
        self.a11y_t = Instant::now();
        let tree = self.a11y_tree();
        let mut x: u64 = 0xcbf29ce484222325;
        for (id, n) in &tree.nodes {
            for b in id.0.to_le_bytes().iter().chain(n.label().unwrap_or("").as_bytes()) {
                x ^= *b as u64;
                x = x.wrapping_mul(0x100000001b3);
            }
            if let Some(b) = n.bounds() {
                x ^= (b.x0 as u64) << 1 ^ (b.y0 as u64) << 17 ^ (b.x1 as u64) << 33 ^ (b.y1 as u64) << 49;
            }
        }
        if x != self.a11y_sig {
            self.a11y_sig = x;
            if let Some(a) = self.a11y.as_mut() {
                a.update_if_active(|| tree);
            }
        }
    }

    /// Пересобрать то, что показывают экраны в картинке (по последним проверкам, сессиям и скорости).
    fn refresh_sstate(&mut self, t: f64) {
        let all: Vec<diag::Check> = self.checks.iter().chain(self.speed_checks.iter()).cloned().collect();
        let speed = self.ui.as_ref().and_then(|u| u.speed_values());
        self.sstate = screen_state(&all, &self.sessions, speed, &self.sstate, t);
        self.life.trouble = self.trouble_env || self.sstate.mood == "bad";
    }

    fn apply_checks(&mut self, checks: Vec<diag::Check>, full_log: bool, t: f64) {
        self.raw_checks = checks.clone();
        let checks = diag::apply_ignored(&checks);
        self.checks = checks.clone();
        // замер скорости — общий с виджетом (speed.json, не старше 6 часов): после перезапуска окно его
        // помнит, а устаревший уходит из итога в обоих местах одинаково
        let speed = crate::sched::load_speed();
        let running = self.ui.as_ref().is_some_and(|u| u.is_speed_running());
        let differs = speed.len() != self.speed_checks.len() || speed.iter().zip(&self.speed_checks).any(|(a, b)| a.value != b.value);
        if !running && differs && std::env::var_os("UI_SHOT").is_none() {
            self.speed_checks = speed.clone();
            if let Some(u) = self.ui.as_mut() {
                u.set_speed_rows(speed);
            }
        }
        // macOS: состояние для виджета WidgetKit (macos/widget)
        #[cfg(target_os = "macos")]
        widget::export_mac(&checks);
        if let Some(u) = self.ui.as_mut() {
            u.set_checks(checks, full_log || self.first_checks);
        }
        self.first_checks = false;
        self.checks_running = false;
        self.refresh_sstate(t);
        log(&format!("проверки: итог {}{}", self.sstate.mood, if self.life.trouble { " — кот беспокоится" } else { "" }));
    }

    fn apply_sessions(&mut self, ss: Vec<diag::Session>, t: f64) {
        let changed =
            ss.len() != self.sessions.len() || ss.iter().zip(&self.sessions).any(|(a, b)| a.id != b.id || a.status != b.status || a.live != b.live);
        if changed {
            self.sessions = ss.clone();
            self.refresh_sstate(t);
        }
        if let Some(u) = self.ui.as_mut() {
            u.set_sessions(ss);
        }
    }

    /// Всё, что пришло из фоновых потоков.
    fn drain_bg(&mut self, t: f64) {
        if let Some(note) = crate::sched::take_eco_note() {
            if let Some(u) = self.ui.as_mut() {
                u.log(note);
            }
        }
        while let Ok(m) = self.bg_rx.try_recv() {
            match m {
                Bg::Sessions(ss) => self.apply_sessions(ss, t),
                Bg::Checks(c, full) => self.apply_checks(c, full, t),
                Bg::Speed(p) => {
                    // стрелка спидометра на экране «Сеть» в картинке
                    let (ph, val) = match p.phase {
                        diag::SpeedPhase::Ping => (0, p.ping_ms.unwrap_or(0.0)),
                        diag::SpeedPhase::Down => (1, p.mbps),
                        diag::SpeedPhase::Up => (2, p.mbps),
                    };
                    self.sstate.speed_live = Some((ph, val));
                    if let Some(u) = self.ui.as_mut() {
                        u.speed_progress(p);
                    }
                }
                Bg::SpeedAuto(ok) => {
                    if ok {
                        self.start_speed();
                    } else if let Some(u) = self.ui.as_mut() {
                        u.log("Сеть лимитная — скорость не мерю (замер качает около 60 МБ). Измерить: кнопка «Скорость».");
                    }
                }
                Bg::Analytics(a) => {
                    if let Some(u) = self.ui.as_mut() {
                        u.analytics_ready(a);
                    }
                }
                Bg::SpeedDone(res) => {
                    self.sstate.speed_live = None;
                    crate::sched::save_speed(&res);
                    self.speed_checks = res.clone();
                    if let Some(u) = self.ui.as_mut() {
                        u.speed_done(res);
                    }
                    self.refresh_sstate(t);
                }
                Bg::FixDone(ok, msg) => {
                    self.fixes_running = self.fixes_running.saturating_sub(1);
                    if let Some(u) = self.ui.as_mut() {
                        u.fix_done(ok, &msg);
                    }
                    self.recheck_at = Some(Instant::now() + Duration::from_secs(3));
                }
            }
        }
        if self.recheck_at.is_some_and(|r| Instant::now() >= r) {
            self.recheck_at = None;
            self.start_checks(false);
        }
    }

    /// Прогон проверок. speed — и замер скорости («Проверить всё» по кнопке; перепроверка после починки — без него).
    fn start_checks(&mut self, speed: bool) {
        if self.checks_running {
            return;
        }
        self.checks_running = true;
        if let Some(u) = self.ui.as_mut() {
            u.log("Проверка начата…");
            u.set_checking(true);
        }
        // «Проверить всё» мерит и скорость (качает ~60 МБ) — кроме лимитной сети (мобильный интернет)
        if speed {
            let tx = self.bg_tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Bg::SpeedAuto(!diag::is_metered()));
            });
        }
        if self.diag_rx.is_some() {
            // «Проверить всё» — расписанию (у себя или у процесса, который проверяет); итог придёт как обычно
            sched::check_now();
        } else {
            let tx = self.bg_tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Bg::Checks(diag::run_all(), true));
            });
        }
    }

    fn start_speed(&mut self) {
        let Some(u) = self.ui.as_mut() else { return };
        if u.is_speed_running() {
            return;
        }
        u.start_speed();
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let mut last = (Instant::now() - Duration::from_secs(1), None::<diag::SpeedPhase>);
            let t2 = tx.clone();
            let res = diag::speed_test(move |p| {
                // не чаще 8 раз в секунду, но смену этапа показываем сразу
                if last.1 != Some(p.phase) || last.0.elapsed() >= Duration::from_millis(120) {
                    last = (Instant::now(), Some(p.phase));
                    let _ = t2.send(Bg::Speed(p));
                }
            });
            let _ = tx.send(Bg::SpeedDone(res));
        });
    }

    fn start_fix(&mut self, id: String, label: String, arg: Option<String>) {
        self.fixes_running += 1;
        if let Some(u) = self.ui.as_mut() {
            u.fix_started(&label);
        }
        let tx = self.bg_tx.clone();
        std::thread::spawn(move || {
            let (ok, msg) = fixes::apply(&id, arg.as_deref());
            let _ = tx.send(Bg::FixDone(ok, msg));
        });
    }

    /// Открыть терминал с командой (кнопка ▶ у сессии и «Текстовый режим»). В DRY_RUN только пишет в журнал.
    fn open_terminal(&mut self, script: String) {
        if fixes::dry_run() {
            log(&format!("DRY_RUN: открыл бы терминал: {script}"));
            return;
        }
        match open_terminal_with(&script) {
            Ok(t) => log(&format!("открыл терминал ({t})")),
            Err(e) => {
                log(&format!("терминал не открылся: {e}"));
                if let Some(u) = self.ui.as_mut() {
                    u.say(&format!("Не открылся терминал: {e}"));
                }
            }
        }
    }

    fn handle_cmds(&mut self, cmds: Vec<Cmd>, el: Option<&ActiveEventLoop>) {
        for c in cmds {
            match c {
                Cmd::Quit => {
                    if let Some(el) = el {
                        el.exit();
                    }
                }
                Cmd::Minimize => {
                    if let Some(w) = &self.window {
                        w.set_minimized(true);
                    }
                }
                Cmd::Maximize => self.toggle_maximize(),
                Cmd::DragWindow => {
                    if let Some(w) = &self.window {
                        let _ = w.drag_window();
                    }
                }
                Cmd::RunChecks => self.start_checks(true),
                Cmd::Recheck => self.start_checks(false),
                Cmd::AnimChanged => {
                    self.anim = crate::prefs::app_anim();
                    wlog(if self.anim {
                        "анимация картинки включена"
                    } else {
                        "анимация картинки выключена"
                    });
                }
                Cmd::IgnoreChanged => {
                    let t = self.t0.elapsed().as_secs_f64();
                    self.apply_checks(self.raw_checks.clone(), false, t);
                }
                Cmd::RunSpeed => self.start_speed(),
                Cmd::RunFix { id, label, arg } => self.start_fix(id, label, arg),
                Cmd::CopyText(t) => clip::copy(t),
                Cmd::OpenResume(cmd) => self.open_terminal(cmd),
                Cmd::TextMode => {
                    let exe = procs::self_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or("aptechka".into());
                    let run = if cfg!(windows) {
                        format!("& {} --diag", crate::agents::ps_quote(&exe))
                    } else {
                        format!("{} --diag", diag::shell_quote(&exe))
                    };
                    self.open_terminal(run);
                }
                Cmd::SaveSpravka => self.save_spravka(),
                Cmd::TrayChanged => {
                    std::thread::spawn(crate::tray::ensure);
                }
                Cmd::LoadAnalytics => {
                    let tx = self.bg_tx.clone();
                    std::thread::spawn(move || {
                        crate::footprint::background_priority();
                        let _ = tx.send(Bg::Analytics(spravka::Analytics::load()));
                    });
                }
                Cmd::ShowWidget => {
                    // уже открыт — второй не нужен
                    let open = procs::list().iter().any(|p| p.is_our("--widget"));
                    if open {
                        // уже открыт, но под окнами — попросить его показаться поверх на несколько секунд
                        widget::request_show();
                        wlog("виджет уже открыт — показываю его поверх окон");
                    } else if let Some(exe) = procs::self_exe() {
                        crate::procs::spawn_detached(std::process::Command::new(exe).arg("--widget"));
                        wlog("открыл виджет на рабочем столе");
                    }
                }
                Cmd::LangChanged => {
                    if let Some(w) = &self.window {
                        w.set_title(&i18n::t("Аптечка"));
                    }
                    wlog(&format!("язык: {}", i18n::lang().id()));
                    #[cfg(target_os = "macos")]
                    widget::export_mac(&self.checks);
                }
                Cmd::WidgetChanged => {
                    let (s, z) = widget_look::chosen();
                    wlog(&format!("виджет: вид «{}», {}", s.name(), if z == widget_look::Size::Small { "маленький" } else { "средний" }));
                    #[cfg(target_os = "macos")]
                    widget::export_mac(&self.checks);
                }
                Cmd::ArtClick(x, y) => {
                    let s = self.scale.max(1) as i32;
                    if x >= 0 && y >= 0 {
                        let t = self.t0.elapsed().as_secs_f64();
                        self.click((x / s) as usize, (y / s) as usize, t);
                    }
                }
            }
        }
    }

    /// «Сохранить как картинку…» в справке. В DRY_RUN только пишет, куда сохранила бы.
    /// Значок в панели просил открыть «Медкарту расхода» (файл analytics.req в папке сеанса).
    fn analytics_request(&mut self) {
        let p = crate::store::runtime_dir().join("analytics.req");
        if std::fs::symlink_metadata(&p).is_err() {
            return;
        }
        crate::store::remove_file_nofollow(&p);
        let cmds = self.ui.as_mut().map(|u| u.open_analytics()).unwrap_or_default();
        self.handle_cmds(cmds, None);
    }

    fn save_spravka(&mut self) {
        let Some(u) = self.ui.as_mut() else { return };
        let Some((path, w, h, rgb)) = u.spravka_png() else { return };
        if fixes::dry_run() {
            u.log(&format!("DRY_RUN: сохранил бы справку в {path}"));
            return;
        }
        // такой файл уже есть — «Справка_…(2).png», а не поверх
        let path = (1..100)
            .map(|i| {
                if i == 1 {
                    path.clone()
                } else {
                    match path.strip_suffix(".png") {
                        Some(stem) => format!("{stem} ({i}).png"),
                        None => format!("{path} ({i})"),
                    }
                }
            })
            .find(|p| std::fs::symlink_metadata(p).is_err())
            .unwrap_or(path);
        match save_rgb(&path, w, h, &rgb) {
            Ok(()) => u.log(&format!("Справка сохранена: {path}")),
            Err(e) => u.log(&format!("Не удалось сохранить справку: {e}")),
        }
    }

    fn render(&mut self, t: f64) {
        let dt = (t - self.last_t).max(0.0);
        self.last_t = t;
        // анимация выключена — картинка замерла в покое (t = 0), шаг жизни нулевой
        let (lt, ldt) = if self.anim { (t, dt) } else { (0.0, 0.0) };
        self.life.step(lt, ldt, &mut self.draws);
        // тычки по расписанию (режим записи кадров)
        while let Some(&(ct, x, y)) = self.clicks.first() {
            if ct > t {
                break;
            }
            self.clicks.remove(0);
            self.click(x, y, t);
        }
        compose(&self.sc, &self.draws, &self.life.seam, &mut self.frame, &mut self.owner);
        // аватарка в облачке: лицо из первого собранного кадра
        if !self.avatar_done && t > 0.0 {
            if let (Some(u), Some(pm)) = (self.ui.as_mut(), make_avatar(&self.frame, self.sc.w, self.sc.h)) {
                u.set_avatar(pm);
                self.avatar_done = true;
            }
        }
        // результаты проверок (приходят из фонового потока раз в минуту)
        let mut latest = None;
        if let Some(rx) = &self.diag_rx {
            while let Ok(c) = rx.try_recv() {
                latest = Some(c);
            }
        }
        if let Some(checks) = latest {
            // ответ на «Проверить всё» — с подробным журналом, как раньше
            let manual = self.checks_running;
            self.apply_checks(checks, manual, t);
        }
        self.drain_bg(t);
        // экраны — 12 раз в секунду
        if self.screens.is_none() && self.scale > 0 {
            self.screens = Screens::load(&self.art, self.sc.w, self.sc.h, self.scale);
        }
        if let Some(scr) = self.screens.as_mut() {
            if t - self.screens_t >= 1.0 / 12.0 {
                self.screens_t = t;
                if scr.draw(t, &self.sstate) {
                    self.screens_dirty = true;
                }
            }
        }
        // часы в строке состояния
        let sec = ui::now_secs();
        if sec != self.last_sec {
            self.last_sec = sec;
            if let Some(u) = self.ui.as_mut() {
                u.tick_second();
            }
        }
    }

    fn click(&mut self, x: usize, y: usize, t: f64) {
        if x >= self.sc.w || y >= self.sc.h {
            return;
        }
        let o = self.owner[y * self.sc.w + x];
        if o == u16::MAX {
            return;
        }
        let name = self.sc.layers[o as usize].name.clone();
        let what = self.life.poke(o as usize, x, y, t).map(str::to_string);
        match what {
            Some(what) => {
                // ножки (по месту) — фразы про ножки, даже если сверху поза в слое «волосы»
                let layer = if self.life.is_legs(x, y) && what.contains("смущ") { "тело".to_string() } else { name.clone() };
                let q = self.quips.pick(&layer, &what);
                log(&format!("тычок в {name} ({x},{y}): {what}{}", q.map(|q| format!(" — «{q}»")).unwrap_or_default()));
                if let (Some(q), Some(u)) = (q, self.ui.as_mut()) {
                    u.quip(q);
                }
            }
            None => log(&format!("тычок в {name} ({x},{y}): пока без реакции")),
        }
    }

    /// Свёрнуто (Windows не присылает Occluded). На X11 это запрос к серверу — там не спрашиваем.
    fn minimized(&self) -> bool {
        cfg!(windows) && self.window.as_ref().and_then(|w| w.is_minimized()) == Some(true)
    }

    /// Закрыто ли окно целиком другими окнами (раз в секунду; в фокусе — нет). Открылось — сразу полный кадр.
    fn check_covered(&mut self) {
        if self.focused {
            if self.covered {
                self.need_full = true;
            }
            self.covered = false;
            return;
        }
        if self.cover_t.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.cover_t = Instant::now();
        let Some(w) = self.window.as_ref() else { return };
        #[cfg(all(unix, not(target_os = "macos")))]
        let now = {
            let (Ok(p), s) = (w.outer_position(), w.outer_size()) else { return };
            let cover = self.cover.get_or_insert_with(crate::cover::Cover::new);
            cover.as_ref().and_then(|c| c.covered((p.x, p.y, s.width as i32, s.height as i32))).unwrap_or(false)
        };
        #[cfg(windows)]
        let now = widget::covered_windows(w).unwrap_or(false);
        #[cfg(target_os = "macos")]
        let now = {
            let _ = w;
            false
        };
        if self.covered && !now {
            self.need_full = true;
        }
        self.covered = now;
    }

    /// Программу обновили, пока окно открыто: как только ничего не происходит (не идёт починка и замер,
    /// не открыт диалог или меню) — перезапуск новой версией, само. Проверяется и когда окно свёрнуто или
    /// закрыто другими (тогда оно не рисуется), иначе старая версия жила бы, пока на неё не посмотрят.
    fn check_update(&mut self) {
        let sec = ui::now_secs();
        if sec == self.update_sec || sec % 5 != 0 {
            return;
        }
        self.update_sec = sec;
        if self.exe_stamp.is_none() || procs::self_stamp() == self.exe_stamp {
            return;
        }
        let idle = self.fixes_running == 0 && self.ui.as_ref().is_none_or(|u| u.is_idle());
        if idle && !fixes::dry_run() {
            log("файл программы обновлён — перезапускаюсь новой версией");
            self.restart_now = true;
        } else if !self.told_update {
            self.told_update = true;
            if let Some(u) = self.ui.as_mut() {
                u.say("Аптечку обновили! Как только закончу дела — перезапущусь сам.");
            }
        }
    }

    fn tick(&mut self) {
        self.check_update();
        self.a11y_refresh(false);
        self.check_covered();
        let quiet = self.hidden || self.minimized() || self.covered;
        // окно снова на виду — сессии обновить сразу, а не через 30 с
        if !quiet && SESS_QUIET.swap(false, std::sync::atomic::Ordering::Relaxed) {
            SESS_POKE.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if quiet {
            SESS_QUIET.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if quiet {
            // окна не видно — не рисуем, только принимаем результаты проверок и сессий
            let t = self.t0.elapsed().as_secs_f64();
            self.drain_bg(t);
            return;
        }
        let t0 = Instant::now();
        let t = self.t0.elapsed().as_secs_f64();
        self.render(t);
        if std::env::var_os("TIMING").is_some() {
            wlog(&format!("сцена {:.1} мс", t0.elapsed().as_secs_f64() * 1000.0));
        }
        if let Some(u) = self.ui.as_mut() {
            u.tick_tip();
        }
        let ui_dirty = self.ui.as_ref().is_some_and(|u| u.has_dirty());
        let changed = self.need_full || self.frame != self.prev || self.screens_dirty || ui_dirty;
        // macOS и Windows отдают системе всё окно каждый раз: если меняется
        // только картинка (дыхание, хвост), показываем не чаще 10 раз в секунду; щелчки и меню — сразу
        // на Маке каждый показ пишет 14 МБ в память, которую система размечает заново, — там реже (~6 раз в с)
        let gap = if cfg!(target_os = "macos") { 160 } else { 100 };
        let throttled = !cfg!(target_os = "linux") && !ui_dirty && !self.need_full && self.last_present.elapsed() < Duration::from_millis(gap);
        if changed && !throttled {
            self.last_present = Instant::now();
            self.present();
            self.prev.copy_from_slice(&self.frame);
        }
        self.stat.0 += 1;
        self.stat.1 += t0.elapsed();
        if self.stat.2.elapsed() >= Duration::from_secs(10) {
            let (redrawn, checked) = self.screens.as_ref().map_or((0, 0), |s| (s.redrawn, s.checked));
            log(&format!(
                "за 10 с: тиков {}, работа {:.3} мс на тик, экраны: перерисовано {redrawn} из {checked}",
                self.stat.0,
                self.stat.1.as_secs_f64() * 1000.0 / self.stat.0 as f64
            ));
            if let Some(s) = self.screens.as_mut() {
                s.redrawn = 0;
                s.checked = 0;
            }
            self.stat = (0, Duration::ZERO, Instant::now());
        }
    }

    /// Нарисовать в буфер окна то, что изменилось: сначала «грязные» куски интерфейса,
    /// потом строки картинки (они меняются по-своему), потом всплывающее поверх картинки.
    /// full — всё заново. Возвращает прямоугольники, которые надо показать.
    /// Потолок и шов под масштаб клетки s: ширина картинки × (CEIL_CELLS + SEAM_CELLS) клеток (плавно, усреднением).
    fn ensure_ceiling(&mut self) {
        let s = self.scale.max(1);
        if self.ceiling_px.0 == s {
            return;
        }
        let (tw, th) = (self.sc.w * s, (CEIL_CELLS + SEAM_CELLS) * s);
        let (sw, sh, src) = (self.ceiling.0, self.ceiling.1, &self.ceiling.2);
        let mut out = vec![0u32; tw * th];
        for ty in 0..th {
            let (y0, y1) = (ty * sh / th, ((ty + 1) * sh).div_ceil(th).min(sh).max(ty * sh / th + 1));
            for tx in 0..tw {
                let (x0, x1) = (tx * sw / tw, ((tx + 1) * sw).div_ceil(tw).min(sw).max(tx * sw / tw + 1));
                let (mut r, mut g, mut b, mut n, mut all) = (0u32, 0u32, 0u32, 0u32, 0u32);
                for y in y0..y1 {
                    for &p in &src[y * sw + x0..y * sw + x1] {
                        all += 1;
                        if p >> 24 == 0 {
                            continue; // прозрачное (в шве) — там видна сама картинка
                        }
                        r += (p >> 16) & 255;
                        g += (p >> 8) & 255;
                        b += p & 255;
                        n += 1;
                    }
                }
                // старший байт: 255 — рисуем, 0 — пропускаем (непрозрачна хотя бы половина)
                out[ty * tw + tx] = if n * 2 < all { 0 } else { 255 << 24 | (r / n) << 16 | (g / n) << 8 | (b / n) };
            }
        }
        self.ceiling_px = (s, tw, th, out);
    }

    fn paint_frame(&mut self, buf: &mut [u32], full: bool) -> Vec<Rect> {
        self.ensure_ceiling();
        let Some(ui) = self.ui.as_mut() else { return Vec::new() };
        let (ww, wh, art, s, w) = (ui.w, ui.h, ui.art, self.scale.max(1), self.sc.w);
        let mut dmg = Vec::new();
        let taken = ui.take_dirty();
        let dirty = if full { vec![Rect::new(0, 0, ww, wh)] } else { taken };
        {
            let mut c = Canvas::new(buf, ww as usize, ww, wh);
            for r in &dirty {
                c.set_clip(*r);
                ui.paint(&mut c);
                dmg.push(*r);
            }
        }
        // строки клеток картинки, которые надо перерисовать: изменившиеся, задетые кусками
        // интерфейса поверх картинки; строки экранов — все сразу, если экраны обновились
        let mut redo: Vec<bool> = self.frame.chunks(w).zip(self.prev.chunks(w)).map(|(a, b)| full || a != b).collect();
        for r in &dirty {
            let i = r.intersect(art);
            if !i.is_empty() {
                for y in ((i.y - art.y) / s as i32)..=((i.bottom() - 1 - art.y) / s as i32) {
                    if let Some(v) = redo.get_mut(y as usize) {
                        *v = true;
                    }
                }
            }
        }
        let mut overlay = false;
        if let Some(scr) = &self.screens {
            let touched = self.screens_dirty || redo.iter().zip(&scr.rows).any(|(a, b)| *a && *b);
            if touched {
                for (r, &sr) in redo.iter_mut().zip(&scr.rows) {
                    *r |= sr;
                }
            }
            overlay = touched;
        }
        upscale(&self.frame, w, s, buf, ww as usize, art.x as usize, art.y as usize, &redo);
        if overlay {
            if let Some(scr) = &self.screens {
                scr.overlay(buf, ww as usize, art.x as usize, art.y as usize);
            }
        }
        // шов: верх картинки так, как его дорисовал ChatGPT вместе с потолком (петля замкнута)
        {
            let (_, cw, _, cpx) = &self.ceiling_px;
            let width = (*cw).min(art.w.max(0) as usize);
            for (cy, _) in redo.iter().enumerate().take(SEAM_CELLS).filter(|(_, r)| **r) {
                for py in 0..s {
                    let (src, y) = ((CEIL_CELLS + cy) * s + py, art.y as usize + cy * s + py);
                    let dst = y * ww as usize + art.x.max(0) as usize;
                    if (y as i32) >= wh || dst + width > buf.len() || (src + 1) * cw > cpx.len() {
                        continue;
                    }
                    for (o, &p) in buf[dst..dst + width].iter_mut().zip(&cpx[src * cw..src * cw + width]) {
                        if p >> 24 != 0 {
                            *o = p & 0xFF_FFFF;
                        }
                    }
                }
            }
        }
        // над картинкой пусто (окно выше неё) — продолжаем провода: верхний ряд картинки тянем вверх
        let void = ui.art_void();
        if !void.is_empty() && (full || redo.first() == Some(&true) || dirty.iter().any(|r| !r.intersect(void).is_empty())) {
            // сначала дорисованный потолок (art/layers/потолок.png, ровно над картинкой),
            // выше — его верхний ряд тянется вверх и гаснет: провода уходят в темноту
            let (_, cw, chh, cpx) = &self.ceiling_px;
            let (cw, chh) = (*cw, (*chh).min(CEIL_CELLS * s)); // ниже в файле — шов, он рисуется по картинке
            let width = cw.min(art.w.max(0) as usize);
            let (top, above) = (void.y.max(0) as usize, (art.y - void.y).max(1) as usize);
            let extra = above.saturating_sub(chh).max(1) as u32; // сколько тянем выше потолка
            for y in top..art.y as usize {
                let d = art.y as usize - 1 - y; // пикселей над картинкой
                let (row, f) = if d < chh { (chh - 1 - d, 256u32) } else { (0, 256 - ((d - chh) as u32 * 150 / extra).min(150)) };
                let dst = y * ww as usize + art.x.max(0) as usize;
                if dst + width > buf.len() {
                    continue;
                }
                let line = &cpx[row * cw..row * cw + width];
                if f == 256 {
                    for (o, &p) in buf[dst..dst + width].iter_mut().zip(line) {
                        *o = p & 0xFF_FFFF;
                    }
                } else {
                    for (o, &p) in buf[dst..dst + width].iter_mut().zip(line) {
                        let ch = |sh: u32| ((((p >> sh) & 255) * f) >> 8) << sh;
                        *o = ch(16) | ch(8) | ch(0);
                    }
                }
            }
            dmg.push(void);
        }
        // строки картинки → прямоугольники (соседние строки сливаем)
        let mut y = 0;
        let (mut y0, mut y1) = (usize::MAX, 0);
        while y < redo.len() {
            if redo[y] {
                if y0 == usize::MAX {
                    y0 = y;
                }
                y1 = y + 1;
            } else if y0 != usize::MAX {
                dmg.push(Rect::new(art.x, art.y + (y0 * s) as i32, art.w, ((y1 - y0) * s) as i32));
                y0 = usize::MAX;
            }
            y += 1;
        }
        if y0 != usize::MAX {
            dmg.push(Rect::new(art.x, art.y + (y0 * s) as i32, art.w, ((y1 - y0) * s) as i32));
        }
        // меню и окно вопроса лежат поверх картинки: после её обновления рисуем их снова
        let ov = ui.overlay_rect();
        if !ov.is_empty() {
            let band = dmg.iter().skip(dirty.len()).fold(Rect::default(), |a, b| a.union(*b));
            let part = ov.intersect(band);
            if !part.is_empty() {
                let mut c = Canvas::new(buf, ww as usize, ww, wh);
                c.set_clip(part);
                ui.paint_overlay(&mut c);
            }
        }
        self.screens_dirty = false;
        dmg
    }

    /// Показать изменившееся в окне.
    fn present(&mut self) {
        let Some(mut surface) = self.surface.take() else { return };
        let q0 = Instant::now();
        if let Ok(mut buf) = surface.buffer_mut() {
            let t_buf = q0.elapsed();
            // X11 отдаёт буфер с прошлым кадром — дорисовываем прямо в нём (после смены размера — всё заново);
            // macOS и Windows отдают пустой — рисуем в свою копию.
            // Монитор с цветовым профилем (colorman): кадр пересчитывается под монитор — тогда и на X11 рисуем
            // в свою копию (иначе дорисовка ложилась бы на уже пересчитанные точки и пересчитывалась дважды),
            // а в буфер X11 переносим и пересчитываем только изменившееся
            let cm = colorman::get();
            let linux = cfg!(target_os = "linux");
            let direct = linux && cm.is_none();
            if !direct && self.back.len() != buf.len() {
                self.back = vec![0; buf.len()];
                self.need_full = true;
            }
            let full = self.need_full || (linux && buf.age() != 1);
            let q = Instant::now();
            let mut t_copy = std::time::Duration::ZERO;
            let dmg = if direct {
                self.paint_frame(&mut buf, full)
            } else {
                let mut back = std::mem::take(&mut self.back);
                let d = self.paint_frame(&mut back, full);
                let qc = Instant::now();
                let bw = self.ui.as_ref().map(|u| u.w.max(1) as usize).unwrap_or(1);
                if linux && !full {
                    // только изменившиеся куски: остальное в буфере X11 — прошлый кадр, уже пересчитанный
                    let bh = buf.len() / bw;
                    for r in &d {
                        let r = r.intersect(Rect::new(0, 0, bw as i32, bh as i32));
                        if r.w <= 0 || r.h <= 0 {
                            continue;
                        }
                        let (x, w) = (r.x as usize, r.w as usize);
                        for y in r.y as usize..(r.y + r.h) as usize {
                            buf[y * bw + x..y * bw + x + w].copy_from_slice(&back[y * bw + x..y * bw + x + w]);
                        }
                        if let Some(cm) = cm {
                            cm.apply_rect(&mut buf, bw, x, r.y as usize, w, r.h as usize);
                        }
                    }
                } else {
                    buf.copy_from_slice(&back);
                    if let Some(cm) = cm {
                        cm.apply_rect(&mut buf, bw, 0, 0, bw, back.len() / bw);
                    }
                }
                t_copy = qc.elapsed();
                self.back = back;
                d
            };
            let t_paint = q.elapsed() - t_copy;
            // X11 молча отбрасывает кусок, вылезающий за окно хоть на пиксель, — обрезаем по окну
            let (bw, bh) = self.ui.as_ref().map(|u| (u.w, u.h)).unwrap_or((0, 0));
            let rects: Vec<softbuffer::Rect> = dmg
                .iter()
                .map(|r| r.intersect(Rect::new(0, 0, bw, bh)))
                .filter_map(|r| {
                    Some(softbuffer::Rect {
                        x: r.x.max(0) as u32,
                        y: r.y.max(0) as u32,
                        width: NonZeroU32::new(r.w.max(0) as u32)?,
                        height: NonZeroU32::new(r.h.max(0) as u32)?,
                    })
                })
                .collect();
            let qp = Instant::now();
            let _ = if full || !linux { buf.present() } else { buf.present_with_damage(&rects) };
            if std::env::var_os("TIMING").is_some() {
                let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
                wlog(&format!(
                    "кадр: буфер {:.1} · отрисовка {:.1} · копия {:.1} · показ {:.1} мс{}",
                    ms(t_buf),
                    ms(t_paint),
                    ms(t_copy),
                    ms(qp.elapsed()),
                    if full { " (всё окно)" } else { "" }
                ));
            }
            self.need_full = false;
        }
        self.surface = Some(surface);
    }

    /// «Развернуть»: картинка крупнее — самый большой масштаб клетки, при котором окно
    /// влезает в экран; повторное нажатие — вернуть прежний размер.
    fn toggle_maximize(&mut self) {
        let Some(win) = self.window.clone() else { return };
        // окно тянется свободно — разворачиваем средствами системы на весь экран (без панелей);
        // раскладку подстроит fit_window, когда придёт новый размер
        let maxed = !win.is_maximized();
        self.base_scale = if maxed { Some(self.scale) } else { None };
        win.set_maximized(maxed);
        self.need_full = true;
        wlog(if maxed { "развернул на весь экран" } else { "вернул размер" });
    }

    /// Самый мелкий масштаб клетки: картинка во всю высоту середины окна.
    fn min_scale(&self, k: i32) -> usize {
        // самое маленькое окно — ровно по картинке, без пустого места над ней
        ((Ui::MIN_BODY * k - 4 * k) as usize).div_ceil(self.sc.h).max(1)
    }

    /// Окно меняют за край: сразу перерисовать под новый размер. Картинка — самая крупная
    /// целого масштаба (пиксели чёткие), которая влезает; остальное место забирает правая часть.
    fn fit_window(&mut self, size: PhysicalSize<u32>) {
        let Some(k) = self.ui.as_ref().map(|u| u.k()) else { return };
        let min = self.min_scale(k);
        let mut best = min;
        for s in min + 1..64 {
            let (w, h) = Ui::size_for(k, (self.sc.w * s) as i32, (self.sc.h * s) as i32);
            if w as u32 <= size.width && h as u32 <= size.height {
                best = s;
            }
        }
        if best != self.scale {
            self.scale = best;
            self.screens = None; // экраны пересчитаются под новый масштаб
            wlog(&format!("клетка картинки ×{best}"));
        }
        let (aw, ah) = ((self.sc.w * best) as i32, (self.sc.h * best) as i32);
        // развёрнуто ли — спрашиваем систему (пользователь мог вернуть размер, потянув окно)
        let maxed = self.window.as_ref().is_some_and(|w| w.is_maximized());
        self.base_scale = if maxed { Some(best) } else { None };
        let Some(u) = self.ui.as_mut() else { return };
        u.resize(size.width as i32, size.height as i32, aw, ah, maxed);
        let (w, h) = (u.w, u.h);
        if let (Some(s), Some(nw), Some(nh)) = (self.surface.as_mut(), NonZeroU32::new(w as u32), NonZeroU32::new(h as u32)) {
            let _ = s.resize(nw, nh);
        }
        self.need_full = true;
        self.render(self.t0.elapsed().as_secs_f64());
        self.present();
        self.prev.copy_from_slice(&self.frame);
    }

    /// Меньше этого окно не сжимается (самый мелкий масштаб картинки).
    fn set_min_size(&self) {
        let Some(win) = &self.window else { return };
        let (w, h) = self.min_size();
        win.set_min_inner_size(Some(PhysicalSize::new(w as u32, h as u32)));
    }

    /// Самое маленькое окно — ровно по картинке при самом мелком масштабе.
    fn min_size(&self) -> (i32, i32) {
        let k = self.ui.as_ref().map(|u| u.k()).unwrap_or(1);
        let s = self.min_scale(k);
        Ui::size_for(k, (self.sc.w * s) as i32, (self.sc.h * s) as i32)
    }

    /// Край окна под точкой (верх не считаем: там заголовок и кнопки).
    fn edge_at(&self, x: f64, y: f64) -> Option<ResizeDirection> {
        let u = self.ui.as_ref()?;
        let k = u.k() as f64;
        // уголок в правом нижнем углу строки состояния — тянуть удобно
        if x >= u.w as f64 - 16.0 * k && y >= u.h as f64 - 16.0 * k {
            return Some(ResizeDirection::SouthEast);
        }
        let m = 6.0 * k;
        let (l, r, b) = (x < m, x >= u.w as f64 - m, y >= u.h as f64 - m);
        let top = y < 30.0 * k;
        Some(match (l, r, b) {
            (true, _, true) => ResizeDirection::SouthWest,
            (_, true, true) => ResizeDirection::SouthEast,
            (true, _, _) if !top => ResizeDirection::West,
            (_, true, _) if !top => ResizeDirection::East,
            (_, _, true) => ResizeDirection::South,
            _ => return None,
        })
    }

    fn build_ui(&mut self, k: i32) -> (i32, i32) {
        let (aw, ah) = ((self.sc.w * self.scale) as i32, (self.sc.h * self.scale) as i32);
        let u = Ui::new(k, aw, ah);
        let size = (u.w, u.h);
        self.ui = Some(u);
        size
    }
}

/// Высота потолка над картинкой в клетках картинки (art/layers/потолок.png покрывает ширину картинки × столько).
const CEIL_CELLS: usize = 126;
/// Ниже потолка в том же файле — «шов»: верхние клетки картинки так, как их перерисовал ChatGPT
/// (петля проводов замкнута); непрозрачен только там, где отличается от картинки.
const SEAM_CELLS: usize = 16;

fn window_icon() -> Option<winit::window::Icon> {
    let pm = gfx::decode_png(include_bytes!("../../assets/aptechka.png"))?;
    let small = gfx::box_scale(&pm, 64)?;
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for p in small.data().chunks(4) {
        let a = p[3] as u32;
        let un = |c: u8| (c as u32 * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
        rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), p[3]]);
    }
    winit::window::Icon::from_rgba(rgba, 64, 64).ok()
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mon = el.primary_monitor();
        let forced_k = env::<i32>("UI_K");
        let mut k = forced_k.unwrap_or_else(|| mon.as_ref().map(|m| m.scale_factor().round() as i32).unwrap_or(1)).clamp(1, 4);
        let mon_h = mon.as_ref().map(|m| m.size().height);
        let preset_scale = self.scale != 0;
        if !preset_scale {
            self.scale = pick_scale(k, self.sc.h, mon_h);
        }
        let (mut w, mut h) = self.build_ui(k);
        // дробный масштаб экрана (×1,5 округляется до ×2) на невысоком экране: окно выше экрана. Тогда
        // интерфейс на ступень мельче, пока не влезет (если масштаб не задан вручную через UI_K)
        while forced_k.is_none() && !preset_scale && k > 1 && mon_h.is_some_and(|mh| h as u32 > mh * 95 / 100) {
            k -= 1;
            self.scale = pick_scale(k, self.sc.h, mon_h);
            (w, h) = self.build_ui(k);
            wlog(&format!("окно не влезало по высоте — интерфейс ×{k}"));
        }
        let attrs = Window::default_attributes().with_title(i18n::t("Аптечка"));
        // WM_CLASS — чтобы док GNOME связал окно с ярлыком
        #[cfg(target_os = "linux")]
        let attrs = attrs.with_name("io.github.aptechka", "io.github.aptechka");
        let attrs = attrs
            .with_window_icon(window_icon())
            .with_decorations(false) // заголовок рисуем сами, в стиле XP
            .with_inner_size(PhysicalSize::new(w as u32, h as u32))
            .with_resizable(true) // тянуть за край: раскладка подстраивается сразу (fit_window)
            // показать после того, как подключены специальные возможности (AccessKit требует так)
            .with_visible(false);
        let Ok(window) = el.create_window(attrs) else {
            wlog("не удалось создать окно");
            el.exit();
            return;
        };
        let window = Rc::new(window);
        let (Ok(ctx), Some(nw), Some(nh)) = (softbuffer::Context::new(window.clone()), NonZeroU32::new(w as u32), NonZeroU32::new(h as u32)) else {
            wlog("softbuffer не поднялся");
            el.exit();
            return;
        };
        let Ok(mut surface) = softbuffer::Surface::new(&ctx, window.clone()) else {
            wlog("поверхность не создалась");
            el.exit();
            return;
        };
        let _ = surface.resize(nw, nh);
        if let Some(p) = self.proxy.clone() {
            self.a11y = Some(accesskit_winit::Adapter::with_event_loop_proxy(el, &window, p));
        }
        window.set_visible(true);
        self.window = Some(window);
        self.surface = Some(surface);
        self.set_min_size();
        wlog(&format!("{w}×{h}, интерфейс ×{k}, клетка картинки ×{}", self.scale));
        // окно открыли из значка в панели кнопкой «Аналитика расхода»
        self.analytics_request();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        if let (Some(a), Some(w)) = (self.a11y.as_mut(), self.window.as_ref()) {
            a.process_event(w, &ev);
        }
        // ручное изменение размера (macOS) — только пока кнопка нажата: новое нажатие, потеря фокуса
        // или отпускание, которое пришло без нашего Released, его заканчивают
        match &ev {
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                if *state == ElementState::Pressed {
                    self.manual_resize = None;
                }
                self.left_down = *state == ElementState::Pressed;
            }
            WindowEvent::Focused(false) => {
                self.manual_resize = None;
                self.left_down = false;
            }
            _ => {}
        }
        if let WindowEvent::Focused(f) = ev {
            self.focused = f;
            if f {
                // вернулись к окну — кадр сразу, а не через долгий шаг фоновой частоты
                self.next = Instant::now();
            }
        }
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::CursorMoved { position, .. } if self.manual_resize.is_some() => {
                // правый/нижний край на месте окна не двигает начало координат: размер = старт + сдвиг мыши
                self.cursor = (position.x, position.y);
                let Some((d, (x0, y0), (w0, h0))) = self.manual_resize else { return };
                if !self.left_down {
                    self.manual_resize = None;
                    return;
                }
                let dw = if matches!(d, ResizeDirection::South) { 0.0 } else { position.x - x0 };
                let dh = if matches!(d, ResizeDirection::East) { 0.0 } else { position.y - y0 };
                // не меньше «ровно по картинке»: программная смена размера системный минимум не учитывает
                let (mw, mh) = self.min_size();
                if let Some(w) = &self.window {
                    let _ = w.request_inner_size(PhysicalSize::new((w0 + dw).max(mw as f64) as u32, (h0 + dh).max(mh as f64) as u32));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                let edge = self.edge_at(position.x, position.y);
                if edge != self.edge {
                    self.edge = edge;
                    if let Some(w) = &self.window {
                        w.set_cursor(edge.map(CursorIcon::from).unwrap_or_default());
                    }
                }
                if let Some(u) = self.ui.as_mut() {
                    u.mouse_move(position.x as i32, position.y as i32);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(u) = self.ui.as_mut() {
                    u.mouse_leave();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.edge.is_some() => {
                let (Some(d), Some(w)) = (self.edge, self.window.clone()) else { return };
                // Linux и Windows тянут окно сами (как за рамку); macOS так не умеет — тянем вслед за мышью
                if w.drag_resize_window(d).is_err() && matches!(d, ResizeDirection::East | ResizeDirection::South | ResizeDirection::SouthEast) {
                    let s = w.inner_size();
                    self.manual_resize = Some((d, self.cursor, (s.width as f64, s.height as f64)));
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } if self.manual_resize.is_some() => {
                self.manual_resize = None;
            }
            WindowEvent::Resized(size) => {
                if !self.ui.as_ref().is_some_and(|u| u.w as u32 == size.width && u.h as u32 == size.height) {
                    self.fit_window(size);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let (x, y) = (self.cursor.0 as i32, self.cursor.1 as i32);
                let cmds = match (self.ui.as_mut(), state) {
                    (Some(u), ElementState::Pressed) => u.mouse_down(x, y),
                    (Some(u), ElementState::Released) => u.mouse_up(x, y),
                    _ => Vec::new(),
                };
                self.handle_cmds(cmds, Some(el));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 24.0,
                };
                if let Some(u) = self.ui.as_mut() {
                    u.wheel(self.cursor.0 as i32, self.cursor.1 as i32, lines);
                }
            }
            WindowEvent::ModifiersChanged(m) => self.alt = m.state().alt_key(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let alt = self.alt;
                let cmds = match (self.ui.as_mut(), &event.logical_key) {
                    // Alt+буква — меню (Alt+Ф «Файл», Alt+С «Справка» и т.д.)
                    (Some(u), Key::Character(ch)) if alt => {
                        if let Some(c) = ch.chars().next() {
                            u.key_alt(c);
                        }
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::ArrowDown)) => {
                        if !u.key_menu(1, 0) {
                            u.key_scroll(1);
                        }
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::ArrowUp)) => {
                        if !u.key_menu(-1, 0) {
                            u.key_scroll(-1);
                        }
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::ArrowLeft)) => {
                        u.key_menu(0, -1);
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::ArrowRight)) => {
                        u.key_menu(0, 1);
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::PageDown)) => {
                        u.key_scroll(15);
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::PageUp)) => {
                        u.key_scroll(-15);
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::Home)) => {
                        u.key_scroll(-100_000);
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::End)) => {
                        u.key_scroll(100_000);
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::Escape)) => {
                        u.key_escape();
                        Vec::new()
                    }
                    (Some(u), Key::Named(NamedKey::Enter)) => u.key_enter(),
                    _ => Vec::new(),
                };
                self.handle_cmds(cmds, Some(el));
            }
            WindowEvent::Occluded(hidden) => {
                self.hidden = hidden;
                if !hidden {
                    self.need_full = true;
                    self.next = Instant::now();
                    self.tick();
                }
            }
            WindowEvent::RedrawRequested => {
                self.need_full = true;
                self.present();
            }
            _ => {}
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: AppEvent) {
        match ev {
            // вторая копия попросила показать окно
            AppEvent::ShowWindow => {
                if let Some(w) = &self.window {
                    w.set_minimized(false);
                    w.set_visible(true);
                    w.focus_window();
                    log("вторая копия: показываю окно");
                }
                self.analytics_request();
            }
            AppEvent::A11y(e) => match e.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    // диктор подключился (в любой момент) — с этого времени дерево обновляется
                    self.a11y_on = true;
                    self.a11y_sig = 0;
                    let tree = self.a11y_tree();
                    if let Some(a) = self.a11y.as_mut() {
                        a.update_if_active(|| tree);
                    }
                }
                accesskit_winit::WindowEvent::ActionRequested(req) => {
                    use accesskit::Action;
                    if matches!(req.action, Action::Click) {
                        let cmds = self.ui.as_mut().map(|u| u.a11y_click(req.target_node.0)).unwrap_or_default();
                        log(&format!("специальные возможности: нажатие {:x}", req.target_node.0));
                        self.handle_cmds(cmds, Some(el));
                        self.a11y_refresh(true);
                    }
                }
                // диктор отключился — дерево 4 раза в секунду больше не строим
                accesskit_winit::WindowEvent::AccessibilityDeactivated => self.a11y_on = false,
            },
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.restart_now {
            self.restart_now = false;
            // новая копия с теми же аргументами; она дождётся, пока эта закроется (APTECHKA_REPLACES)
            if let Some(exe) = procs::self_exe() {
                let mut c = std::process::Command::new(exe);
                c.args(cli_args().into_iter().skip(1)).env("APTECHKA_REPLACES", std::process::id().to_string());
                if procs::spawn_detached(&mut c) {
                    el.exit();
                    return;
                }
            }
        }
        let now = Instant::now();
        // события мыши уже изменили интерфейс — показываем сразу, не дожидаясь кадра
        if self.ui.as_ref().is_some_and(|u| u.has_dirty()) && now < self.next {
            self.present();
        } else if now >= self.next {
            self.tick();
            // окна не видно — просыпаемся редко, только проверить, не показали ли его
            let step = frame_step(self.hidden || self.minimized() || self.covered, self.focused, self.fps);
            // без анимации — раз в секунду (данные, часы на экранчиках); нажатия всё равно сразу
            let step = if self.anim { step } else { step.max(1.0) };
            self.next = now + Duration::from_secs_f64(step);
        }
        // во время анимации интерфейса — 60 кадров в секунду, иначе обычный темп
        let wake = if self.ui.as_ref().is_some_and(|u| u.animating()) {
            let soon = Instant::now() + Duration::from_millis(16);
            if soon < self.next {
                self.next = soon;
            }
            self.next
        } else {
            self.next
        };
        el.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}

/// Окно не видно (скрыто, свёрнуто, закрыто другими): сессии опрашиваются реже. И просьба опросить сейчас.
static SESS_QUIET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SESS_POKE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Шаг между кадрами, секунды: окна не видно (закрыто другим окном, свёрнуто) — раз в секунду, только принять
/// результаты проверок; на виду, но без фокуса — втрое реже обычного; в фокусе — обычный темп.
fn frame_step(hidden: bool, focused: bool, fps: f64) -> f64 {
    if hidden {
        1.0
    } else if !focused {
        // без фокуса — 4 кадра в секунду при 20 (моргание ещё видно; реже — кадры заметно пропадают)
        5.0 / fps
    } else {
        1.0 / fps
    }
}

/// Скромнее к другим программам: допуск таймера 2 мс, nice 5; CORES=8-15,24-31 — привязать к ядрам
/// (на 9950X3D кэш-блок 0–7 оставляем играм). SCHED=0 — выключить. Только Linux.
#[cfg(target_os = "linux")]
fn tune_scheduler() {
    if std::env::var("SCHED").as_deref() == Ok("0") {
        return;
    }
    extern "C" {
        fn sched_setaffinity(pid: i32, size: usize, mask: *const u64) -> i32;
        fn prctl(op: i32, arg: u64, a3: u64, a4: u64, a5: u64) -> i32;
        fn setpriority(which: i32, who: u32, prio: i32) -> i32;
    }
    // привязка к ядрам — только если попросили (CORES=8-15,24-31): у каждого процессора свои ядра
    if let Ok(spec) = std::env::var("CORES") {
        let mut mask = [0u64; 16];
        for part in spec.split(',') {
            let (a, b) = part.split_once('-').unwrap_or((part, part));
            if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                for c in a..=b.min(1023) {
                    mask[c / 64] |= 1 << (c % 64);
                }
            }
        }
        unsafe {
            sched_setaffinity(0, std::mem::size_of_val(&mask), mask.as_ptr());
        }
    }
    unsafe {
        prctl(29, 2_000_000, 0, 0, 0); // PR_SET_TIMERSLACK
        setpriority(0, 0, 5);
    }
}

#[cfg(not(target_os = "linux"))]
fn tune_scheduler() {}

/// Записать готовый RGB-буфер в PNG.
fn save_rgb(path: &str, w: usize, h: usize, rgb: &[u8]) -> Result<(), String> {
    // новый файл, не поверх существующего и не по ссылке (create_new = O_EXCL)
    let f = std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    let mut wr = enc.write_header().map_err(|e| e.to_string())?;
    wr.write_image_data(rgb).map_err(|e| e.to_string())
}

fn save_png(path: &str, w: usize, h: usize, s: usize, frame: &[u32]) {
    let f = std::fs::File::create(path).expect("файл кадра");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), (w * s) as u32, (h * s) as u32);
    enc.set_color(png::ColorType::Rgb);
    let mut wr = enc.write_header().expect("png");
    let mut data = Vec::with_capacity(w * h * s * s * 3);
    for y in 0..h * s {
        for x in 0..w * s {
            let p = frame[(y / s) * w + x / s];
            data.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    wr.write_image_data(&data).expect("png");
}

/// UI_SHOT: нарисовать всё окно в PNG без показа на экране.
fn ui_shot(app: &mut App, path: &str) {
    let fps = app.fps;
    let k = env::<i32>("UI_K").unwrap_or(1).clamp(1, 4);
    if app.scale == 0 {
        app.scale = pick_scale(k, app.sc.h, None);
    }
    let (mut w, mut h) = app.build_ui(k);
    // UI_H=1500 — окно выше картинки (видно потолок над ней)
    if let (Some(hh), Some(u)) = (env::<i32>("UI_H"), app.ui.as_mut()) {
        let (aw, ah) = ((app.sc.w * app.scale) as i32, (app.sc.h * app.scale) as i32);
        u.resize(w, hh.max(h), aw, ah, false);
        (w, h) = (u.w, u.h);
    }
    let fake = std::env::var("UI_FAKE").ok();
    let secs: f64 = env("T").unwrap_or(2.0);
    let n = (secs * fps) as usize;
    // данные: образцы (UI_FAKE) или настоящие проверки (читают систему, ничего не меняют)
    let (checks, sessions) = match &fake {
        Some(kind) => (ui::demo_checks(kind), ui::demo_sessions()),
        None => (diag::run_all(), diag::all_sessions(12)),
    };
    for i in 0..n.max(2) {
        app.render(i as f64 / fps);
    }
    let t = n as f64 / fps;
    app.first_checks = false;
    app.apply_sessions(sessions, t);
    app.apply_checks(checks, true, t);
    if fake.is_some() {
        // образец замера скорости, чтобы строка скорости и экран в картинке были с цифрами
        let res = ui::demo_speed();
        app.speed_checks = res.clone();
        if let Some(u) = app.ui.as_mut() {
            u.speed_done(res);
        }
        app.refresh_sstate(t);
    }
    let mut cmds = Vec::new();
    for what in std::env::var("UI_DO").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        if what == "speed" {
            // ход замера (образец): скачивание на 63 %
            if let Some(u) = app.ui.as_mut() {
                u.demo_state(Some((diag::SpeedPhase::Down, 158.0, 0.63)));
            }
        } else if let Some(u) = app.ui.as_mut() {
            cmds.extend(u.script(what));
        }
    }
    // UI_CLICK="x,y;x,y" — настоящие нажатия мышью по окну (проверка областей и меню)
    for xy in std::env::var("UI_CLICK").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        let Some((x, y)) = xy.split_once(',').and_then(|(a, b)| Some((a.trim().parse::<i32>().ok()?, b.trim().parse::<i32>().ok()?))) else {
            continue;
        };
        if let Some(u) = app.ui.as_mut() {
            u.mouse_move(x, y);
            cmds.extend(u.mouse_down(x, y));
            cmds.extend(u.mouse_up(x, y));
        }
    }
    // UI_POKE="x,y;x,y" — тычки в картинку (x,y в клетках), чтобы проверить реплики в облачке
    for xy in std::env::var("UI_POKE").unwrap_or_default().split(';').filter(|s| !s.is_empty()) {
        if let Some((x, y)) = xy.split_once(',').and_then(|(a, b)| Some((a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?))) {
            app.click(x, y, t);
        }
    }
    // команды окна: настоящие запуски только в пробном режиме (DRY_RUN=1), иначе лишь пишем
    for c in &cmds {
        wlog(&format!("команда окна: {c:?}"));
    }
    if fixes::dry_run() {
        let run: Vec<Cmd> = cmds.into_iter().filter(|c| matches!(c, Cmd::RunFix { .. } | Cmd::SaveSpravka)).collect();
        app.handle_cmds(run, None);
        std::thread::sleep(Duration::from_millis(400));
        app.drain_bg(t);
    }
    for i in 0..3 {
        app.render(t + (i + 1) as f64 / fps);
    }
    let mut buf = vec![0u32; (w * h) as usize];
    app.screens_dirty = true;
    app.paint_frame(&mut buf, true);
    save_png(path, w as usize, h as usize, 1, &buf);
    // BENCH_TAB=1 — замер: сколько стоит перерисовка после переключения вкладки
    if std::env::var_os("BENCH_TAB").is_some() {
        let mut worst = 0.0f64;
        let mut sum = 0.0;
        for i in 0..40 {
            if let Some(u) = app.ui.as_mut() {
                let _ = u.script(&format!("tab{}", i % 4));
            }
            let q = Instant::now();
            app.paint_frame(&mut buf, false);
            let ms = q.elapsed().as_secs_f64() * 1000.0;
            worst = worst.max(ms);
            sum += ms;
        }
        wlog(&format!("переключение вкладки: в среднем {:.1} мс, худшее {:.1} мс", sum / 40.0, worst));
    }
    wlog(&format!("снимок окна {w}×{h} (интерфейс ×{k}, клетка ×{}) → {path}", app.scale));
}

/// Windows: у оконной программы своей консоли нет — для текстовых режимов берём консоль того, кто запустил.
#[cfg(windows)]
fn attach_console() {
    extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    unsafe {
        AttachConsole(u32::MAX); // ATTACH_PARENT_PROCESS
    }
}

/// aptechka --help
const HELP: &str = "Аптечка — проверка компьютера с котом-доктором.\n\n\
aptechka                 открыть окно\n\
aptechka --diag          все проверки таблицей в терминале\n\
aptechka --sessions      сессии ИИ-агентов и как их продолжить\n\
aptechka --speed         замер скорости в терминале\n\
aptechka --widget        только виджет на рабочем столе (Linux, Windows)\n\
aptechka --tray          только значок с лимитами ИИ в панели (Linux, Windows)\n\
aptechka --fix ID [ARG]  одна починка; DRY_RUN=1 — только показать, что было бы\n\
aptechka --claude-limits on|off  лимиты Claude Code через его строку состояния (off — вернуть прежнюю)\n\n\
Язык: APTECHKA_LANG=ru|en";

fn main() {
    // строка состояния Claude Code: зовётся часто, поэтому — первым делом, без подготовки остального
    // (и до подключения к консоли на Windows: вход и выход — каналы от Claude Code)
    if cli_args().get(1).is_some_and(|a| a == "--statusline") {
        limits::statusline_main();
        return;
    }
    #[cfg(windows)]
    if cli_args().iter().skip(1).any(|a| a.starts_with("--")) || std::env::var_os("UI_SHOT").is_some() {
        attach_console();
    }
    if cli_args().iter().any(|a| a == "--help" || a == "-h") {
        println!("{} {}\n", i18n::t("Аптечка"), env!("CARGO_PKG_VERSION"));
        println!("{}", i18n::tr(HELP));
        return;
    }
    if let Some(i) = cli_args().iter().position(|a| a == "--claude-limits") {
        let want = cli_args().get(i + 1).map(|v| v != "off").unwrap_or(true);
        limits::sync_with(Some(want));
        return;
    }
    if let Ok(path) = std::env::var("TRAY_SHOT") {
        tray::shot(&path);
        return;
    }
    if cli_args().iter().any(|a| a == "--tray") {
        tray::run();
        return;
    }
    if cli_args().iter().any(|a| a == "--diag") {
        diag::print_table();
        return;
    }
    if cli_args().iter().any(|a| a == "--sessions") {
        diag::print_sessions();
        return;
    }
    if cli_args().iter().any(|a| a == "--speed") {
        diag::print_speed();
        return;
    }
    if std::env::var("DRY_RUN").as_deref() == Ok("1") {
        fixes::set_dry_run(true);
        log("DRY_RUN=1: лечение только пишет команды в журнал");
    }
    // виджет: маленькое окно на рабочем столе (или его снимок без окна)
    if let Ok(path) = std::env::var("WIDGET_SHOT") {
        widget::shot(&path);
        return;
    }
    if cli_args().iter().any(|a| a == "--widget") {
        // macOS: виджет — системный (WidgetKit, «Изменить виджеты» на рабочем столе); окна-виджета там нет
        if cfg!(target_os = "macos") {
            eprintln!("{}", i18n::tr("На macOS виджет Аптечки добавляется через «Изменить виджеты» на рабочем столе."));
            return;
        }
        widget::run();
        return;
    }
    // aptechka --fix <действие> [параметр] — одно лечение из терминала (с DRY_RUN=1 — пробно)
    let args: Vec<String> = cli_args();
    if let Some(i) = args.iter().position(|a| a == "--fix") {
        let Some(id) = args.get(i + 1) else {
            eprintln!("{}", i18n::tr("Нужно: aptechka --fix <действие> [параметр]"));
            std::process::exit(2);
        };
        let (ok, msg) = fixes::apply(id, args.get(i + 2).map(String::as_str));
        println!("{} {}", if ok { "OK:" } else { i18n::pick("НЕ ВЫШЛО:", "FAILED:") }, diag::term_safe(&i18n::tr(&msg)));
        std::process::exit(if ok { 0 } else { 1 });
    }
    tune_scheduler();
    crate::store::housekeeping();
    // вторая копия — сразу «покажись» первой и выход, ещё до загрузки картинок; снимки и запись кадров
    // окон не открывают — им «одна копия» не нужна
    // перезапуск новой версией: подождать (до 5 с), пока прежняя копия закроется и освободит «ящик»
    if let Some(old) = std::env::var("APTECHKA_REPLACES").ok().and_then(|v| v.parse::<i64>().ok()) {
        for _ in 0..50 {
            if !procs::list_fresh().iter().any(|p| p.pid == old) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let single_box = if std::env::var_os("FRAMES").is_some() || std::env::var_os("UI_SHOT").is_some() {
        None
    } else {
        match single::claim() {
            Ok(Some(l)) => {
                // приложение (пере)запустилось — «Игнорировать» снимается: за это время могла появиться новая беда
                crate::prefs::clear_ignored();
                Some(l)
            }
            Ok(None) => {
                log("Аптечка уже открыта — показываю её окно");
                return;
            }
            Err(e) => {
                log(&format!("не смог проверить вторую копию ({e}), запускаюсь так"));
                None
            }
        }
    };
    // картинки вшиты; ART=папка — брать с диска (для правки поз без пересборки)
    let art = std::env::var("ART").unwrap_or_default();
    let t = Instant::now();
    let sc = Scene::load(&art);
    log(&format!("слоёв {}, холст {}×{}, загрузка {:.0} мс", sc.layers.len(), sc.w, sc.h, t.elapsed().as_secs_f64() * 1000.0));
    let mut life = Life::new(&sc, &art, env("SEED").unwrap_or(0x5EED));
    // TROUBLE=1 — беспокоится всё время (как будто диагностика нашла проблему)
    life.trouble = std::env::var("TROUBLE").as_deref() == Ok("1");
    let n = sc.w * sc.h;
    // частота кадров — конечная и разумная (FPS=0, отрицательная или NaN роняли окно на делении)
    let fps: f64 = env::<f64>("FPS").filter(|f| f.is_finite() && (1.0..=240.0).contains(f)).unwrap_or(20.0);
    let headless = std::env::var("FRAMES").is_ok() || std::env::var("UI_SHOT").is_ok();
    let (bg_tx, bg_rx) = channel();
    let mut app = App {
        draws: vec![Draw::default(); sc.layers.len()],
        frame: vec![0; n],
        owner: vec![u16::MAX; n],
        cursor: (0.0, 0.0),
        // CLICK="1.5:84,146;4:62,243" — тычки в режиме записи кадров (секунда:x,y)
        clicks: std::env::var("CLICK")
            .unwrap_or_default()
            .split(';')
            .filter_map(|c| {
                let (t, xy) = c.split_once(':')?;
                let (x, y) = xy.split_once(',')?;
                Some((t.parse().ok()?, x.parse().ok()?, y.parse().ok()?))
            })
            .collect(),
        prev: vec![0; n],
        scale: env::<usize>("SCALE").filter(|s| *s <= 16).unwrap_or(0),
        fps,
        anim: crate::prefs::app_anim(),
        sc,
        ceiling: scene::load_png(&art, "потолок.png"),
        ceiling_px: (0, 0, 0, Vec::new()),
        back: Vec::new(),
        hidden: false,
        covered: false,
        cover_t: Instant::now(),
        #[cfg(all(unix, not(target_os = "macos")))]
        cover: None,
        focused: true,
        last_present: Instant::now(),
        exe_stamp: procs::self_stamp(),
        update_sec: 0,
        told_update: false,
        fixes_running: 0,
        restart_now: false,
        life,
        quips: quips::Quips::new(),
        window: None,
        surface: None,
        t0: Instant::now(),
        last_t: 0.0,
        next: Instant::now(),
        need_full: true,
        stat: (0, Duration::ZERO, Instant::now()),
        art: art.clone(),
        screens: None,
        sstate: ScreenState::default(),
        screens_t: -1.0,
        screens_dirty: false,
        // проверки — сразу и потом раз в минуту (NODIAG=1 — без них, для записи кадров)
        diag_rx: if std::env::var("NODIAG").is_ok() || headless {
            None
        } else {
            // расписание: у каждой проверки своя частота, проверка по событию; общий результат с виджетом
            Some(sched::start())
        },
        trouble_env: std::env::var("TROUBLE").as_deref() == Ok("1"),
        ui: None,
        bg_tx,
        bg_rx,
        checks: Vec::new(),
        raw_checks: Vec::new(),
        speed_checks: Vec::new(),
        sessions: Vec::new(),
        first_checks: true,
        checks_running: false,
        recheck_at: None,
        last_sec: 0,
        avatar_done: false,
        base_scale: None,
        alt: false,
        edge: None,
        manual_resize: None,
        left_down: false,
        a11y: None,
        proxy: None,
        a11y_sig: 0,
        a11y_on: false,
        a11y_t: Instant::now(),
    };

    // сессии Claude Code: обновляем раз в несколько секунд (это дёшево)
    if !headless && std::env::var("NODIAG").is_err() {
        let tx = app.bg_tx.clone();
        std::thread::spawn(move || loop {
            if tx.send(Bg::Sessions(diag::all_sessions(12))).is_err() {
                break;
            }
            // окно на виду — раз в 4 с; скрыто, свёрнуто, закрыто другими — раз в 30 с (опрос — это обход
            // процессов и папок сессий); показали снова — сразу (SESS_POKE)
            let every = if SESS_QUIET.load(std::sync::atomic::Ordering::Relaxed) { 30_000 } else { 4_000 };
            let mut waited = 0;
            while waited < every && !SESS_POKE.swap(false, std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(250));
                waited += 250;
            }
        });
    }

    // UI_SHOT=файл.png — снимок всего окна без показа
    if let Ok(path) = std::env::var("UI_SHOT") {
        ui_shot(&mut app, &path);
        return;
    }

    // FRAMES=папка N=кадров — записать кадры без окна (время идёт по 1/FPS)
    if let Ok(dir) = std::env::var("FRAMES") {
        let count: usize = env("N").unwrap_or(200);
        let s = app.scale.max(1);
        std::fs::create_dir_all(&dir).expect("папка кадров");
        let t = Instant::now();
        app.scale = s;
        if let Some(sec) = env::<f64>("DIAG_WAIT") {
            // подождать результатов проверок перед записью (чтобы экраны были с данными)
            std::thread::sleep(Duration::from_secs_f64(sec));
        }
        let (w, h) = (app.sc.w, app.sc.h);
        let mut win = vec![0u32; w * s * h * s];
        for i in 0..count {
            app.render(i as f64 / fps);
            upscale(&app.frame, w, s, &mut win, w * s, 0, 0, &vec![true; h]);
            if let Some(scr) = &app.screens {
                scr.overlay(&mut win, w * s, 0, 0);
            }
            save_png(&format!("{dir}/{i:05}.png"), w * s, h * s, 1, &win);
        }
        log(&format!("записано {count} кадров в {dir} за {:.1} с", t.elapsed().as_secs_f64()));
        return;
    }
    // нет экрана (запуск по ssh, без графической сессии): понятное сообщение вместо падения
    let el = match EventLoop::<AppEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            log(&format!("нет экрана: {e}"));
            eprintln!(
                "{}",
                i18n::tr("Аптечке нужен экран: не нашёл графическую сессию.\nЗапусти её из рабочего стола. Проверка без окна: aptechka --diag")
            );
            std::process::exit(2);
        }
    };
    app.proxy = Some(el.create_proxy());
    // одна копия: если Аптечка уже открыта, показываем её окно и выходим
    if let Some(l) = single_box {
        let proxy = el.create_proxy();
        single::listen(l, move || {
            let _ = proxy.send_event(AppEvent::ShowWindow);
        });
    }
    if let Err(e) = el.run_app(&mut app) {
        eprintln!("{}", i18n::tr(&format!("Окно Аптечки закрылось с ошибкой: {e}")));
        std::process::exit(1);
    }
    // окно закрыто: данные измерений в папке сеанса больше не нужны
    sched::cleanup_on_exit();
}

#[cfg(test)]
mod frame_step_tests {
    use super::frame_step;

    #[test]
    fn throttled_when_nobody_looks() {
        assert_eq!(frame_step(false, true, 20.0), 0.05);
        assert!((frame_step(false, false, 20.0) - 0.25).abs() < 1e-9);
        assert_eq!(frame_step(true, true, 20.0), 1.0);
        assert_eq!(frame_step(true, false, 20.0), 1.0);
    }
}
