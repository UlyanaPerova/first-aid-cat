//! Значок в верхней панели (`aptechka --tray`): самый заполненный лимит ИИ рядом с котом («64%»),
//! по нажатию — окошко с лимитами всех агентов, временем сброса и расходом токенов.
//!
//! Отдельный лёгкий процесс: окна у него нет, пока не нажали на значок; данные (limits.rs) обновляются
//! раз в 30 секунд и читают журналы по кусочку. Один на пользователя (замок tray.lock).
//! Linux — StatusNotifierItem по D-Bus (в GNOME его показывает расширение AppIndicator, текст рядом со
//! значком — свойство XAyatanaLabel; в KDE текста нет, только значок и подсказка).
// macOS: значка в строке меню пока нет (следующий шаг) — общие части модуля там пока не используются
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::gfx::{Canvas, Fonts, Rect};
use crate::limits::Agent;

/// Как часто обновлять цифры.
const EVERY: Duration = Duration::from_secs(30);

/// Кот на значке показывает, что делают агенты (нарисован ChatGPT, белый силуэт на прозрачном, 64×64):
/// спит — агенты выключены, идёт — какой-то думает, сидит — работа закончена (сессия открыта и ждёт).
const ICONS: [&[u8]; 3] =
    [include_bytes!("../assets/значок_кот_спит.png"), include_bytes!("../assets/значок_кот_идёт.png"), include_bytes!("../assets/значок_кот.png")];

/// Состояние агентов для значка: 0 — спят (нет открытых сессий), 1 — работают, 2 — закончили.
static STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn icon_choice() -> u8 {
    STATE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Агенты, о которых умеем показывать лимиты (для меню «Следить за: …»).
pub const AGENTS: [&str; 2] = ["Claude Code", "Codex"];

/// Следим ли за агентом (по умолчанию — да; снятые — в limits_hide в ui.json).
pub fn shown(name: &str) -> bool {
    !crate::prefs::load().get("limits_hide").and_then(|v| v.as_array()).is_some_and(|a| a.iter().any(|x| x.as_str() == Some(name)))
}

/// Включить или выключить слежение за агентом.
pub fn toggle_shown(name: &str) {
    let hide = shown(name);
    crate::prefs::update(|v| {
        let mut list: Vec<String> = v
            .get("limits_hide")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        list.retain(|x| x != name);
        if hide {
            list.push(name.to_string());
        }
        v["limits_hide"] = serde_json::json!(list);
    });
}

/// Отпечаток выбора агентов (чтобы значок заметил перемену).
fn shown_sig() -> Vec<bool> {
    let mut v: Vec<bool> = AGENTS.iter().map(|a| shown(a)).collect();
    v.push(label_each());
    v
}

/// Состояние агентов, за которыми следим: хоть один думает — 1; есть открытые сессии — 2; иначе 0.
fn agents_state() -> u8 {
    let ids: Vec<&str> = AGENTS.iter().filter(|a| shown(a)).map(|a| if a.starts_with("Claude") { "claude" } else { "codex" }).collect();
    let live: Vec<_> = crate::diag::all_sessions(30).into_iter().filter(|s| s.live && ids.contains(&s.agent)).collect();
    if live.iter().any(|s| s.status == "busy") {
        1
    } else if !live.is_empty() {
        2
    } else {
        0
    }
}

#[derive(Debug)]
pub enum Ev {
    /// новые цифры
    Data(Vec<Agent>),
    /// нажали на значок (где — если панель сказала; 0,0 — не сказала)
    Click(i32, i32),
    /// навели или нажали на значок нашего расширения GNOME: показать плашку (не закрывать, если открыта)
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Show(i32, i32),
    /// поменяли, за какими агентами следить
    Shown,
    /// значок выключили в настройках
    Quit,
    /// выбрали другой значок
    Icon,
    /// файл программы обновили — перезапуститься новой версией
    Update,
}

/// Текст рядом со значком: «64%», или пусто, если лимиты неизвестны (на Windows — число на самом значке).
#[cfg_attr(windows, allow(dead_code))]
pub fn label(agents: &[Agent], now: i64) -> String {
    // перед числом — широкий пробел: панель ставит текст вплотную к значку, а у соседних показателей
    // (температура, загрузка) между значком и числом есть воздух
    if label_each() {
        // у каждого агента своё: «Cl 10%  Co 13%»
        let parts: Vec<String> = agents.iter().filter_map(|a| Some(format!("{} {:.0}%", short(a.name), a.limits?.top(now)?))).collect();
        return if parts.is_empty() { String::new() } else { format!("\u{2002}{}", parts.join("\u{2002}")) };
    }
    crate::limits::top(agents, now).map(|p| format!("\u{2002}{p:.0}%")).unwrap_or_default()
}

/// Короткое имя агента для панели.
fn short(name: &str) -> &'static str {
    if name.starts_with("Claude") {
        "Cl"
    } else {
        "Co"
    }
}

/// Число в панели — у каждого агента (иначе — самый занятый лимит).
pub fn label_each() -> bool {
    crate::prefs::load().get("tray_label").and_then(|v| v.as_str()) == Some("each")
}

pub fn set_label_each(on: bool) {
    crate::prefs::update(|v| v["tray_label"] = serde_json::json!(if on { "each" } else { "max" }));
}

/// Уведомлять о лимитах (80 %, 95 %, сброс) — по умолчанию да.
pub fn notify_on() -> bool {
    crate::prefs::load().get("limits_notify").and_then(|v| v.as_bool()).unwrap_or(true)
}

pub fn set_notify(on: bool) {
    crate::prefs::update(|v| v["limits_notify"] = serde_json::json!(on));
}

/// Подсказка при наведении: по строке на агента.
pub fn tooltip(agents: &[Agent], now: i64) -> String {
    if agents.is_empty() {
        return "Лимиты ИИ: пока нет данных".into();
    }
    agents
        .iter()
        .map(|a| {
            let mut parts = Vec::new();
            if let Some(l) = a.limits {
                if let Some(w) = l.five {
                    parts.push(format!("5 ч {:.0}%", w.now_pct(now)));
                }
                if let Some(w) = l.week {
                    parts.push(format!("неделя {:.0}%", w.now_pct(now)));
                }
            }
            parts.push(format!("сегодня {}", crate::limits::fmt_tokens(a.today)));
            format!("{}: {}", a.name, parts.join(" · "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// PNG из RGBA.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut e = png::Encoder::new(&mut buf, w, h);
        e.set_color(png::ColorType::Rgba);
        if let Ok(mut wr) = e.write_header() {
            let _ = wr.write_image_data(rgba);
        }
    }
    buf
}

/// Значок нужного размера (ARGB, уменьшение средним по клеткам).
fn icon_rgba(size: u32) -> Option<(u32, Vec<[u8; 4]>)> {
    let pm = crate::gfx::decode_png(ICONS[icon_choice() as usize % ICONS.len()])?;
    let (sw, sh) = (pm.width() as usize, pm.height() as usize);
    let d = pm.data();
    let n = size as usize;
    let mut out = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let (x0, x1) = (x * sw / n, ((x + 1) * sw / n).max(x * sw / n + 1));
            let (y0, y1) = (y * sh / n, ((y + 1) * sh / n).max(y * sh / n + 1));
            let mut acc = [0u32; 4];
            let mut cnt = 0;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let k = (yy * sw + xx) * 4;
                    for (i, a) in acc.iter_mut().enumerate() {
                        *a += d[k + i] as u32;
                    }
                    cnt += 1;
                }
            }
            // предумноженный RGBA → обычный ARGB
            let a = acc[3] / cnt;
            let un = |c: u32| (c / cnt * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
            out.push([a as u8, un(acc[0]), un(acc[1]), un(acc[2])]);
        }
    }
    Some((size, out))
}

// ───────────────────────────── Linux: StatusNotifierItem ─────────────────────────────

#[cfg(target_os = "linux")]
mod sni {
    use super::Ev;
    use std::sync::{Arc, Mutex};
    use winit::event_loop::EventLoopProxy;

    type Pixmap = (i32, i32, Vec<u8>);

    #[derive(Default)]
    pub struct State {
        pub label: String,
        pub tip: String,
        pub icons: Vec<Pixmap>,
        /// плашка картинкой для меню расширения GNOME: путь, ширина, высота, кнопка «Медкарта», «×»
        pub panel: Option<(String, u32, u32, [i32; 4], [i32; 4])>,
    }

    pub struct Item {
        pub state: Arc<Mutex<State>>,
        pub proxy: EventLoopProxy<Ev>,
    }

    /// Значок в нескольких размерах (ARGB32 в сетевом порядке байт) — панель выберет подходящий.
    fn pixmaps() -> Vec<Pixmap> {
        [22u32, 32, 44].iter().filter_map(|&s| super::icon_rgba(s)).map(|(s, px)| (s as i32, s as i32, px.into_iter().flatten().collect())).collect()
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Item {
        #[zbus(property)]
        fn category(&self) -> &str {
            "ApplicationStatus"
        }
        #[zbus(property)]
        fn id(&self) -> &str {
            "aptechka-limits"
        }
        #[zbus(property)]
        fn title(&self) -> String {
            crate::i18n::t("Лимиты ИИ")
        }
        #[zbus(property)]
        fn status(&self) -> &str {
            "Active"
        }
        #[zbus(property)]
        fn icon_name(&self) -> &str {
            ""
        }
        #[zbus(property)]
        fn icon_pixmap(&self) -> Vec<Pixmap> {
            self.state.lock().map(|s| s.icons.clone()).unwrap_or_default()
        }
        #[zbus(property)]
        fn tool_tip(&self) -> (String, Vec<Pixmap>, String, String) {
            let tip = self.state.lock().map(|s| s.tip.clone()).unwrap_or_default();
            (String::new(), Vec::new(), crate::i18n::t("Лимиты ИИ"), crate::i18n::t(&tip))
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn menu(&self) -> zbus::zvariant::OwnedObjectPath {
            zbus::zvariant::OwnedObjectPath::try_from("/NO_DBUSMENU").expect("правильный путь")
        }
        #[zbus(property, name = "XAyatanaLabel")]
        fn x_ayatana_label(&self) -> String {
            self.state.lock().map(|s| s.label.clone()).unwrap_or_default()
        }
        #[zbus(property, name = "XAyatanaLabelGuide")]
        fn x_ayatana_label_guide(&self) -> &str {
            "\u{2002}100%"
        }
        fn activate(&self, x: i32, y: i32) {
            let _ = self.proxy.send_event(Ev::Click(x, y));
        }
        fn secondary_activate(&self, x: i32, y: i32) {
            let _ = self.proxy.send_event(Ev::Click(x, y));
        }
        fn context_menu(&self, x: i32, y: i32) {
            let _ = self.proxy.send_event(Ev::Click(x, y));
        }
        fn scroll(&self, _delta: i32, _orientation: &str) {}

        #[zbus(signal)]
        async fn new_icon(e: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
        #[zbus(signal)]
        async fn new_tool_tip(e: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
        #[zbus(signal, name = "XAyatanaNewLabel")]
        async fn x_ayatana_new_label(e: &zbus::object_server::SignalEmitter<'_>, label: &str, guide: &str) -> zbus::Result<()>;
    }

    /// Значок на шине. Два подключения: `conn` — для нашего расширения GNOME (имя io.github.aptechka.Tray,
    /// всегда), `sni` — значок через AppIndicator (своё подключение: закрыл его — значок пропал из панели;
    /// с одним подключением значок держался за имя расширения и не исчезал — получалось два кота).
    pub struct Tray {
        conn: zbus::blocking::Connection,
        sni: std::cell::RefCell<Option<zbus::blocking::Connection>>,
        state: Arc<Mutex<State>>,
        proxy: EventLoopProxy<Ev>,
        name: String,
    }

    /// Наше расширение GNOME зовёт программу: плашка-окно (не GNOME) и «Медкарта расхода».
    pub struct Shell {
        pub proxy: EventLoopProxy<Ev>,
    }

    #[zbus::interface(name = "io.github.aptechka.Tray")]
    impl Shell {
        fn show(&self, x: i32, y: i32) {
            let _ = self.proxy.send_event(Ev::Show(x, y));
        }
        /// щелчок по значку: открыть плашку, второй щелчок — закрыть
        fn toggle(&self, x: i32, y: i32) {
            let _ = self.proxy.send_event(Ev::Click(x, y));
        }
        /// кнопка «Медкарта расхода…» на плашке в меню GNOME
        fn open_analytics(&self) {
            super::open_analytics();
        }
    }

    /// Имя, которое держит работающее расширение, и имя, по которому оно зовёт программу.
    const SHELL_ICON: &str = "io.github.aptechka.ShellIcon";
    const BUS: &str = "io.github.aptechka.Tray";

    const PATH: &str = "/StatusNotifierItem";

    impl Tray {
        pub fn start(proxy: EventLoopProxy<Ev>) -> Option<Tray> {
            let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
            let state = Arc::new(Mutex::new(State { icons: pixmaps(), ..Default::default() }));
            let conn = zbus::blocking::connection::Builder::session()
                .ok()?
                .name(BUS)
                .ok()?
                .serve_at("/io/github/aptechka/Tray", Shell { proxy: proxy.clone() })
                .ok()?
                .build()
                .ok()?;
            let t = Tray { conn, sni: std::cell::RefCell::new(None), state, proxy, name };
            t.register();
            Some(t)
        }

        /// Работает ли наше расширение GNOME (тогда значок рисует оно).
        pub fn shell_icon(&self) -> bool {
            self.conn
                .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(SHELL_ICON,))
                .ok()
                .and_then(|m| m.body().deserialize::<(bool,)>().ok())
                .is_some_and(|(b,)| b)
        }

        /// Показать значок через AppIndicator — если наше расширение не работает; работает — закрыть это
        /// подключение (панель сама уберёт значок). Повторно — безвредно (панель могла перезапуститься).
        pub fn register(&self) -> bool {
            if self.shell_icon() {
                if self.sni.borrow_mut().take().is_some() {
                    crate::diag::log("значок в панели: рисует расширение GNOME — значок AppIndicator убран");
                }
                return true;
            }
            let mut sni = self.sni.borrow_mut();
            if sni.is_none() {
                let item = Item { state: self.state.clone(), proxy: self.proxy.clone() };
                *sni = zbus::blocking::connection::Builder::session()
                    .ok()
                    .and_then(|b| b.name(self.name.as_str()).ok())
                    .and_then(|b| b.serve_at(PATH, item).ok())
                    .and_then(|b| b.build().ok());
            }
            let Some(c) = sni.as_ref() else { return false };
            let ok = c
                .call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "RegisterStatusNotifierItem",
                    &(self.name.as_str(),),
                )
                .is_ok();
            if !ok {
                crate::diag::log("значок в панели: нет StatusNotifierWatcher (в GNOME нужно расширение AppIndicator)");
            }
            ok
        }

        /// Плашка картинкой для меню нашего расширения GNOME: путь, размер (пиксели экрана) и где кнопки.
        pub fn set_panel(&self, path: String, w: u32, h: u32, btn: [i32; 4], close: [i32; 4]) {
            if let Ok(mut s) = self.state.lock() {
                s.panel = Some((path, w, h, btn, close));
            }
            self.write_shell_state();
        }

        /// Для нашего расширения GNOME: tray.json в папке сеанса (текст, картинка значка, картинка плашки,
        /// когда обновлено). Зовётся при каждой перемене и раз в 10 с — расширение прячет значок, если
        /// файл старше 2 минут.
        pub fn write_shell_state(&self) {
            let dir = crate::store::runtime_dir();
            let choice = super::icon_choice();
            let icon = dir.join(format!("tray_cat_{choice}.png"));
            // каждый вид — заново один раз за запуск значка (картинку могли поменять в новой версии)
            static WRITTEN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
            let bit = 1u8 << choice;
            if WRITTEN.fetch_or(bit, std::sync::atomic::Ordering::Relaxed) & bit == 0 {
                if let Some((n, px)) = super::icon_rgba(44) {
                    let rgba: Vec<u8> = px.iter().flat_map(|p| [p[1], p[2], p[3], p[0]]).collect();
                    let _ = crate::store::write_private_quick(&icon, &super::png_rgba(n, n, &rgba));
                }
            }
            let (label, panel) = self.state.lock().map(|s| (s.label.trim().to_string(), s.panel.clone())).unwrap_or_default();
            let mut body = serde_json::json!({ "label": label, "icon": icon.to_string_lossy(), "visible": true, "at": crate::ui::now_secs() });
            if let Some((p, w, h, btn, close)) = panel {
                body["panel"] = serde_json::json!({ "path": p, "w": w, "h": h, "btn": btn, "close": close });
            }
            let _ = crate::store::write_private_quick(&dir.join("tray.json"), body.to_string().as_bytes());
        }

        /// Значок закрывается: расширению — спрятать его сразу.
        pub fn hide_shell(&self) {
            let body = serde_json::json!({ "visible": false, "at": crate::ui::now_secs() });
            let _ = crate::store::write_private_quick(&crate::store::runtime_dir().join("tray.json"), body.to_string().as_bytes());
        }

        /// Кот на значке сменился.
        pub fn refresh_icon(&self) {
            if let Ok(mut s) = self.state.lock() {
                s.icons = pixmaps();
            }
            self.write_shell_state();
            let sni = self.sni.borrow();
            let Some(iface) = sni.as_ref().and_then(|c| c.object_server().interface::<_, Item>(PATH).ok()) else { return };
            let _ = zbus::block_on(Item::new_icon(iface.signal_emitter()));
        }

        pub fn set(&self, label: &str, tip: &str) {
            if let Ok(mut s) = self.state.lock() {
                s.label = label.to_string();
                s.tip = tip.to_string();
            }
            self.write_shell_state();
            let sni = self.sni.borrow();
            let Some(iface) = sni.as_ref().and_then(|c| c.object_server().interface::<_, Item>(PATH).ok()) else { return };
            let e = iface.signal_emitter();
            let _ = zbus::block_on(Item::x_ayatana_new_label(e, label, "\u{2002}100%"));
            let _ = zbus::block_on(Item::new_tool_tip(e));
        }
    }
}

// ───────────────────────────── Windows: значок в области уведомлений ─────────────────────────────

/// Значок с числом для Windows (текст рядом со значком там не показать): цветной квадрат и процент белым.
/// Возвращает ARGB по строкам сверху вниз; None — данных нет (тогда значок кота).
/// Белый значок на тёмно-сером квадрате со скруглёнными углами (для Windows).
#[cfg_attr(not(windows), allow(dead_code))]
fn on_dark(n: i32, px: Vec<[u8; 4]>) -> Vec<[u8; 4]> {
    let corner = |i: usize| {
        let (x, y) = (i as i32 % n, i as i32 / n);
        (x == 0 || x == n - 1) && (y == 0 || y == n - 1)
    };
    px.into_iter()
        .enumerate()
        .map(|(i, p)| {
            if corner(i) {
                return [0, 0, 0, 0];
            }
            // смешать белый поверх 0x505050 по альфе
            let a = p[0] as u32;
            let c = |v: u8| ((v as u32 * a + 0x50 * (255 - a)) / 255) as u8;
            [255, c(p[1]), c(p[2]), c(p[3])]
        })
        .collect()
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn number_icon(fonts: &mut Fonts, n: i32, pct: Option<f64>) -> Option<Vec<[u8; 4]>> {
    let pct = pct?;
    let mut buf = vec![0u32; (n * n) as usize];
    {
        let mut c = Canvas::new(&mut buf, n as usize, n, n);
        c.fill(Rect::new(0, 0, n, n), bar_col(pct));
        let txt = format!("{:.0}", pct.min(100.0));
        // крупнее, но чтобы влезло
        let mut px = (n as f32 * 0.72) as u16;
        while px > 6 && fonts.width(&txt, px, true) > n - 2 {
            px -= 1;
        }
        let tw = fonts.width(&txt, px, true);
        fonts.draw(&mut c, (n - tw) / 2, fonts.center_base(0, n, px), &txt, 0xFFFFFF, px, true);
    }
    // скруглённые углы: по пикселю с каждого угла прозрачный
    let corner = |x: i32, y: i32| (x == 0 || x == n - 1) && (y == 0 || y == n - 1);
    Some(
        buf.iter()
            .enumerate()
            .map(|(i, p)| {
                let (x, y) = (i as i32 % n, i as i32 / n);
                let a = if corner(x, y) { 0 } else { 255 };
                [a, (p >> 16) as u8, (p >> 8) as u8, *p as u8]
            })
            .collect(),
    )
}

#[cfg(windows)]
mod win {
    use super::Ev;
    use std::sync::{Mutex, OnceLock};
    use winit::event_loop::EventLoopProxy;

    type Hwnd = *mut core::ffi::c_void;
    type Hicon = *mut core::ffi::c_void;
    type Hbitmap = *mut core::ffi::c_void;

    #[repr(C)]
    struct NotifyIconData {
        cb_size: u32,
        hwnd: Hwnd,
        id: u32,
        flags: u32,
        callback: u32,
        icon: Hicon,
        tip: [u16; 128],
        state: u32,
        state_mask: u32,
        info: [u16; 256],
        version: u32,
        info_title: [u16; 64],
        info_flags: u32,
        guid: [u8; 16],
        balloon_icon: Hicon,
    }

    #[repr(C)]
    struct WndClass {
        style: u32,
        proc_: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut core::ffi::c_void,
        icon: Hicon,
        cursor: *mut core::ffi::c_void,
        background: *mut core::ffi::c_void,
        menu_name: *const u16,
        class_name: *const u16,
    }

    #[repr(C)]
    struct IconInfo {
        is_icon: i32,
        x_hot: u32,
        y_hot: u32,
        mask: Hbitmap,
        color: Hbitmap,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: [i32; 2],
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassW(c: *const WndClass) -> u16;
        fn CreateWindowExW(
            ex: u32,
            class: *const u16,
            name: *const u16,
            style: u32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            parent: Hwnd,
            menu: *mut core::ffi::c_void,
            inst: *mut core::ffi::c_void,
            param: *mut core::ffi::c_void,
        ) -> Hwnd;
        fn DefWindowProcW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
        fn GetMessageW(m: *mut Msg, h: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
        fn PostMessageW(h: Hwnd, m: u32, w: usize, l: isize) -> i32;
        fn RegisterWindowMessageW(s: *const u16) -> u32;
        fn CreateIconIndirect(i: *const IconInfo) -> Hicon;
        fn DestroyIcon(i: Hicon) -> i32;
        fn GetSystemMetrics(i: i32) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateBitmap(w: i32, h: i32, planes: u32, bits: u32, data: *const core::ffi::c_void) -> Hbitmap;
        fn DeleteObject(o: *mut core::ffi::c_void) -> i32;
    }
    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(msg: u32, data: *mut NotifyIconData) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
    }

    const WM_APP: u32 = 0x8000;
    const CB: u32 = WM_APP + 1;
    const UPDATE: u32 = WM_APP + 2;
    const WM_LBUTTONUP: isize = 0x0202;
    const WM_RBUTTONUP: isize = 0x0205;
    const NIM_ADD: u32 = 0;
    const NIM_MODIFY: u32 = 1;
    const NIF_MESSAGE: u32 = 1;
    const NIF_ICON: u32 = 2;
    const NIF_TIP: u32 = 4;

    /// Что показать: значок (ARGB сверху вниз, сторона n) и подсказка.
    struct Want {
        n: i32,
        argb: Vec<[u8; 4]>,
        tip: String,
    }

    static PROXY: OnceLock<Mutex<EventLoopProxy<Ev>>> = OnceLock::new();
    static WANT: Mutex<Option<Want>> = Mutex::new(None);
    static TASKBAR: OnceLock<u32> = OnceLock::new();

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Окошко-получатель (невидимое, верхнего уровня: только такие получают «панель задач перезапущена»).
    pub struct Tray {
        hwnd: usize,
    }

    /// Сторона значка в области уведомлений (зависит от масштаба экрана).
    pub fn icon_side() -> i32 {
        // SM_CXSMICON
        (unsafe { GetSystemMetrics(49) }).clamp(16, 64)
    }

    fn make_icon(n: i32, argb: &[[u8; 4]]) -> Hicon {
        // BGRA для CreateBitmap: строки сверху вниз
        let bgra: Vec<u8> = argb.iter().flat_map(|p| [p[3], p[2], p[1], p[0]]).collect();
        let mask = vec![0u8; ((n + 15) / 16 * 2 * n) as usize];
        unsafe {
            let color = CreateBitmap(n, n, 1, 32, bgra.as_ptr().cast());
            let m = CreateBitmap(n, n, 1, 1, mask.as_ptr().cast());
            let info = IconInfo { is_icon: 1, x_hot: 0, y_hot: 0, mask: m, color };
            let ic = CreateIconIndirect(&info);
            DeleteObject(color);
            DeleteObject(m);
            ic
        }
    }

    fn data(hwnd: Hwnd) -> NotifyIconData {
        NotifyIconData {
            cb_size: std::mem::size_of::<NotifyIconData>() as u32,
            hwnd,
            id: 1,
            flags: 0,
            callback: CB,
            icon: std::ptr::null_mut(),
            tip: [0; 128],
            state: 0,
            state_mask: 0,
            info: [0; 256],
            version: 0,
            info_title: [0; 64],
            info_flags: 0,
            guid: [0; 16],
            balloon_icon: std::ptr::null_mut(),
        }
    }

    /// Показать то, что лежит в WANT (add — значка ещё нет: первый раз или после перезапуска панели задач).
    fn apply(hwnd: Hwnd, add: bool) {
        static CUR: Mutex<usize> = Mutex::new(0);
        let g = WANT.lock().unwrap_or_else(|e| e.into_inner());
        let Some(w) = g.as_ref() else { return };
        let mut d = data(hwnd);
        d.flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        d.icon = make_icon(w.n, &w.argb);
        for (i, c) in w.tip.encode_utf16().take(127).enumerate() {
            d.tip[i] = c;
        }
        unsafe {
            if Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &mut d) == 0 && !add {
                Shell_NotifyIconW(NIM_ADD, &mut d);
            }
        }
        let mut cur = CUR.lock().unwrap_or_else(|e| e.into_inner());
        if *cur != 0 {
            unsafe { DestroyIcon(*cur as Hicon) };
        }
        *cur = d.icon as usize;
    }

    unsafe extern "system" fn proc_(h: Hwnd, m: u32, w: usize, l: isize) -> isize {
        match m {
            CB if l == WM_LBUTTONUP || l == WM_RBUTTONUP => {
                if let Some(p) = PROXY.get() {
                    let _ = p.lock().unwrap_or_else(|e| e.into_inner()).send_event(Ev::Click(0, 0));
                }
                0
            }
            UPDATE => {
                apply(h, false);
                0
            }
            _ if Some(&m) == TASKBAR.get() => {
                apply(h, true);
                0
            }
            _ => unsafe { DefWindowProcW(h, m, w, l) },
        }
    }

    impl Tray {
        pub fn start(proxy: EventLoopProxy<Ev>) -> Option<Tray> {
            let _ = PROXY.set(Mutex::new(proxy));
            let (tx, rx) = std::sync::mpsc::channel::<usize>();
            std::thread::spawn(move || unsafe {
                let _ = TASKBAR.set(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()));
                let inst = GetModuleHandleW(std::ptr::null());
                let class = wide("AptechkaTray");
                let wc = WndClass {
                    style: 0,
                    proc_,
                    cls_extra: 0,
                    wnd_extra: 0,
                    instance: inst,
                    icon: std::ptr::null_mut(),
                    cursor: std::ptr::null_mut(),
                    background: std::ptr::null_mut(),
                    menu_name: std::ptr::null(),
                    class_name: class.as_ptr(),
                };
                RegisterClassW(&wc);
                let h = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    wide("Aptechka tray").as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    inst,
                    std::ptr::null_mut(),
                );
                let _ = tx.send(h as usize);
                if h.is_null() {
                    return;
                }
                let mut msg: Msg = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            });
            let hwnd = rx.recv().ok().filter(|&h| h != 0)?;
            Some(Tray { hwnd })
        }

        /// Убрать значок сразу (иначе он висит «призраком», пока не наведут мышь).
        pub fn remove(&self) {
            let mut d = data(self.hwnd as Hwnd);
            unsafe { Shell_NotifyIconW(2, &mut d) };
        }

        pub fn set(&self, n: i32, argb: Vec<[u8; 4]>, tip: &str) {
            let first = WANT.lock().unwrap_or_else(|e| e.into_inner()).replace(Want { n, argb, tip: tip.to_string() }).is_none();
            if first {
                // первый показ — из потока окошка (Shell_NotifyIcon привязан к его окну, не к потоку, но так проще)
                unsafe { PostMessageW(self.hwnd as Hwnd, *TASKBAR.get().unwrap_or(&0), 0, 0) };
            } else {
                unsafe { PostMessageW(self.hwnd as Hwnd, UPDATE, 0, 0) };
            }
        }
    }
}

// ───────────────────────────── включение и автозапуск ─────────────────────────────

/// Значок включён в настройках (по умолчанию — да).
pub fn enabled() -> bool {
    crate::prefs::load().get("tray").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// Значок есть пока на Linux и Windows (на macOS — следующий шаг).
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", windows));

/// Есть ли о чём показывать: стоит Claude Code или Codex.
fn has_agents() -> bool {
    crate::limits::claude_dir().is_some() || crate::agents::find("codex").and_then(|a| a.data_dir()).is_some()
}

/// Путь к программе для автозапуска; None — сборка из исходников (папка target/: путь временный).
fn exe() -> Option<std::path::PathBuf> {
    let p = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from).or_else(|| std::env::current_exe().ok())?;
    let s = p.to_string_lossy().replace('\\', "/");
    (!(s.contains("/target/release/") || s.contains("/target/debug/") || s.contains("/target/x86_64-"))).then_some(p)
}

#[cfg(target_os = "linux")]
fn autostart_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| crate::store::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("autostart").join("io.github.aptechka-tray.desktop"))
}

/// Наше расширение GNOME: значок в панели, который показывает плашку при наведении (через AppIndicator
/// GNOME зовёт программу только двойным щелчком). Кладём его в папку расширений и включаем; выключили
/// значок — выключаем и расширение. Новое расширение GNOME подхватит после перезапуска оболочки или входа.
#[cfg(target_os = "linux")]
fn gnome_extension(on: bool) {
    const UUID: &str = "aptechka-limits@aptechka.github.io";
    const META: &str = include_str!("../../packaging/gnome/aptechka-limits@aptechka.github.io/metadata.json");
    const JS: &str = include_str!("../../packaging/gnome/aptechka-limits@aptechka.github.io/extension.js");
    let gnome = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_uppercase().contains("GNOME"));
    if !gnome || crate::procs::which("gnome-shell").is_none() {
        return;
    }
    let Some(home) = crate::store::home_dir() else { return };
    let dir = home.join(".local/share/gnome-shell/extensions").join(UUID);
    let get = || crate::diag::run(&["gsettings", "get", "org.gnome.shell", "enabled-extensions"], 5).1;
    let list = get();
    let enabled = list.contains(UUID);
    if on {
        for (name, body) in [("metadata.json", META), ("extension.js", JS)] {
            let p = dir.join(name);
            if crate::store::read_capped(&p, 1 << 20).ok().as_deref() != Some(body) {
                let _ = std::fs::create_dir_all(&dir);
                let _ = crate::store::write_mode(&p, body.as_bytes(), 0o644);
            }
        }
        if !enabled {
            // в список включённых (GNOME прочтёт его при перезапуске оболочки), и попробовать включить сразу
            let items: Vec<String> = list
                .trim()
                .trim_start_matches("@as")
                .trim()
                .trim_matches(['[', ']'])
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
            let mut items = items;
            items.push(format!("'{UUID}'"));
            let v = format!("[{}]", items.join(", "));
            let _ = crate::diag::run(&["gsettings", "set", "org.gnome.shell", "enabled-extensions", &v], 5);
            let _ = crate::diag::run(&["gnome-extensions", "enable", UUID], 5);
            crate::diag::log("значок в панели: расширение GNOME установлено (заработает после перезапуска оболочки или входа)");
        }
    } else if enabled {
        let _ = crate::diag::run(&["gnome-extensions", "disable", UUID], 5);
    }
}

/// Записать или убрать автозапуск значка при входе в систему.
fn set_autostart(on: bool) {
    #[cfg(target_os = "linux")]
    {
        let Some(f) = autostart_file() else { return };
        if !on {
            crate::store::remove_file_nofollow(&f);
            return;
        }
        let Some(exe) = exe() else { return };
        let q = exe.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        let body = format!(
            "[Desktop Entry]\nType=Application\nName={}\nExec=\"{q}\" --tray\nIcon=io.github.aptechka\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\nX-GNOME-Autostart-Delay=8\n",
            crate::i18n::t("Аптечка — лимиты ИИ")
        );
        if crate::store::read_capped(&f, 4096).ok().as_deref() != Some(body.as_str()) {
            if let Some(d) = f.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = crate::store::write_mode(&f, body.as_bytes(), 0o644);
        }
    }
    #[cfg(windows)]
    {
        const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        if !on {
            let _ = crate::diag::run(&["reg", "delete", KEY, "/v", "AptechkaTray", "/f"], 10);
            return;
        }
        let Some(exe) = exe() else { return };
        let val = format!("\"{}\" --tray", exe.display());
        let _ = crate::diag::run(&["reg", "add", KEY, "/v", "AptechkaTray", "/t", "REG_SZ", "/d", &val, "/f"], 10);
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = on;
}

/// Привести значок в соответствие с настройкой: включён — автозапуск на месте и значок запущен;
/// выключен — автозапуск убран (запущенный значок закроется сам, см. `run`).
/// Зовётся при запуске программы (в фоне) и после переключения в меню.
pub fn ensure() {
    if !SUPPORTED {
        return;
    }
    let on = enabled() && has_agents();
    set_autostart(on);
    #[cfg(target_os = "linux")]
    gnome_extension(on);
    if on && crate::store::lock::free(&crate::store::runtime_dir().join("tray.lock")) {
        if let Some(exe) = exe() {
            crate::procs::spawn_detached(std::process::Command::new(exe).arg("--tray"));
        }
    }
}

// ───────────────────────────── уведомления и прогноз ─────────────────────────────

/// Одно окно лимита (агент, 0 — 5 ч, 1 — неделя): что видели и когда.
#[derive(Default)]
struct WinState {
    resets_at: i64,
    pct: f64,
    /// о каком пороге уже сказали в этом окне (0, 80, 95)
    alerted: u8,
    /// (когда, %) за последний час — для темпа
    samples: std::collections::VecDeque<(i64, f64)>,
}

/// Слежение за лимитами: уведомления (80 %, 95 %, «снова свободен») и прогноз «когда кончится».
#[derive(Default)]
pub struct Watch {
    w: std::collections::HashMap<(String, u8), WinState>,
}

fn level(p: f64) -> u8 {
    if p >= 95.0 {
        95
    } else if p >= 80.0 {
        80
    } else {
        0
    }
}

fn win_name(which: u8) -> &'static str {
    if which == 0 {
        "на 5 ч"
    } else {
        "на неделю"
    }
}

impl Watch {
    /// Новые цифры. Возвращает уведомления (заголовок, текст). Первое, что увидели, — только запоминаем
    /// (при входе в систему не сыпать уведомлениями о том, что было и раньше).
    pub fn update(&mut self, agents: &[Agent], now: i64) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for a in agents {
            let Some(l) = a.limits else { continue };
            for (which, win) in [(0u8, l.five), (1u8, l.week)] {
                let Some(win) = win else { continue };
                let pct = win.now_pct(now);
                let at = if l.at > 0 { l.at } else { now };
                let key = (a.name.to_string(), which);
                let Some(st) = self.w.get_mut(&key) else {
                    let mut st = WinState { resets_at: win.resets_at, pct, alerted: level(pct), ..Default::default() };
                    st.samples.push_back((at, pct));
                    self.w.insert(key, st);
                    continue;
                };
                // окно сбросилось: процент заметно упал (новое время сброса или старое уже прошло)
                if pct + 5.0 < st.pct && (win.resets_at != st.resets_at || (st.resets_at > 0 && now >= st.resets_at)) {
                    if st.pct >= 80.0 {
                        out.push((format!("{}: лимит {} снова свободен", a.name, win_name(which)), "Можно работать дальше.".to_string()));
                    }
                    st.alerted = 0;
                    st.samples.clear();
                }
                st.resets_at = win.resets_at;
                let lv = level(pct);
                if lv > st.alerted {
                    let reset = if win.resets_at > now { format!("Сброс {}.", crate::limits::when(win.resets_at, now)) } else { String::new() };
                    out.push((format!("{}: занято {:.0}% лимита {}", a.name, pct, win_name(which)), reset));
                }
                st.alerted = st.alerted.max(lv);
                st.pct = pct;
                if st.samples.back().is_none_or(|s| s.0 != at) {
                    st.samples.push_back((at, pct));
                }
                while st.samples.front().is_some_and(|s| at - s.0 > 3600) {
                    st.samples.pop_front();
                }
            }
        }
        out
    }

    /// Когда кончится лимит при нынешнем темпе (за последний час, не меньше 10 минут наблюдений);
    /// None — темп не известен или до сброса хватит.
    pub fn runs_out(&self, name: &str, which: u8) -> Option<i64> {
        let st = self.w.get(&(name.to_string(), which))?;
        let (&(t0, p0), &(t1, p1)) = (st.samples.front()?, st.samples.back()?);
        if t1 - t0 < 600 || p1 <= p0 || p1 >= 100.0 {
            return None;
        }
        let rate = (p1 - p0) / (t1 - t0) as f64;
        let eta = t1 + ((100.0 - p1) / rate) as i64;
        (st.resets_at == 0 || eta < st.resets_at).then_some(eta)
    }
}

// ───────────────────────────── окошко по нажатию ─────────────────────────────

// ───────────────────────────── плашка по дизайну (вариант 2) ─────────────────────────────
//
// Плашка — это дизайн ChatGPT, порезанный на куски (tools/art_gpt/panel_template.py): «шапка» (заголовок),
// «группа» (рамка одного агента, без данных) и «низ» (кнопка, место под сценку). Программа складывает
// куски и рисует поверх только то, что меняется, — в тех же местах, теми же цветами, что на дизайне.
// Куски уменьшаются под экран один раз и запоминаются (картинка готова заранее, мощности не тратит).

/// Масштаб: дизайн — окно шириной 888 точек; на экране ×2 плашка — 0,84 от него.
fn tpl_scale(k: i32) -> f32 {
    0.42 * k as f32
}

/// Картинка: ширина, высота, точки 0xRRGGBB.
type Img = (i32, i32, Vec<u32>);

fn decode(bytes: &[u8]) -> Img {
    match crate::gfx::decode_png(bytes) {
        Some(pm) => {
            (pm.width() as i32, pm.height() as i32, pm.data().chunks(4).map(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32).collect())
        }
        None => (1, 1, vec![0xD7D4D1]),
    }
}

/// Уменьшение ближайшей точкой — для пиксельных сценок: каждая точка рисунка остаётся цельной (усреднение
/// размывало контуры в «мыло»).
fn resample_nearest(src: &Img, s: f32) -> Img {
    let (sw, sh, px) = src;
    let (dw, dh) = (((*sw as f32) * s).round().max(1.0) as i32, ((*sh as f32) * s).round().max(1.0) as i32);
    let mut out = Vec::with_capacity((dw * dh) as usize);
    for y in 0..dh {
        let sy = ((y as f32 + 0.5) / s) as i32;
        for x in 0..dw {
            let sx = ((x as f32 + 0.5) / s) as i32;
            out.push(px[(sy.min(sh - 1) * sw + sx.min(sw - 1)) as usize]);
        }
    }
    (dw, dh, out)
}

/// Куски плашки под один масштаб экрана: аватарка (из значка программы) и пиксельные сценки — уменьшены
/// один раз. Всё остальное (рамка, заголовок, группы, кнопка) рисует код по меркам дизайна.
pub struct Tpl {
    k: i32,
    s: f32,
    /// аватарка в заголовке: (ширина, высота, ARGB с прозрачностью)
    avatar: Img,
    /// сценки: 0 — агенты закрыты (спит), 1 — работают (пишет луком), 2 — закончили
    scenes: [Img; 3],
}

impl Tpl {
    fn new(k: i32) -> Tpl {
        let s = tpl_scale(k);
        let d = |v: i32| (v as f32 * s).round() as i32;
        // сценка — настоящий пиксель-арт 367×103: увеличение ближайшей точкой до места на плашке
        let scene = |b: &[u8]| {
            let src = decode(b);
            resample_nearest(&src, d(876) as f32 / src.0 as f32)
        };
        Tpl {
            k,
            s,
            avatar: avatar(d(52)),
            scenes: [
                scene(include_bytes!("../assets/сценка_спит.png")),
                scene(include_bytes!("../assets/сценка_работает.png")),
                scene(include_bytes!("../assets/сценка_закончил.png")),
            ],
        }
    }

    /// Точка дизайна → точка на экране.
    fn d(&self, v: i32) -> i32 {
        (v as f32 * self.s).round() as i32
    }

    fn font(&self, v: i32) -> u16 {
        (v as f32 * self.s).round().max(6.0) as u16
    }

    /// Высоты частей (по дизайну: заголовок 98, группа агента 512, низ 349 точек).
    fn parts(&self) -> (i32, i32, i32) {
        (self.d(98), self.d(512), self.d(349))
    }

    fn size(&self, agents: usize) -> (i32, i32) {
        let (h, g, f) = self.parts();
        (self.d(888), h + g * agents.max(1) as i32 + f)
    }
}

/// Аватарка: значок программы (он и так внутри программы), уменьшенный средним, с прозрачностью.
fn avatar(side: i32) -> Img {
    let Some(pm) = crate::gfx::decode_png(include_bytes!("../../assets/aptechka.png")) else { return (1, 1, vec![0]) };
    let (sw, sh, d) = (pm.width() as i32, pm.height() as i32, pm.data());
    let n = side.max(1);
    let mut out = Vec::with_capacity((n * n) as usize);
    for y in 0..n {
        for x in 0..n {
            let (x0, x1) = (x * sw / n, ((x + 1) * sw / n).max(x * sw / n + 1));
            let (y0, y1) = (y * sh / n, ((y + 1) * sh / n).max(y * sh / n + 1));
            let mut acc = [0u32; 4];
            let mut cnt = 0;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let k = ((yy * sw + xx) * 4) as usize;
                    for (i, a) in acc.iter_mut().enumerate() {
                        *a += d[k + i] as u32;
                    }
                    cnt += 1;
                }
            }
            let c = |v: u32| v / cnt.max(1);
            // предумноженный RGBA → 0xAARRGGBB
            out.push((c(acc[3]) << 24) | (c(acc[0]) << 16) | (c(acc[1]) << 8) | c(acc[2]));
        }
    }
    (n, n, out)
}

fn blit(c: &mut Canvas, img: &Img, x: i32, y: i32) {
    let (w, h, px) = img;
    for yy in 0..*h {
        for xx in 0..*w {
            c.blend(x + xx, y + yy, px[(yy * w + xx) as usize], 255);
        }
    }
}

/// С прозрачностью (0xAARRGGBB, цвет предумножен на альфу).
fn blit_alpha(c: &mut Canvas, img: &Img, x: i32, y: i32) {
    let (w, h, px) = img;
    for yy in 0..*h {
        for xx in 0..*w {
            let p = px[(yy * w + xx) as usize];
            let a = p >> 24;
            if a == 0 {
                continue;
            }
            // обратно из предумноженного
            let un = |v: u32| (v * 255 / a).min(255);
            let col = (un(p >> 16 & 255) << 16) | (un(p >> 8 & 255) << 8) | un(p & 255);
            c.blend(x + xx, y + yy, col, a);
        }
    }
}

/// Вертикальный градиент по точкам (0..1, цвет).
fn gradient_v(c: &mut Canvas, r: Rect, stops: &[(f32, u32)]) {
    let lerp = |a: u32, b: u32, t: f32| {
        let ch = |s: u32| {
            let (x, y) = ((a >> s & 255) as f32, (b >> s & 255) as f32);
            ((x + (y - x) * t).round() as u32) << s
        };
        ch(16) | ch(8) | ch(0)
    };
    for y in 0..r.h {
        let t = y as f32 / (r.h - 1).max(1) as f32;
        let i = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0);
        let (a, b) = (stops[i], stops[(i + 1).min(stops.len() - 1)]);
        let u = if b.0 > a.0 { (t - a.0) / (b.0 - a.0) } else { 0.0 };
        c.fill(Rect::new(r.x, r.y + y, r.w, 1), lerp(a.1, b.1, u));
    }
}

/// Рамка в одну точку экрана.
fn frame(c: &mut Canvas, r: Rect, col: u32) {
    c.fill(Rect::new(r.x, r.y, r.w, 1), col);
    c.fill(Rect::new(r.x, r.bottom() - 1, r.w, 1), col);
    c.fill(Rect::new(r.x, r.y, 1, r.h), col);
    c.fill(Rect::new(r.right() - 1, r.y, 1, r.h), col);
}

use crate::gfx::{BLACK, WHITE};

const W: i32 = 444;
const BODY: u32 = 0xD7D4D1;
const INK: u32 = 0x151515;
const GREY: u32 = 0x5E5D5C;
const TRACK: u32 = 0xC2BDB8;

/// цвет агента (как на дизайне: Claude — оранжевый, Codex — синий)
fn agent_col(name: &str) -> u32 {
    if name.starts_with("Claude") {
        0xF45A26
    } else {
        0x0F6CF2
    }
}

/// Цвет состояния лимита (всегда вместе с числом — не один цвет): хорошо, внимание, почти всё.
fn bar_col(p: f64) -> u32 {
    if p >= 90.0 {
        0xD03B3B
    } else if p >= 70.0 {
        0xEDA100
    } else {
        0x04B001
    }
}

/// Окно плашки по дизайну (вариант 2): рамка XP, заголовок с градиентом, аватаркой и «×».
fn paint_head(f: &mut Fonts, t: &Tpl, c: &mut Canvas, w: i32, h: i32) -> Rect {
    let d = |v: i32| t.d(v);
    c.fill(Rect::new(0, 0, w, h), BODY);
    // рамка окна: тёмно-синяя, голубая, светлая
    for (i, col) in [0x071EBB, 0x1798FC, 0x8FCBFD].into_iter().enumerate() {
        frame(c, Rect::new(i as i32, i as i32, w - 2 * i as i32, h - 2 * i as i32), col);
    }
    let title = Rect::new(3, 3, w - 6, d(66));
    gradient_v(c, title, &[(0.0, 0x62B6FE), (0.07, 0x0275F9), (0.35, 0x0046D4), (0.85, 0x004EDA), (1.0, 0x065EDA)]);
    blit_alpha(c, &t.avatar, d(21), d(10));
    let base = f.center_base(title.y, title.h, t.font(34));
    f.draw(c, d(98) + 1, base + 1, "Лимиты ИИ", 0x002A8C, t.font(34), true);
    f.draw(c, d(98), base, "Лимиты ИИ", WHITE, t.font(34), true);
    let close = Rect::new(d(821), d(15), d(52), d(48));
    c.fill(close, 0x2C2C2C);
    c.raised(close.inset(1), 0xD6D3CE);
    let xw = f.width("×", t.font(36), true);
    f.draw(c, close.x + (close.w - xw) / 2, f.center_base(close.y, close.h, t.font(36)), "×", BLACK, t.font(36), true);
    close
}

/// Рамка-группа агента (как на дизайне): вдавленная рамка группы, под подписью — вдавленная панель.
fn paint_group(c: &mut Canvas, t: &Tpl, gy: i32) {
    let d = |v: i32| t.d(v);
    let outer = Rect::new(d(28), gy + d(5), d(835), d(495));
    frame(c, outer, 0x8E9196);
    frame(c, outer.inset(1), WHITE);
    let panel = Rect::new(d(40), gy + d(54), d(811), d(436));
    c.fill(Rect::new(panel.x, panel.y, panel.w, 1), 0x999C9B);
    c.fill(Rect::new(panel.x, panel.y + 1, panel.w, 1), 0x888A8F);
    c.fill(Rect::new(panel.x, panel.y, 1, panel.h), 0x999C9B);
    c.fill(Rect::new(panel.x + 1, panel.y, 1, panel.h), 0x888A8F);
    c.fill(Rect::new(panel.x, panel.bottom() - 1, panel.w, 1), WHITE);
    c.fill(Rect::new(panel.right() - 1, panel.y, 1, panel.h), WHITE);
}

/// Кнопка XP в одну тёмную точку по краю.
fn paint_button(f: &mut Fonts, t: &Tpl, c: &mut Canvas, r: Rect, label: &str) {
    c.fill(r, 0x3B3B3B);
    c.raised(r.inset(1), 0xD6D3CE);
    let tw = f.width(label, t.font(26), false);
    f.draw(c, r.x + (r.w - tw) / 2, f.center_base(r.y, r.h, t.font(26)), label, BLACK, t.font(26), false);
}

/// Нарисовать плашку (окно — кодом, сценка — пиксель-арт); возвращает места кнопок «Медкарта расхода» и «×».
fn paint(f: &mut Fonts, agents: &[Agent], watch: &Watch, data_t: i64, t: &Tpl, c: &mut Canvas) -> (Rect, Rect) {
    let now = crate::ui::now_secs();
    let d = |v: i32| t.d(v);
    let (w, h) = t.size(agents.len());
    let close = paint_head(f, t, c, w, h);
    let (hh, gh, _) = t.parts();
    let n = agents.len().max(1);
    for i in 0..n {
        let gy = hh + gh * i as i32;
        paint_group(c, t, gy);
        let Some(a) = agents.get(i) else {
            f.draw(c, d(60), gy + d(250), "Пока нет данных: ни Claude Code, ни Codex ещё не отвечали.", GREY, t.font(24), false);
            continue;
        };
        // заголовок группы: значок и название — по центру полосы между верхней рамкой и панелью
        let (band_y, band_h) = (gy + d(7), d(47));
        let mark = d(26);
        c.fill(Rect::new(d(55), band_y + (band_h - mark) / 2, mark, mark), agent_col(a.name));
        f.draw(c, d(55) + mark + d(15), f.center_base(band_y, band_h, t.font(30)), a.name, INK, t.font(30), true);
        // кольца лимитов: белый ободок, дорожка, дуга, белая серединка с процентом; подпись и сброс — под ним
        if let Some(lim) = a.limits.filter(|_| has_rings(a)) {
            let wins: Vec<(u8, &str, crate::limits::Window)> =
                [(0u8, "5 часов", lim.five), (1u8, "неделя", lim.week)].into_iter().filter_map(|(w, s, x)| x.map(|x| (w, s, x))).collect();
            let centers: &[i32] = if wins.len() == 2 { &[276, 609] } else { &[444] };
            for ((which, what, win), cx) in wins.iter().zip(centers) {
                let pct = win.now_pct(now);
                let (cx, cy) = (d(*cx), gy + d(167));
                crate::gfx::ring(c, cx, cy, d(90) as f32, d(87) as f32, 1.0, 0xFCFBFC, 0xFCFBFC);
                crate::gfx::ring(c, cx, cy, d(87) as f32, d(68) as f32, pct / 100.0, bar_col(pct), TRACK);
                crate::gfx::ring(c, cx, cy, d(68) as f32, 0.0, 1.0, 0xF4F3F4, 0xF4F3F4);
                let p = format!("{pct:.0}%");
                let pw = f.width(&p, t.font(37), true);
                f.draw(c, cx - pw / 2, cy + d(13), &p, INK, t.font(37), true);
                let lw = f.width(what, t.font(29), false);
                f.draw(c, cx - lw / 2, gy + d(285), what, INK, t.font(29), false);
                let (line, col) = match watch.runs_out(a.name, *which) {
                    Some(eta) => (format!("кончится ≈ {}", crate::limits::when(eta, now)), 0xB3001B),
                    None if win.resets_at > now => (format!("сброс {}", crate::limits::when(win.resets_at, now)), GREY),
                    None if win.resets_at > 0 => ("уже сброшен".to_string(), GREY),
                    None => (String::new(), GREY),
                };
                let rw = f.width(&line, t.font(23), false);
                f.draw(c, cx - rw / 2, gy + d(322), &line, col, t.font(23), false);
            }
        }
        // столбики токенов по дням
        let days = a.days.len().max(1) as i32;
        let (x0, x1, base, maxh) = (d(58), d(830), gy + d(442), d(100));
        let slot = (x1 - x0) as f32 / days as f32;
        let max = a.days.iter().copied().max().unwrap_or(0).max(1);
        for (j, v) in a.days.iter().enumerate() {
            let hh = ((*v as f64 / max as f64) * maxh as f64).round() as i32;
            if hh > 0 {
                let bx = x0 + (j as f32 * slot).round() as i32;
                let bw = ((j + 1) as f32 * slot).round() as i32 - (j as f32 * slot).round() as i32 - d(3).max(1);
                c.fill(Rect::new(bx, base - hh, bw.max(1), hh), agent_col(a.name));
            }
        }
        let lh = d(1).max(1);
        c.fill(Rect::new(x0, base, x1 - x0, lh), 0x8F979E);
        c.fill(Rect::new(x0, base + lh, x1 - x0, lh), 0x808B91);
        c.fill(Rect::new(x0, base + 2 * lh, x1 - x0, lh), 0xFEFDFC);
        let first = crate::ui::localtime(now - (crate::limits::BAR_DAYS - 1) * 86_400);
        f.draw(c, d(61), gy + d(474), &format!("{:02}.{:02}", first.mday, first.mon), GREY, t.font(22), false);
        let tl = "сегодня";
        let tw = f.width(tl, t.font(22), false);
        f.draw(c, d(833) - tw, gy + d(474), tl, GREY, t.font(22), false);
    }
    // низ: сценка с котом (что делают агенты), «обновлено в…», кнопка «Медкарта расхода…» (она — в шаблоне)
    let fy = hh + gh * n as i32;
    blit(c, &t.scenes[(icon_choice() as usize).min(2)], d(11), fy + d(92));
    if data_t > 0 {
        let tm = crate::ui::localtime(data_t);
        f.draw(c, d(36), fy + d(55), &format!("обновлено в {:02}:{:02}", tm.hour, tm.min), GREY, t.font(22), false);
    }
    let btn = Rect::new(d(458), fy + d(15), d(405), d(62));
    paint_button(f, t, c, btn, "Медкарта расхода…");
    (btn, close)
}

fn has_rings(a: &Agent) -> bool {
    a.limits.is_some_and(|l| l.five.is_some() || l.week.is_some())
}

struct Popup {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    /// кнопка «Медкарта расхода» и «×» (в пикселях окна)
    btn: Rect,
    close: Rect,
    cursor: (i32, i32),
    /// когда открыли (второй щелчок двойного нажатия не закрывает)
    opened: Instant,
}

struct App {
    proxy: EventLoopProxy<Ev>,
    /// все агенты из последней сводки; `agents` — только те, за которыми следим
    all: Vec<Agent>,
    agents: Vec<Agent>,
    #[cfg(target_os = "linux")]
    tray: Option<sni::Tray>,
    #[cfg(windows)]
    tray: Option<win::Tray>,
    popup: Option<Popup>,
    fonts: Fonts,
    k: i32,
    #[cfg(target_os = "linux")]
    reg_t: Instant,
    watch: Watch,
    /// шаблон плашки под масштаб экрана
    tpl: Option<Tpl>,
    /// счётчик картинок плашки (меню расширения GNOME)
    #[cfg(target_os = "linux")]
    panel_n: u32,
    /// когда пришли последние цифры
    data_t: i64,
}

/// Сколько плашка не закрывается после открытия (потеря фокуса от того же щелчка, второй щелчок двойного нажатия).
const GRACE: Duration = Duration::from_millis(900);

impl App {
    /// Шаблон плашки под нынешний масштаб экрана (уменьшается один раз) и размер плашки под агентов.
    fn tpl_size(&mut self) -> (i32, i32) {
        if self.tpl.as_ref().is_none_or(|t| t.k != self.k) {
            self.tpl = Some(Tpl::new(self.k.max(1)));
        }
        self.tpl.as_ref().map_or((W, W), |t| t.size(self.agents.len()))
    }

    fn draw(&mut self) {
        let Some(p) = self.popup.as_mut() else { return };
        let size = p.window.inner_size();
        let (w, h) = (size.width as i32, size.height as i32);
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w as u32), NonZeroU32::new(h as u32)) else { return };
        if p.surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = p.surface.buffer_mut() else { return };
        let (btn, close) = {
            let mut c = Canvas::new(&mut buf, w as usize, w, h);
            paint(&mut self.fonts, &self.agents, &self.watch, self.data_t, self.tpl.as_ref().expect("шаблон готов"), &mut c)
        };
        p.btn = btn;
        p.close = close;
        let _ = buf.present();
    }
}

impl App {
    fn open_popup(&mut self, el: &ActiveEventLoop, cx: i32, cy: i32) {
        crate::diag::log("значок: нажатие");
        if let Some(p) = self.popup.take() {
            // второе нажатие — закрыть; но не второй щелчок двойного нажатия
            if p.opened.elapsed() < GRACE {
                self.popup = Some(p);
            } else {
                crate::diag::log("плашка: закрыта повторным нажатием");
            }
            return;
        }
        let k = el.primary_monitor().map(|m| m.scale_factor().round() as i32).unwrap_or(1).clamp(1, 4);
        self.k = k;
        let (w, h) = self.tpl_size();
        // правый верхний угол основного экрана, под панелью: там и значок
        // Linux и macOS — правый верхний угол под панелью; Windows — правый нижний над панелью задач
        let pos = el.primary_monitor().map(|m| {
            let (mp, ms) = (m.position(), m.size());
            let y = if cfg!(windows) { mp.y + ms.height as i32 - h - 56 * k } else { mp.y + 36 * k };
            // панель сказала, где нажали, — плашка под значком (в пределах экрана)
            let right = mp.x + ms.width as i32 - w - 8 * k;
            let x = if cx > 0 { (cx - w / 2).clamp(mp.x + 8 * k, right) } else { right };
            let y = if cy > 0 && !cfg!(windows) { y.max(cy + 12 * k) } else { y };
            PhysicalPosition::new(x, y)
        });
        let mut attrs = Window::default_attributes()
            .with_title(crate::i18n::t("Лимиты ИИ"))
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_inner_size(PhysicalSize::new(w as u32, h as u32));
        if let Some(p) = pos {
            attrs = attrs.with_position(p);
        }
        #[cfg(target_os = "linux")]
        let attrs = {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};
            attrs.with_x11_window_type(vec![WindowType::PopupMenu]).with_name("io.github.aptechka", "aptechka-tray")
        };
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_skip_taskbar(true)
        };
        let Ok(window) = el.create_window(attrs) else { return };
        let window = Rc::new(window);
        let Ok(ctx) = softbuffer::Context::new(window.clone()) else { return };
        let Ok(surface) = softbuffer::Surface::new(&ctx, window.clone()) else { return };
        window.focus_window();
        self.popup =
            Some(Popup { window, surface, btn: Rect::new(0, 0, 0, 0), close: Rect::new(0, 0, 0, 0), cursor: (0, 0), opened: Instant::now() });
        self.draw();
    }

    /// Пришли новые цифры: отфильтровать по выбору, обновить значок и открытую плашку.
    fn user_event_data(&mut self, a: Vec<Agent>) {
        self.data_t = crate::ui::now_secs();
        self.all = a;
        let a: Vec<Agent> = self.all.iter().filter(|x| shown(x.name)).cloned().collect();
        for (title, body) in self.watch.update(&a, self.data_t) {
            if notify_on() {
                crate::notify::send(crate::i18n::t(&title), crate::i18n::t(&body));
            }
        }
        let changed = a != self.agents;
        self.agents = a;
        if changed {
            self.set_tray();
            let (w, h) = self.tpl_size();
            if let Some(p) = &self.popup {
                // высота могла поменяться (появился агент)
                let _ = p.window.request_inner_size(PhysicalSize::new(w as u32, h as u32));
                p.window.request_redraw();
            }
        }
    }

    /// Поменяли, за какими агентами следить: пересобрать показанное из последней сводки.
    fn apply_shown(&mut self) {
        let all = std::mem::take(&mut self.all);
        let t = self.data_t;
        self.user_event_data(all);
        self.data_t = t;
        // значок — в любом случае (агентов могло не остаться вовсе)
        self.set_tray();
        let (w, h) = self.tpl_size();
        if let Some(p) = &self.popup {
            let _ = p.window.request_inner_size(PhysicalSize::new(w as u32, h as u32));
            p.window.request_redraw();
        }
    }

    /// Плашка картинкой для меню нашего расширения GNOME (оно показывает её мгновенно, как родное меню
    /// панели; окно-плашка остаётся для KDE и Windows).
    #[cfg(target_os = "linux")]
    fn render_panel(&mut self) {
        if !self.tray.as_ref().is_some_and(|t| t.shell_icon()) {
            return;
        }
        let (w, h) = self.tpl_size();
        let mut buf = vec![0u32; (w * h) as usize];
        let (btn, close) = {
            let mut c = Canvas::new(&mut buf, w as usize, w, h);
            paint(&mut self.fonts, &self.agents, &self.watch, self.data_t, self.tpl.as_ref().expect("шаблон готов"), &mut c)
        };
        let rgba: Vec<u8> = buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, 255]).collect();
        // GNOME запоминает картинку по имени файла (повтор имени = старая картинка), поэтому имя каждый раз
        // новое: номер процесса и счётчик; прошлые картинки удаляем
        self.panel_n = self.panel_n.wrapping_add(1);
        let dir = crate::store::runtime_dir();
        let name = format!("tray_panel_{}_{}.png", std::process::id(), self.panel_n);
        let path = dir.join(&name);
        let _ = crate::store::write_private_quick(&path, &png_rgba(w as u32, h as u32, &rgba));
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let f = e.file_name().to_string_lossy().into_owned();
                if f.starts_with("tray_panel_") && f.ends_with(".png") && f != name {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let r = |x: Rect| [x.x, x.y, x.w, x.h];
        let Some(t) = &self.tray else { return };
        t.set_panel(path.to_string_lossy().into_owned(), w as u32, h as u32, r(btn), r(close));
    }

    fn set_tray(&mut self) {
        #[cfg(target_os = "linux")]
        self.render_panel();
        let now = crate::ui::now_secs();
        #[cfg(target_os = "linux")]
        if let Some(t) = &self.tray {
            t.set(&label(&self.agents, now), &tooltip(&self.agents, now));
        }
        #[cfg(windows)]
        if let Some(t) = &self.tray {
            let n = win::icon_side();
            let argb = number_icon(&mut self.fonts, n, crate::limits::top(&self.agents, now))
                // данных нет — кот; белый силуэт на тёмном квадрате (на светлой панели задач иначе не виден)
                .or_else(|| icon_rgba(n as u32).map(|(_, px)| on_dark(n, px)))
                .unwrap_or_default();
            t.set(n, argb, &crate::i18n::t(&tooltip(&self.agents, now)));
        }
        let _ = now;
    }
}

impl ApplicationHandler<Ev> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        // масштаб экрана — для картинки плашки (меню расширения GNOME рисует её без окна)
        self.k = el.primary_monitor().map(|m| m.scale_factor().round() as i32).unwrap_or(1).clamp(1, 4);
        self.set_tray();
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: Ev) {
        match ev {
            Ev::Data(a) => self.user_event_data(a),
            Ev::Click(x, y) => self.open_popup(el, x, y),
            Ev::Show(x, y) => {
                if self.popup.is_none() {
                    self.open_popup(el, x, y);
                }
            }
            Ev::Shown => self.apply_shown(),
            Ev::Icon => {
                #[cfg(target_os = "linux")]
                if let Some(t) = &self.tray {
                    t.refresh_icon();
                }
                // на Windows значок рисуется заново вместе с числом
                self.set_tray();
            }
            Ev::Update => {
                #[cfg(windows)]
                if let Some(t) = &self.tray {
                    t.remove();
                }
                if let Some(exe) = crate::procs::self_exe() {
                    let mut c = std::process::Command::new(exe);
                    c.arg("--tray").env("APTECHKA_REPLACES", std::process::id().to_string());
                    crate::procs::spawn_detached(&mut c);
                }
                el.exit()
            }
            Ev::Quit => {
                #[cfg(windows)]
                if let Some(t) = &self.tray {
                    t.remove();
                }
                #[cfg(target_os = "linux")]
                if let Some(t) = &self.tray {
                    t.hide_shell();
                }
                el.exit()
            }
        }
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CloseRequested => self.popup = None,

            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && event.logical_key == Key::Named(NamedKey::Escape) => {
                self.popup = None
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(p) = self.popup.as_mut() {
                    p.cursor = (position.x as i32, position.y as i32);
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                if let Some(p) = self.popup.take() {
                    if p.btn.contains(p.cursor.0, p.cursor.1) {
                        open_analytics();
                    } else if !p.close.contains(p.cursor.0, p.cursor.1) {
                        self.popup = Some(p);
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // панель могла перезапуститься — напоминаем о значке раз в минуту
        #[cfg(target_os = "linux")]
        // раз в 10 с: наше расширение GNOME появилось или пропало — значок через AppIndicator убрать или вернуть
        // (и панель могла перезапуститься); файл для расширения — свежий
        if self.reg_t.elapsed() > Duration::from_secs(10) {
            self.reg_t = Instant::now();
            if let Some(t) = &self.tray {
                t.register();
                t.write_shell_state();
            }
        }
        let wait = Duration::from_secs(10);
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + wait));
    }
}

/// Открыть Аптечку сразу на «Аналитике расхода» (просьба — файлом в папке сеанса: окно могло быть уже открыто).
fn open_analytics() {
    let _ = crate::store::write_private_quick(&crate::store::runtime_dir().join("analytics.req"), b"1");
    if let Some(exe) = crate::procs::self_exe() {
        crate::procs::spawn_detached(&mut std::process::Command::new(exe));
    }
}

/// TRAY_SHOT=файл.png — плашку значка в картинку, без панели и окна (с настоящими цифрами; UI_K — масштаб).
pub fn shot(path: &str) {
    let k = std::env::var("UI_K").ok().and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 4);
    // UI_FAKE — образец данных (для снимков в README: свои расходы не светим)
    let agents: Vec<Agent> = if std::env::var_os("UI_FAKE").is_some() {
        sample_agents()
    } else {
        crate::limits::snapshot().into_iter().filter(|a| shown(a.name)).collect()
    };
    let t = Tpl::new(k);
    let (w, h) = t.size(agents.len());
    let mut buf = vec![0u32; (w * h) as usize];
    {
        let mut c = Canvas::new(&mut buf, w as usize, w, h);
        paint(&mut Fonts::new(), &agents, &Watch::default(), crate::ui::now_secs(), &t, &mut c);
    }
    let rgb: Vec<u8> = buf.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]).collect();
    let Ok(f) = std::fs::File::create(path) else { return };
    let mut e = png::Encoder::new(f, w as u32, h as u32);
    e.set_color(png::ColorType::Rgb);
    if let Ok(mut wr) = e.write_header() {
        let _ = wr.write_image_data(&rgb);
    }
}

/// Образец: два агента с правдоподобными цифрами.
fn sample_agents() -> Vec<Agent> {
    use crate::limits::{Limits, Window as Win};
    let now = crate::ui::now_secs();
    let lim = |a: f64, b: f64| Limits {
        five: Some(Win { pct: a, resets_at: now + 2 * 3600 + 1800, minutes: 300 }),
        week: Some(Win { pct: b, resets_at: now + 4 * 86_400, minutes: 10080 }),
        at: now,
    };
    vec![
        Agent {
            name: "Claude Code",
            limits: Some(lim(42.0, 18.0)),
            today: 18_400_000,
            week: 96_000_000,
            days: vec![6, 9, 4, 12, 0, 14, 7, 11, 16, 9, 3, 10, 13, 8].into_iter().map(|v| v * 1_000_000).collect(),
        },
        Agent {
            name: "Codex",
            limits: Some(lim(73.0, 35.0)),
            today: 4_100_000,
            week: 21_000_000,
            days: vec![0, 2, 3, 0, 1, 4, 2, 0, 3, 5, 2, 4, 3, 4].into_iter().map(|v| v * 1_000_000).collect(),
        },
    ]
}

/// `aptechka --tray`.
pub fn run() {
    // перезапуск новой версией (APTECHKA_REPLACES): прежний значок сейчас закроется — подождать его замок
    let lock = crate::store::runtime_dir().join("tray.lock");
    let one = if std::env::var_os("APTECHKA_REPLACES").is_some() {
        crate::store::lock_wait(&lock, Duration::from_secs(5))
    } else {
        crate::store::lock::try_take(&lock)
    };
    let Some(_one) = one else {
        eprintln!("[значок] уже запущен");
        return;
    };
    let Ok(el) = EventLoop::<Ev>::with_user_event().build() else {
        eprintln!("[значок] нет экрана");
        return;
    };
    el.listen_device_events(winit::event_loop::DeviceEvents::Never);
    let proxy = el.create_proxy();
    // данные — в своём потоке, на фоновом приоритете (первое чтение журналов — около секунды)
    let p2 = proxy.clone();
    let stamp = crate::procs::self_stamp();
    std::thread::spawn(move || {
        crate::footprint::background_priority();
        loop {
            // выключили в меню — значок закрывается сам
            if !enabled() {
                let _ = p2.send_event(Ev::Quit);
                return;
            }
            if p2.send_event(Ev::Data(crate::limits::snapshot())).is_err() {
                return;
            }
            // до следующих цифр — следим за настройками (выбрали другой значок, выключили), раз в 2 с
            let (t0, icon, sig) = (Instant::now(), icon_choice(), shown_sig());
            while t0.elapsed() < EVERY {
                std::thread::sleep(Duration::from_secs(2));
                if !enabled() {
                    break;
                }
                if stamp.is_some() && crate::procs::self_stamp() != stamp {
                    let _ = p2.send_event(Ev::Update);
                    return;
                }
                // что делают агенты — раз в 4 с (кот на значке: спит, идёт, сидит)
                if t0.elapsed().as_secs() % 4 < 2 {
                    let st = agents_state();
                    if st != icon {
                        STATE.store(st, std::sync::atomic::Ordering::Relaxed);
                        let _ = p2.send_event(Ev::Icon);
                        break;
                    }
                }
                if shown_sig() != sig {
                    let _ = p2.send_event(Ev::Shown);
                    break;
                }
            }
        }
    });
    let mut app = App {
        #[cfg(target_os = "linux")]
        tray: sni::Tray::start(proxy.clone()),
        #[cfg(windows)]
        tray: win::Tray::start(proxy.clone()),
        proxy,
        all: Vec::new(),
        agents: Vec::new(),
        popup: None,
        fonts: Fonts::new(),
        k: 1,
        #[cfg(target_os = "linux")]
        reg_t: Instant::now(),
        data_t: 0,
        watch: Watch::default(),
        tpl: None,
        #[cfg(target_os = "linux")]
        panel_n: 0,
    };
    let _ = &app.proxy;
    // значок сразу, ещё до первых цифр
    app.set_tray();
    let _ = el.run_app(&mut app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{Limits, Window as Win};

    #[test]
    fn label_and_tooltip() {
        let now = 1_000;
        let a = Agent {
            name: "Codex",
            limits: Some(Limits {
                five: Some(Win { pct: 64.4, resets_at: 5_000, minutes: 300 }),
                week: Some(Win { pct: 90.0, resets_at: 500, minutes: 10080 }),
                at: 900,
            }),
            today: 1_500_000,
            week: 9_000_000,
            days: vec![0; 14],
        };
        // недельное окно уже сброшено — главный лимит 5-часовой
        assert_eq!(label(std::slice::from_ref(&a), now), "\u{2002}64%");
        assert_eq!(tooltip(&[a], now), "Codex: 5 ч 64% · неделя 0% · сегодня 1,5 млн");
        assert_eq!(label(&[], now), "");
    }

    fn agent(pct5: f64, reset5: i64, at: i64) -> Agent {
        Agent {
            name: "Claude Code",
            limits: Some(Limits { five: Some(Win { pct: pct5, resets_at: reset5, minutes: 300 }), week: None, at }),
            ..Default::default()
        }
    }

    #[test]
    fn watch_alerts_once_and_on_reset() {
        let mut w = Watch::default();
        // первое — только запоминаем, даже если уже много
        assert!(w.update(&[agent(85.0, 10_000, 100)], 100).is_empty());
        assert!(w.update(&[agent(88.0, 10_000, 200)], 200).is_empty());
        let n = w.update(&[agent(96.0, 10_000, 300)], 300);
        assert_eq!(n.len(), 1);
        assert!(n[0].0.contains("96%"), "{n:?}");
        assert!(w.update(&[agent(97.0, 10_000, 400)], 400).is_empty());
        // сброс: новое окно, процент упал
        let n = w.update(&[agent(2.0, 28_000, 10_100)], 10_100);
        assert_eq!(n.len(), 1);
        assert!(n[0].0.contains("снова свободен"), "{n:?}");
        // в новом окне пороги снова работают
        let n = w.update(&[agent(81.0, 28_000, 12_000)], 12_000);
        assert!(n[0].0.contains("81%"), "{n:?}");
    }

    #[test]
    fn watch_forecast() {
        let mut w = Watch::default();
        // 10 % за 20 минут: от 50 % до 100 % — ещё 100 минут; сброс через 3 часа — не хватит
        w.update(&[agent(40.0, 20_000, 1_000)], 1_000);
        w.update(&[agent(50.0, 20_000, 2_200)], 2_200);
        assert_eq!(w.runs_out("Claude Code", 0), Some(2_200 + 6_000));
        // сброс раньше — хватит
        let mut w = Watch::default();
        w.update(&[agent(40.0, 3_000, 1_000)], 1_000);
        w.update(&[agent(50.0, 3_000, 2_200)], 2_200);
        assert_eq!(w.runs_out("Claude Code", 0), None);
    }

    #[test]
    fn number_icon_fits() {
        let mut f = Fonts::new();
        assert!(number_icon(&mut f, 16, None).is_none());
        for p in [5.0, 64.0, 100.0] {
            let px = number_icon(&mut f, 16, Some(p)).unwrap();
            assert_eq!(px.len(), 256);
            // цифры белые — есть светлые точки
            assert!(px.iter().any(|q| q[1] > 230 && q[2] > 230 && q[3] > 230));
        }
    }

    #[test]
    fn icon_sizes() {
        for s in [22, 32, 44] {
            let (n, px) = icon_rgba(s).unwrap();
            assert_eq!((n, px.len()), (s, (s * s) as usize));
        }
    }
}
