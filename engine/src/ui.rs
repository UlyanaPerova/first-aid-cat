//! Окно «Аптечки» в стиле Windows XP: раскладка, рисование и обработка мыши.
//! Всё рисуется своими руками (gfx.rs) прямо в буфер окна. Перерисовываются только
//! «грязные» прямоугольники: наведение, нажатие, новые данные, прокрутка.
//! Картинка кота (art) — отдельная область: её рисует движок (main.rs) по своим правилам.

use crate::diag::{Check, Session, SpeedPhase, SpeedProgress};
use crate::fixes;
use crate::gfx::*;
use crate::spravka;
use std::time::{Duration, Instant};
use tiny_skia::Pixmap;

use crate::catalog::{self, CATS};
/// Вкладки: категории показателей (catalog::CATS) и последняя — «Показатели» (что проверять, а что нет).
const N_TABS: usize = CATS.len() + 1;
const SETTINGS_TAB: u8 = CATS.len() as u8;
fn tab_name(i: usize) -> &'static str {
    CATS.get(i).map(|c| c.name()).unwrap_or("Показатели")
}
fn tab_icon(i: usize) -> &'static str {
    CATS.get(i).map(|c| c.icon()).unwrap_or("settings")
}
/// Кнопки панели инструментов: проверить, вкладки по порядку, журнал, лечение.
const TOOL_ICONS: [&str; N_TABS + 3] = ["check", "net", "ai", "gpu", "dev", "theme", "sys", "settings", "log", "heal"];
const TOOL_LOG: u8 = N_TABS as u8 + 1;
const TOOL_HEAL: u8 = N_TABS as u8 + 2;
const LOG_MAX: usize = 400;
const COL_OK: u32 = 0x007A00;
const COL_WARN: u32 = 0xB36B00;
const COL_BAD: u32 = 0xD40000;
const COL_INFO: u32 = 0x00309C;

fn status_color(st: &str) -> u32 {
    match st {
        "ok" => COL_OK,
        "warn" => COL_WARN,
        "bad" => COL_BAD,
        "info" => COL_INFO,
        _ => BLACK,
    }
}

// ───────────────────────────── Время ─────────────────────────────

pub struct Tm {
    pub mday: i32,
    pub mon: i32,
    pub hour: i32,
    pub min: i32,
    pub sec: i32,
    pub year: i64,
}

#[cfg(unix)]
pub fn localtime(secs: i64) -> Tm {
    #[repr(C)]
    struct C {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        _r: [i32; 3],
        _off: i64,
        _zone: *const u8,
    }
    extern "C" {
        fn localtime_r(t: *const i64, tm: *mut C) -> *mut C;
    }
    let mut c = C { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, _r: [0; 3], _off: 0, _zone: std::ptr::null() };
    unsafe { localtime_r(&secs, &mut c) };
    Tm { mday: c.mday, mon: c.mon + 1, hour: c.hour, min: c.min, sec: c.sec, year: c.year as i64 + 1900 }
}

#[cfg(windows)]
pub fn localtime(secs: i64) -> Tm {
    // struct tm в Windows — 9 int без смещения и зоны
    #[repr(C)]
    struct C {
        sec: i32,
        min: i32,
        hour: i32,
        mday: i32,
        mon: i32,
        year: i32,
        _r: [i32; 3],
    }
    extern "C" {
        fn _localtime64_s(tm: *mut C, t: *const i64) -> i32;
    }
    let mut c = C { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, _r: [0; 3] };
    unsafe { _localtime64_s(&mut c, &secs) };
    Tm { mday: c.mday, mon: c.mon + 1, hour: c.hour, min: c.min, sec: c.sec, year: c.year as i64 + 1900 }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ───────────────────────────── Типы ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    TitleDrag,
    TitleBtn(u8),
    Menu(u8),
    MenuItem(u8, u8),
    Tool(u8),
    Tab(u8),
    Check,
    Speed,
    CopyResume,
    SessionsBtn,
    SessionCopy(u8),
    SessionOpen(u8),
    BalloonClose,
    Fix(u16),
    /// «Игнорировать» / «Не игнорировать» у строки
    Ignore(u16),
    /// «Игнорировать всё» (все красные и жёлтые) / «Вернуть всё»
    IgnoreAll,
    Row(u16),
    Art,
    /// граница между таблицей и журналом (тянется мышью)
    Split,
    Block,
    Dismiss,
    DlgYes,
    DlgCopy,
    DlgNo,
    SpravkaSave,
    SpravkaClose,
    SpravkaX,
    ScrollUp(u8),
    ScrollDown(u8),
    Thumb(u8),
    Track(u8),
}

/// Элемент для дерева специальных возможностей (main.rs → AccessKit).
pub struct A11yNode {
    pub id: u64,
    pub role: A11yRole,
    pub name: String,
    pub rect: Rect,
    pub selected: bool,
    pub value: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum A11yRole {
    Button,
    MenuItem,
    Tab,
    Row,
    Label,
}

/// Постоянный id элемента по его виду и номеру (FNV-1a от текстового вида Hit); 0 и 1 заняты окном и облачком.
fn a11y_id(h: Hit) -> u64 {
    let mut x: u64 = 0xcbf29ce484222325;
    for b in format!("{h:?}").bytes() {
        x ^= b as u64;
        x = x.wrapping_mul(0x100000001b3);
    }
    x | 2
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Popup {
    None,
    Menu(u8),
    Heal,
    Sessions,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuAct {
    TextMode,
    Quit,
    CheckAll,
    Speed,
    Tab(u8),
    ToggleLog,
    Fix(u8),
    Spravka,
    /// лицензии и уведомления (licenses.rs): положить в папку сеанса и открыть
    Licenses,
    About,
    WidgetShow,
    WidgetStyle(u8),
    WidgetSize(u8),
    /// как часто проверять: 0 редко, 1 обычно, 2 часто (sched.rs, ui.json check_freq)
    Freq(u8),
    /// язык интерфейса: 0 как в системе, 1 русский, 2 английский (i18n.rs, ui.json lang)
    Lang(u8),
}

fn lang_now() -> u8 {
    // APTECHKA_LANG важнее меню — галочка там, где язык на самом деле
    let forced = std::env::var("APTECHKA_LANG").ok();
    match forced.as_deref() {
        Some("ru") => return 1,
        Some("en") => return 2,
        _ => {}
    }
    match crate::i18n::chosen() {
        None => 0,
        Some(crate::i18n::Lang::Ru) => 1,
        Some(crate::i18n::Lang::En) => 2,
    }
}

const FREQ_IDS: [&str; 3] = ["rare", "normal", "often"];

fn freq_now() -> u8 {
    let v = crate::prefs::load().get("check_freq").and_then(|v| v.as_str()).map(String::from).unwrap_or_default();
    FREQ_IDS.iter().position(|f| *f == v).unwrap_or(1) as u8
}

/// Номер меню «Лечение» в строке меню (его пункты повторяет кнопка с аптечкой).
fn heal_menu() -> u8 {
    menus().iter().position(|(n, _)| *n == "Лечение").unwrap_or(0) as u8
}

fn menus() -> Vec<(&'static str, Vec<(String, MenuAct)>)> {
    let fixes_menu: Vec<(String, MenuAct)> = fixes::GENERAL.iter().enumerate().map(|(i, (_, l))| (l.to_string(), MenuAct::Fix(i as u8))).collect();
    let mut view: Vec<(String, MenuAct)> = (0..N_TABS).map(|i| (tab_name(i).to_string(), MenuAct::Tab(i as u8))).collect();
    view.push(("Журнал".into(), MenuAct::ToggleLog));
    // названия языков — каждое на своём языке (не переводятся), чтобы найти свой в чужом интерфейсе
    view.push(("Язык: как в системе".into(), MenuAct::Lang(0)));
    view.push(("Русский".into(), MenuAct::Lang(1)));
    view.push(("English".into(), MenuAct::Lang(2)));
    let mut m = vec![
        ("Файл", vec![("Текстовый режим (таблица в терминале)".into(), MenuAct::TextMode), ("Выход".into(), MenuAct::Quit)]),
        (
            "Проверки",
            vec![
                ("Проверить всё".into(), MenuAct::CheckAll),
                ("Измерить скорость".into(), MenuAct::Speed),
                ("Проверять редко (экономно)".into(), MenuAct::Freq(0)),
                ("Проверять обычно".into(), MenuAct::Freq(1)),
                ("Проверять часто".into(), MenuAct::Freq(2)),
            ],
        ),
        ("Вид", view),
        ("Виджет", widget_menu()),
        ("Лечение", fixes_menu),
        (
            "Справка",
            vec![("Справка от Мяудоктора".into(), MenuAct::Spravka), ("Лицензии".into(), MenuAct::Licenses), ("О программе".into(), MenuAct::About)],
        ),
    ];
    // НЕ ВОЗВРАЩАТЬ меню «Виджет» на macOS (даже ради одинаковых меню на всех системах): там виджет —
    // системный (WidgetKit), его добавляют сами через «Изменить виджеты», вид у него один — «Окно»
    // (система не даёт виджетам другую форму), а окно-виджет на рабочем столе на Маке — костыль
    if cfg!(target_os = "macos") {
        m.retain(|(n, _)| *n != "Виджет");
    }
    m
}

/// Меню «Виджет»: показать на рабочем столе, вид (как в macos/widget и widget_look.rs), размер.
fn widget_menu() -> Vec<(String, MenuAct)> {
    let mut v = Vec::new();
    // НЕ ДОБАВЛЯТЬ на macOS «Показать на рабочем столе» (даже ради одинаковых меню на всех системах):
    // там виджет — системный (WidgetKit), его ставят сами через «Изменить виджеты» на рабочем столе,
    // а из программы его не показать. Пункт вёл бы к отдельному окну-виджету, которое на Маке лишнее.
    if !cfg!(target_os = "macos") {
        v.push(("Показать на рабочем столе".to_string(), MenuAct::WidgetShow));
    }
    for (i, s) in crate::widget_look::STYLES.iter().enumerate() {
        v.push((format!("Вид: {}", s.name()), MenuAct::WidgetStyle(i as u8)));
    }
    v.push(("Размер: маленький".into(), MenuAct::WidgetSize(0)));
    v.push(("Размер: средний".into(), MenuAct::WidgetSize(1)));
    v
}

/// Что окно просит сделать у программы (то, что не касается самого рисования).
#[derive(Clone, Debug)]
pub enum Cmd {
    Quit,
    Minimize,
    /// развернуть / вернуть размер (картинка крупнее до размера экрана)
    Maximize,
    DragWindow,
    RunChecks,
    RunSpeed,
    RunFix {
        id: String,
        label: String,
        arg: Option<String>,
    },
    CopyText(String),
    OpenResume(String),
    TextMode,
    ArtClick(i32, i32),
    /// Сохранить открытую справку картинкой (рисует и пишет файл программа, см. `Ui::spravka_png`).
    SaveSpravka,
    /// Открыть виджет на рабочем столе (если ещё не открыт).
    ShowWidget,
    /// Сменили вид или размер виджета (macOS: перерисовать картинки для WidgetKit).
    WidgetChanged,
    /// Сменили язык: заголовок окна, виджет macOS.
    LangChanged,
    /// Строку велели игнорировать (или перестать): пересчитать итог, кота, экраны.
    IgnoreChanged,
}

/// Открытая «Справка от Мяудоктора»: текст и готовая раскладка листа.
struct SpravkaState {
    doc: spravka::Doc,
    page: spravka::Page,
}

/// Всплывающая подсказка: текст и где стояла мышь.
struct Tip {
    text: String,
    x: i32,
    y: i32,
}

/// Через сколько наведение превращается в подсказку.
const TIP_DELAY: Duration = Duration::from_millis(600);

struct Dialog {
    head: String,
    body: String,
    yes_label: Option<String>,
    no_label: String,
    fix: Option<(String, String, Option<String>)>,
    /// Та же починка вручную — команда для терминала (с кнопкой «Копировать команду»).
    cmd: Option<String>,
}

#[derive(Clone, Copy)]
struct SpeedLive {
    phase: SpeedPhase,
    fraction: f64,
}

pub struct Gfx {
    pub fonts: Fonts,
    pub icons: Icons,
}

struct St {
    k: i32,
    w: i32,
    h: i32,
    art_w: i32,
    art_h: i32,
    checks: Vec<Check>,
    speed_checks: Vec<Check>,
    rows: Vec<Check>,
    /// названия игнорируемых проверок (prefs::ignored), обновляются вместе со строками
    ignored: std::collections::HashSet<String>,
    tab: u8,
    sessions: Vec<Session>,
    checking: bool,
    last_check: Option<i64>,
    speed_vals: [Option<f64>; 3], // вниз, вверх, пинг
    speed_live: Option<SpeedLive>,
    balloon: String,
    balloon_closed: Option<String>,
    avatar: Option<Pixmap>,
    log: Vec<String>,
    log_vis: bool,
    hover: Option<Hit>,
    pressed: Option<Hit>,
    popup: Popup,
    dialog: Option<Dialog>,
    scroll: [i32; 3], // таблица, журнал, справка
    spravka: Option<SpravkaState>,
    tip: Option<Tip>,
    drag: Option<(u8, i32, i32)>, // полоса, y мыши, сдвиг при нажатии
    status3: String,
    kernel: String,
    iface: String,
    /// анимация вкладок: прошлая вкладка и когда переключили
    tab_anim: Option<(u8, Instant)>,
    /// высота таблицы в логических пикселях (граница с журналом), None — по умолчанию
    split: Option<i32>,
    /// тянут границу: (y мыши при нажатии, высота таблицы тогда, в пикселях окна)
    split_drag: Option<(i32, i32)>,
    /// окно развёрнуто (картинка крупнее)
    maxed: bool,
}

#[derive(Clone)]
struct Geo {
    title: Rect,
    menubar: Rect,
    tools: [Rect; N_TABS + 3],
    frame: Rect,
    art: Rect,
    right: Rect,
    claude: Rect,
    balloon: Rect,
    tabs: Rect,
    speed: Rect,
    table: Rect,
    log: Rect,
    status: [Rect; 3],
}

/// Окно справки: рамка, заголовок, утопленная область с листом, полоса прокрутки, кнопки.
struct SpGeo {
    win: Rect,
    title: Rect,
    close: Rect,
    frame: Rect,
    view: Rect,
    bar: Rect,
    save: Rect,
    done: Rect,
}

impl St {
    /// Названия нынешних красных и жёлтых строк (игнорируемые уже «для сведения» — их тут нет).
    fn problem_titles(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .checks
            .iter()
            .chain(self.speed_checks.iter())
            .filter(|c| matches!(c.status.as_str(), "bad" | "warn") && unit_on(c.unit))
            .map(|c| c.title.clone())
            .collect();
        v.dedup();
        v
    }

    /// Надпись кнопки «Игнорировать всё»: есть беды — игнорировать их, нет, но что-то игнорируется — вернуть.
    fn ignore_all_label(&self) -> Option<&'static str> {
        if !self.problem_titles().is_empty() {
            Some("Игнорировать всё")
        } else if !self.ignored.is_empty() {
            Some("Вернуть всё")
        } else {
            None
        }
    }

    fn sp_geo(&self) -> SpGeo {
        let k = self.k;
        let sb = 17 * k;
        let pw = (spravka::W * k).min(self.w - 24 * k - sb - 20 * k).max(300 * k);
        let win_w = pw + sb + 20 * k;
        let win_h = (self.h - 20 * k).min(1000 * k);
        let win = Rect::new((self.w - win_w) / 2, (self.h - win_h) / 2, win_w, win_h);
        let title = Rect::new(win.x + 3 * k, win.y + 3 * k, win.w - 6 * k, 22 * k);
        let close = Rect::new(title.right() - 20 * k, title.y + 2 * k, 18 * k, 18 * k);
        let done = Rect::new(win.right() - 12 * k - 110 * k, win.bottom() - 12 * k - 28 * k, 110 * k, 28 * k);
        let save = Rect::new(done.x - 10 * k - 220 * k, done.y, 220 * k, 28 * k);
        let top = title.bottom() + 8 * k;
        let frame = Rect::new(win.x + 8 * k, top, win.w - 16 * k, done.y - 10 * k - top);
        let inner = frame.inset(2 * k);
        let view = Rect::new(inner.x, inner.y, inner.w - sb, inner.h);
        let bar = Rect::new(view.right(), inner.y, sb, inner.h);
        SpGeo { win, title, close, frame, view, bar, save, done }
    }

    fn geo(&self) -> Geo {
        let k = self.k;
        let u = |n: i32| n * k;
        let (w, h) = (self.w, self.h);
        let wt = u(60);
        let tools = std::array::from_fn(|i| Rect::new(u(6), wt + i as i32 * u(48), u(44), u(44)));
        // рамка картинки — во всю высоту середины; если картинка ниже, она прижата к низу
        // (верх у картинки тёмный — пустое место над ней сливается с фоном)
        let frame = Rect::new(u(56), wt, self.art_w + u(4), h - wt - u(38));
        let art = Rect::new(frame.x + u(2), frame.bottom() - u(2) - self.art_h, self.art_w, self.art_h);
        let rx = frame.right() + u(6);
        let right = Rect::new(rx, wt, w - rx - u(6), frame.h);
        let mut y = wt;
        let claude = Rect::new(rx, y, right.w, u(64));
        y = claude.bottom() + u(6);
        let balloon = if self.balloon_visible() {
            let r = Rect::new(rx, y, right.w, u(56));
            y = r.bottom() + u(6);
            r
        } else {
            Rect::default()
        };
        let tabs = Rect::new(rx, y, right.w, u(46));
        y = tabs.bottom() + u(4);
        let speed = Rect::new(rx, y, right.w, u(30));
        y = speed.bottom() + u(6);
        let rest = right.bottom() - y;
        let (table, log) = if self.log_vis {
            let want = self.split.map(u).unwrap_or(rest * 62 / 100);
            let th = want.max(u(110)).min(rest - u(70));
            (Rect::new(rx, y, right.w, th), Rect::new(rx, y + th + u(6), right.w, rest - th - u(6)))
        } else {
            (Rect::new(rx, y, right.w, rest), Rect::default())
        };
        let sy = h - u(30);
        let (w3, w2) = (u(230), u(330));
        let status = [
            Rect::new(u(4), sy, w - u(8) - w3 - w2 - u(6) - u(14), u(26)),
            Rect::new(w - u(18) - w3 - u(3) - w2, sy, w2, u(26)),
            Rect::new(w - u(18) - w3, sy, w3, u(26)),
        ];
        Geo {
            title: Rect::new(0, 0, w, u(30)),
            menubar: Rect::new(0, u(30), w, u(24)),
            tools,
            frame,
            art,
            right,
            claude,
            balloon,
            tabs,
            speed,
            table,
            log,
            status,
        }
    }

    fn balloon_visible(&self) -> bool {
        !self.balloon.is_empty() && self.balloon_closed.as_deref() != Some(self.balloon.as_str())
    }

    fn main_session(&self) -> Option<&Session> {
        self.sessions.iter().find(|s| s.live).or_else(|| self.sessions.first())
    }

    fn table_rows(&self) -> i32 {
        self.rows.len() as i32
    }

    fn scroll_max(&self, id: u8) -> i32 {
        let g = self.geo();
        let k = self.k;
        if id == 2 {
            return self.spravka.as_ref().map_or(0, |sp| (sp.page.h - self.sp_geo().view.h).max(0));
        }
        if id == 0 {
            let view = g.table.h - 4 * k - 22 * k;
            whole_lines(self.table_rows() * 26 * k - view, 26 * k)
        } else {
            let view = g.log.h - 4 * k;
            whole_lines(self.log.len() as i32 * 16 * k + 4 * k - view, 16 * k)
        }
    }
}

/// Прокрутка до конца — на целое число строк: верхняя строка видна целиком, а не срезанной.
fn whole_lines(over: i32, line: i32) -> i32 {
    if over <= 0 {
        0
    } else {
        (over + line - 1) / line * line
    }
}

/// Смешать два цвета 0xRRGGBB: t = 0 — a, t = 1 — b.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        ((x + (y - x) * t.clamp(0.0, 1.0)).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Раскладка окна, общая с Python-версией: ui.json в папке настроек (prefs.rs).
fn ui_file() -> serde_json::Value {
    crate::prefs::load()
}

fn save_ui(split: Option<i32>, log: bool) {
    crate::prefs::update(|v| {
        if let Some(s) = split {
            v["split"] = serde_json::json!(s);
        }
        v["log"] = serde_json::json!(log);
    });
}

/// Длительность анимации переключения вкладок.
const TAB_ANIM: std::time::Duration = std::time::Duration::from_millis(90);

// ───────────────────────────── Ui ─────────────────────────────

pub struct Ui {
    g: Gfx,
    st: St,
    hits: Vec<(Rect, Hit)>,
    hits_ok: bool,
    hover_since: Option<Instant>,
    /// реплика на тычок: (когда сказал, что было в облачке до неё, сама реплика)
    quip: Option<(Instant, String, String)>,
    mouse: (i32, i32),
    dirty: Vec<Rect>,
    pub w: i32,
    pub h: i32,
    pub art: Rect,
}

impl Ui {
    /// k — во сколько раз увеличить «логические» пиксели (масштаб экрана),
    /// art_w/art_h — размер картинки в пикселях окна (клетки × масштаб).
    /// Идёт анимация (переключение вкладок) — окну стоит рисовать чаще.
    pub fn animating(&self) -> bool {
        self.st.tab_anim.is_some()
    }

    pub fn k(&self) -> i32 {
        self.st.k
    }

    /// Размер окна для картинки art_w×art_h при масштабе k.
    pub fn size_for(k: i32, art_w: i32, art_h: i32) -> (i32, i32) {
        let u = |n: i32| n * k.max(1);
        (u(56) + art_w + u(4) + u(6) + u(600) + u(6), u(60) + (art_h + u(4)).max(Self::MIN_BODY * k.max(1)) + u(6) + u(32))
    }

    /// Высота середины окна не меньше этой (×k): столбик из 10 кнопок слева (10×48−4)
    /// и справа — Claude, облачко, вкладки, скорость, хотя бы 3 строки таблицы и журнал.
    pub const MIN_BODY: i32 = 480;

    /// Окно w×h (не меньше нужного картинке art_w×art_h): лишнее место забирает правая часть.
    pub fn resize(&mut self, w: i32, h: i32, art_w: i32, art_h: i32, maxed: bool) {
        let (mw, mh) = Ui::size_for(self.st.k, art_w, art_h);
        let (w, h) = (w.max(mw), h.max(mh));
        self.st.art_w = art_w;
        self.st.art_h = art_h;
        self.st.w = w;
        self.st.h = h;
        self.st.maxed = maxed;
        self.st.popup = Popup::None;
        self.st.tip = None;
        // раскладка сменилась — что было под мышью, уже не там; подсказку не показываем, пока мышь не сдвинется
        self.st.hover = None;
        self.hover_since = None;
        self.w = w;
        self.h = h;
        self.art = self.st.geo().art;
        self.dirty.clear();
        self.invalidate_all();
    }

    pub fn new(k: i32, art_w: i32, art_h: i32) -> Ui {
        let k = k.max(1);
        let (w, h) = Ui::size_for(k, art_w, art_h);
        let mut st = St {
            k,
            w,
            h,
            art_w,
            art_h,
            checks: Vec::new(),
            speed_checks: Vec::new(),
            ignored: Default::default(),
            rows: Vec::new(),
            tab: 0,
            sessions: Vec::new(),
            checking: true,
            last_check: None,
            speed_vals: [None; 3],
            speed_live: None,
            balloon: "Проверяю…".into(),
            balloon_closed: None,
            avatar: None,
            log: Vec::new(),
            log_vis: ui_file().get("log").and_then(|v| v.as_bool()).unwrap_or(true),
            hover: None,
            pressed: None,
            popup: Popup::None,
            dialog: None,
            scroll: [0; 3],
            spravka: None,
            tip: None,
            drag: None,
            status3: String::new(),
            kernel: os_label(),
            iface: default_iface(),
            tab_anim: None,
            split: ui_file().get("split").and_then(|v| v.as_i64()).map(|v| v as i32),
            split_drag: None,
            maxed: false,
        };
        st.status3 = String::new();
        let art = st.geo().art;
        let mut ui = Ui {
            g: Gfx { fonts: Fonts::new(), icons: Icons::new() },
            st,
            hits: Vec::new(),
            hits_ok: false,
            hover_since: None,
            quip: None,
            mouse: (0, 0),
            dirty: vec![Rect::new(0, 0, w, h)],
            w,
            h,
            art,
        };
        ui.tick_second();
        ui.log("Аптечка запущена.");
        ui
    }

    // ───── рисование ─────

    fn with_p<R>(&mut self, c: &mut Canvas, rec: bool, f: impl FnOnce(&mut P) -> R) -> (R, Vec<(Rect, Hit)>) {
        let geo = self.st.geo();
        let mut hits = Vec::new();
        let r = {
            let mut p = P { c, g: &mut self.g, s: &self.st, geo, hits: if rec { Some(&mut hits) } else { None } };
            f(&mut p)
        };
        (r, hits)
    }

    /// Пересчитать области нажатий (прогоном рисования по пустому холсту).
    fn relayout(&mut self) {
        let mut c = Canvas::none();
        let ((), hits) = self.with_p(&mut c, true, |p| {
            p.base();
            p.overlay();
        });
        self.hits = hits;
        self.hits_ok = true;
    }

    fn ensure_hits(&mut self) {
        if !self.hits_ok {
            self.relayout();
        }
    }

    /// Нарисовать всё окно (в пределах c.clip), включая всплывающее поверх.
    pub fn paint(&mut self, c: &mut Canvas) {
        self.with_p(c, false, |p| {
            p.base();
            p.overlay();
        });
    }

    /// Только всплывающее (меню, окно вопроса): поверх картинки, когда она обновилась.
    pub fn paint_overlay(&mut self, c: &mut Canvas) {
        self.with_p(c, false, |p| p.overlay());
    }

    pub fn overlay_rect(&mut self) -> Rect {
        let mut c = Canvas::none();
        let (r, _) = self.with_p(&mut c, false, |p| p.overlay_rect());
        r
    }

    /// Пустое место в рамке над картинкой (окно выше картинки): туда тянем провода — верхний ряд картинки.
    pub fn art_void(&self) -> Rect {
        let g = self.st.geo();
        let top = g.frame.y + 2 * self.st.k;
        Rect::new(g.art.x, top, g.art.w, g.art.y - top)
    }

    pub fn take_dirty(&mut self) -> Vec<Rect> {
        std::mem::take(&mut self.dirty)
    }

    pub fn has_dirty(&self) -> bool {
        !self.dirty.is_empty()
    }

    fn invalidate(&mut self, r: Rect) {
        self.hits_ok = false;
        if !r.is_empty() {
            self.dirty.push(r);
        }
    }

    fn invalidate_all(&mut self) {
        let r = Rect::new(0, 0, self.w, self.h);
        self.invalidate(r);
    }

    fn invalidate_right(&mut self) {
        let r = self.st.geo().right;
        // правая колонка могла сдвинуться (облачко) — перерисуем её целиком
        self.invalidate(r);
    }

    // ───── данные ─────

    pub fn set_avatar(&mut self, pm: Pixmap) {
        self.st.avatar = box_scale(&pm, (52 * self.st.k) as u32);
        let r = self.st.geo().balloon;
        self.invalidate(r);
    }

    pub fn log(&mut self, text: &str) {
        let t = localtime(now_secs());
        let line = format!("[{:02}:{:02}:{:02}] {}", t.hour, t.min, t.sec, text);
        let at_end = self.st.scroll[1] >= self.st.scroll_max(1);
        eprintln!("[окно] {text}");
        self.st.log.push(line);
        if self.st.log.len() > LOG_MAX {
            let extra = self.st.log.len() - LOG_MAX;
            self.st.log.drain(..extra);
        }
        if at_end {
            self.st.scroll[1] = self.st.scroll_max(1);
        }
        let r = self.st.geo().log;
        self.invalidate(r);
    }

    /// Реплика кота на тычок: показывается ненадолго, потом облачко возвращается к прежнему.
    pub fn quip(&mut self, text: &str) {
        let prev = match self.quip.take() {
            Some((_, prev, shown)) if self.st.balloon == shown => prev,
            _ => self.st.balloon.clone(),
        };
        self.st.balloon_closed = None;
        self.say(text);
        self.quip = Some((Instant::now(), prev, text.to_string()));
    }

    /// Сообщение кота в облачке (как say() в Python).
    pub fn say(&mut self, text: &str) {
        if self.st.balloon != text {
            self.st.balloon = text.to_string();
        }
        self.invalidate_right();
    }

    pub fn set_checking(&mut self, on: bool) {
        self.st.checking = on;
        let g = self.st.geo();
        self.invalidate(g.status[0]);
        self.invalidate(g.table);
    }

    fn rebuild_rows(&mut self) {
        // строки перестраиваются (пришли новые проверки) — начатое нажатие на кнопку строки отменяется:
        // кнопка по номеру строки указала бы уже на другую починку
        if matches!(self.st.pressed, Some(Hit::Fix(_)) | Some(Hit::Ignore(_)) | Some(Hit::Row(_))) {
            self.st.pressed = None;
        }
        self.st.ignored = crate::prefs::ignored();
        if self.st.tab == SETTINGS_TAB {
            self.st.rows = settings_rows();
            let m = self.st.scroll_max(0);
            self.st.scroll[0] = self.st.scroll[0].clamp(0, m);
            return;
        }
        let group = CATS.get(self.st.tab as usize).map(|c| c.key()).unwrap_or("net");
        let rank = |s: &str| match s {
            "bad" => 0,
            "warn" => 1,
            "ok" => 2,
            _ => 3,
        };
        let mut rows: Vec<Check> =
            self.st.checks.iter().chain(self.st.speed_checks.iter()).filter(|c| c.key == group && unit_on(c.unit)).cloned().collect();
        rows.sort_by_key(|c| rank(&c.status));
        self.st.rows = rows;
        let m = self.st.scroll_max(0);
        self.st.scroll[0] = self.st.scroll[0].clamp(0, m);
    }

    fn all_checks(&self) -> impl Iterator<Item = &Check> {
        self.st.checks.iter().chain(self.st.speed_checks.iter())
    }

    /// Пришёл новый прогон проверок. full_log — записать каждую проверку в журнал.
    pub fn set_checks(&mut self, checks: Vec<Check>, full_log: bool) {
        if full_log {
            for r in &checks {
                let mark = match r.status.as_str() {
                    "ok" => "OK",
                    "warn" => "внимание:",
                    "bad" => "ОШИБКА:",
                    _ => "",
                };
                // части — переводом по отдельности (i18n.rs): склеенную строку журнала в таблице не найти
                self.log(&format!("{}... {} {}", crate::i18n::tr(&r.title), mark, crate::i18n::tr(&r.value)).replace("  ", " "));
            }
        } else {
            let changes: Vec<String> = checks
                .iter()
                .filter(|n| self.st.checks.iter().find(|o| o.title == n.title).is_some_and(|o| o.status != n.status))
                .map(|n| format!("{} теперь {}: {}", n.title, n.status, n.value))
                .collect();
            for ch in changes {
                self.log(&ch);
            }
        }
        self.st.checks = checks;
        self.st.checking = false;
        self.st.last_check = Some(now_secs());
        let real: Vec<&Check> = self.all_checks().filter(|c| c.status != "info").collect();
        let bad = real.iter().filter(|c| c.status == "bad").count();
        let warn = real.iter().filter(|c| c.status == "warn").count();
        if full_log {
            self.log(&format!("Готово. Проблем: {bad}, предупреждений: {warn}."));
        }
        self.rebuild_rows();
        self.update_mood();
        self.invalidate_all_but_art();
    }

    fn invalidate_all_but_art(&mut self) {
        let g = self.st.geo();
        self.invalidate(g.right);
        self.invalidate(Rect::new(0, g.status[0].y, self.w, g.status[0].h));
    }

    /// Фраза кота по худшей проверке (как _update_mood в gui_xp.py).
    fn update_mood(&mut self) {
        let all: Vec<&Check> = self.all_checks().collect();
        for st in ["bad", "warn"] {
            let items: Vec<&&Check> = all.iter().filter(|r| r.status == st).collect();
            if let Some(r) = items.first() {
                let more = if items.len() > 1 { format!(" И ещё {}.", items.len() - 1) } else { String::new() };
                let act = r.fixes.first().map(|f| format!(" Нажми «{}».", f.label)).unwrap_or_default();
                let pre = if st == "bad" { "Мур! Беда:" } else { "Хм…" };
                // «Системный прокси» → «системный прокси», но названия (OBS, VPN, Firefox) не трогаем
                let mut name = r.title.chars();
                let (c0, c1) = (r.title.chars().next(), r.title.chars().nth(1));
                // по-английски не меняем: строчное название не нашлось бы в таблице перевода (i18n.rs)
                let common = crate::i18n::lang() == crate::i18n::Lang::Ru
                    && c0.is_some_and(|c| ('А'..='Я').contains(&c) || c == 'Ё')
                    && c1.is_some_and(char::is_lowercase);
                let lower: String = if common {
                    name.next().map(|c| c.to_lowercase().collect::<String>()).unwrap_or_default() + name.as_str()
                } else {
                    r.title.clone()
                };
                let msg = format!("{pre} {lower} — {}.{act}{more}", r.value);
                self.say(&msg);
                return;
            }
        }
        let live = self.st.sessions.iter().any(|s| s.live);
        let msg = ok_phrase(live);
        self.say(&msg);
    }

    pub fn set_sessions(&mut self, ss: Vec<Session>) {
        // сравниваем всё, что видно (и название: его могли исправить или скрыть в нём ключ)
        let same = ss.len() == self.st.sessions.len()
            && ss
                .iter()
                .zip(&self.st.sessions)
                .all(|(a, b)| a.id == b.id && a.status == b.status && a.live == b.live && a.title == b.title && a.cwd == b.cwd && a.agent == b.agent);
        if same {
            return;
        }
        // список поменялся — нажатие на кнопку сессии по номеру отменяем
        if matches!(self.st.pressed, Some(Hit::SessionCopy(_)) | Some(Hit::SessionOpen(_))) {
            self.st.pressed = None;
        }
        let was_live = self.st.sessions.iter().any(|s| s.live);
        self.st.sessions = ss;
        let g = self.st.geo();
        self.invalidate(g.claude);
        if self.st.popup == Popup::Sessions {
            let r = self.overlay_rect();
            self.invalidate(r);
        }
        // если кот говорит про Клода устаревшее — обновить фразу
        let live = self.st.sessions.iter().any(|s| s.live);
        if live != was_live && OK_CLAUDE_ON.iter().chain(OK_CLAUDE_OFF.iter()).any(|m| *m == self.st.balloon) {
            let msg = ok_phrase(live);
            self.say(&msg);
        }
    }

    pub fn start_speed(&mut self) {
        self.st.speed_live = Some(SpeedLive { phase: SpeedPhase::Ping, fraction: 0.0 });
        self.st.speed_vals = [None; 3];
        self.say("Меряю скорость: пинг, скачивание и отдачу…");
        self.log("Измеряю скорость…");
        let r = self.st.geo().speed;
        self.invalidate(r);
    }

    /// Ничего не происходит: нет открытого меню, окна подтверждения и замера скорости — можно перезапуститься.
    pub fn is_idle(&self) -> bool {
        matches!(self.st.popup, Popup::None) && self.st.dialog.is_none() && !self.is_speed_running()
    }

    pub fn is_speed_running(&self) -> bool {
        self.st.speed_live.is_some()
    }

    pub fn speed_progress(&mut self, p: SpeedProgress) {
        let idx = match p.phase {
            SpeedPhase::Down => 0,
            SpeedPhase::Up => 1,
            SpeedPhase::Ping => 2,
        };
        if p.phase == SpeedPhase::Ping {
            if p.ping_ms.is_some() {
                self.st.speed_vals[2] = p.ping_ms;
            }
        } else {
            self.st.speed_vals[idx] = Some(p.mbps);
        }
        self.st.speed_live = Some(SpeedLive { phase: p.phase, fraction: p.fraction });
        let r = self.st.geo().speed;
        self.invalidate(r);
    }

    pub fn speed_done(&mut self, res: Vec<Check>) {
        self.st.speed_live = None;
        let num = |t: &str| res.iter().find(|c| c.title == t).and_then(|c| c.value.split_whitespace().next()?.parse::<f64>().ok());
        self.st.speed_vals = [num("Скорость ↓ скачивание"), num("Скорость ↑ отдача"), num("Пинг")];
        for c in &res {
            self.log(&format!("{}... {}", crate::i18n::tr(&c.title), crate::i18n::tr(&c.value)));
        }
        self.st.speed_checks = res;
        self.log("Скорость измерена.");
        self.rebuild_rows();
        self.update_mood();
        self.invalidate_all_but_art();
    }

    /// Строки замера скорости без записи в журнал (подхвачены из общего файла или устарели).
    pub fn set_speed_rows(&mut self, res: Vec<Check>) {
        let num = |t: &str| res.iter().find(|c| c.title == t).and_then(|c| c.value.split_whitespace().next()?.parse::<f64>().ok());
        self.st.speed_vals = [num("Скорость ↓ скачивание"), num("Скорость ↑ отдача"), num("Пинг")];
        self.st.speed_checks = res;
        self.rebuild_rows();
        self.update_mood();
        self.invalidate_all_but_art();
    }

    /// Значения скорости (вниз, вверх, пинг) для экрана в картинке.
    pub fn speed_values(&self) -> Option<(f64, f64, f64)> {
        let v = self.st.speed_vals;
        if self.st.speed_live.is_some() {
            return None;
        }
        Some((v[0]?, v[1]?, v[2]?))
    }

    pub fn fix_started(&mut self, label: &str) {
        let what = if crate::i18n::lang() == crate::i18n::Lang::Ru { label.to_lowercase() } else { label.to_string() };
        self.say(&format!("Лечу: {what}…"));
        self.log(&format!("Лечение: {label}…"));
    }

    pub fn fix_done(&mut self, ok: bool, msg: &str) {
        self.log(&format!("{}{}", if ok { "Вылечено: " } else { "Не вышло: " }, msg));
        self.say(&format!("{}{}", if ok { "Готово! " } else { "Не получилось :( " }, msg));
    }

    /// Раз в секунду: часы в строке состояния и обратный отсчёт.
    pub fn tick_second(&mut self) {
        // ошибка сохранения настроек — в журнал окна, не только в stderr
        if let Some(e) = crate::prefs::take_error() {
            self.log(&e);
        }
        let now = now_secs();
        let t = localtime(now);
        let s3 = format!("{} | {} | {:02}:{:02}:{:02}", self.st.kernel, self.st.iface, t.hour, t.min, t.sec);
        let changed = s3 != self.st.status3;
        self.st.status3 = s3;
        if changed {
            let g = self.st.geo();
            self.invalidate(g.status[1].union(g.status[2]));
        }
    }

    pub fn demo_state(&mut self, speed_live: Option<(SpeedPhase, f64, f64)>) {
        if let Some((phase, mbps, fraction)) = speed_live {
            self.st.speed_vals = [Some(158.0), None, Some(172.0)];
            self.st.speed_vals[if phase == SpeedPhase::Up { 1 } else { 0 }] = Some(mbps);
            self.st.speed_live = Some(SpeedLive { phase, fraction });
        }
        self.invalidate_all();
    }

    // ───── мышь и клавиатура ─────

    fn hit_at(&self, x: i32, y: i32) -> Option<Hit> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y)).map(|(_, h)| *h)
    }

    fn hit_rect(&self, h: Hit) -> Rect {
        self.hits.iter().rev().find(|(_, x)| *x == h).map(|(r, _)| *r).unwrap_or_default()
    }

    pub fn mouse_move(&mut self, x: i32, y: i32) {
        self.mouse = (x, y);
        self.ensure_hits();
        if let Some((y0, th0)) = self.st.split_drag {
            let k = self.st.k;
            let th = (th0 + y - y0).max(110 * k);
            let new = Some(th / k);
            if new != self.st.split {
                self.st.split = new;
                self.st.scroll[0] = self.st.scroll[0].min(self.st.scroll_max(0));
                self.st.scroll[1] = self.st.scroll_max(1);
                self.invalidate_right();
            }
            return;
        }
        if let Some((id, y0, off0)) = self.st.drag {
            let max = self.st.scroll_max(id);
            let track = self.scroll_track(id);
            let thumb = self.thumb_len(id, track);
            let span = (track - thumb).max(1);
            let off = (off0 + (y - y0) * max / span).clamp(0, max);
            self.set_scroll(id, off);
            return;
        }
        let h = self.hit_at(x, y);
        if h == self.st.hover {
            return;
        }
        let (old, new) = (self.st.hover, h);
        self.st.hover = h;
        self.hide_tip();
        self.hover_since = h.and_then(|hh| self.tip_text(hh)).map(|_| Instant::now());
        for hh in [old, new].into_iter().flatten() {
            if matches!(hh, Hit::Block | Hit::Dismiss | Hit::Art | Hit::TitleDrag) {
                continue;
            }
            let r = self.hit_rect(hh);
            self.invalidate_keep_hits(r);
        }
        // при открытом меню наведение на другой пункт строки меню переключает его
        if let (Popup::Menu(cur), Some(Hit::Menu(i))) = (self.st.popup, new) {
            if cur != i {
                let before = self.overlay_rect();
                self.st.popup = Popup::Menu(i);
                let after = self.overlay_rect();
                self.invalidate(before);
                self.invalidate(after);
            }
        }
    }

    fn invalidate_keep_hits(&mut self, r: Rect) {
        if !r.is_empty() {
            self.dirty.push(r);
        }
    }

    pub fn mouse_leave(&mut self) {
        self.hide_tip();
        self.hover_since = None;
        if let Some(h) = self.st.hover.take() {
            let r = self.hit_rect(h);
            self.invalidate_keep_hits(r);
        }
    }

    fn scroll_track(&self, id: u8) -> i32 {
        if id == 2 {
            return self.st.sp_geo().bar.h - 2 * 17 * self.st.k;
        }
        let g = self.st.geo();
        let k = self.st.k;
        let r = if id == 0 { g.table } else { g.log };
        let top = if id == 0 { 22 * k } else { 0 };
        r.h - 4 * k - top - 2 * 17 * k
    }

    fn thumb_len(&self, id: u8, track: i32) -> i32 {
        let k = self.st.k;
        let g = self.st.geo();
        let (view, content) = if id == 2 {
            (self.st.sp_geo().view.h, self.st.spravka.as_ref().map_or(0, |sp| sp.page.h))
        } else if id == 0 {
            (g.table.h - 4 * k - 22 * k, self.st.table_rows() * 26 * k)
        } else {
            (g.log.h - 4 * k, self.st.log.len() as i32 * 16 * k + 4 * k)
        };
        if content <= view {
            return track.max(0);
        }
        (track * view / content.max(1)).max(17 * k).min(track)
    }

    fn set_scroll(&mut self, id: u8, off: i32) {
        let max = self.st.scroll_max(id);
        let off = off.clamp(0, max);
        let i = (id as usize).min(2);
        if self.st.scroll[i] != off {
            self.st.scroll[i] = off;
            let g = self.st.geo();
            let r = match id {
                0 => g.table,
                2 => self.st.sp_geo().frame,
                _ => g.log,
            };
            self.invalidate(r);
        }
    }

    pub fn wheel(&mut self, x: i32, y: i32, lines: f32) {
        let g = self.st.geo();
        let k = self.st.k;
        if self.st.spravka.is_some() {
            let cur = self.st.scroll[2];
            self.set_scroll(2, cur - (lines * 48.0 * k as f32) as i32);
            return;
        }
        let id = if g.table.contains(x, y) {
            0
        } else if self.st.log_vis && g.log.contains(x, y) {
            1
        } else {
            return;
        };
        let step = if id == 0 { 26 * k } else { 16 * k };
        let cur = self.st.scroll[id as usize];
        self.set_scroll(id, cur - (lines * step as f32 * 1.5) as i32);
    }

    pub fn mouse_down(&mut self, x: i32, y: i32) -> Vec<Cmd> {
        self.hide_tip();
        self.hover_since = None;
        self.ensure_hits();
        let h = self.hit_at(x, y);
        let mut cmds = Vec::new();
        // клик мимо открытого меню закрывает его
        if self.st.popup != Popup::None {
            let inside = matches!(
                h,
                Some(Hit::MenuItem(..))
                    | Some(Hit::SessionCopy(_))
                    | Some(Hit::SessionOpen(_))
                    | Some(Hit::Menu(_))
                    | Some(Hit::SessionsBtn)
                    | Some(Hit::Tool(TOOL_HEAL))
            );
            if !inside {
                self.close_popup();
                if h != Some(Hit::Art) {
                    return cmds;
                }
            }
        }
        self.st.pressed = h;
        if h == Some(Hit::Split) {
            let th = self.st.geo().table.h;
            self.st.split_drag = Some((y, th));
            return cmds;
        }
        match h {
            Some(Hit::TitleDrag) => cmds.push(Cmd::DragWindow),
            Some(Hit::Art) => cmds.push(Cmd::ArtClick(x - self.art.x, y - self.art.y)),
            Some(Hit::Menu(i)) => self.toggle_popup(Popup::Menu(i)),
            Some(Hit::SessionsBtn) => self.toggle_popup(Popup::Sessions),
            Some(Hit::Tool(TOOL_HEAL)) => self.toggle_popup(Popup::Heal),
            Some(Hit::Thumb(id)) => {
                self.st.drag = Some((id, y, self.st.scroll[id as usize]));
            }
            Some(Hit::ScrollUp(id)) => {
                let cur = self.st.scroll[id as usize];
                self.set_scroll(id, cur - 26 * self.st.k);
            }
            Some(Hit::ScrollDown(id)) => {
                let cur = self.st.scroll[id as usize];
                self.set_scroll(id, cur + 26 * self.st.k);
            }
            Some(Hit::Track(id)) => {
                let thumb = self.hit_rect(Hit::Thumb(id));
                let page = match id {
                    0 => self.st.geo().table.h - 60 * self.st.k,
                    2 => self.st.sp_geo().view.h * 9 / 10,
                    _ => self.st.geo().log.h - 20 * self.st.k,
                };
                let cur = self.st.scroll[id as usize];
                self.set_scroll(id, if y < thumb.y { cur - page } else { cur + page });
            }
            _ => {}
        }
        if let Some(hh) = h {
            if !matches!(hh, Hit::Block | Hit::Dismiss | Hit::Art | Hit::TitleDrag) {
                let r = self.hit_rect(hh);
                self.invalidate_keep_hits(r);
            }
        }
        cmds
    }

    pub fn mouse_up(&mut self, x: i32, y: i32) -> Vec<Cmd> {
        self.ensure_hits();
        self.st.drag = None;
        if self.st.split_drag.take().is_some() {
            // запоминаем, как в Python-версии (реальную высоту, с учётом ограничений)
            let th = self.st.geo().table.h / self.st.k;
            self.st.split = Some(th);
            save_ui(self.st.split, self.st.log_vis);
        }
        let down = self.st.pressed.take();
        let up = self.hit_at(x, y);
        if let Some(d) = down {
            let r = self.hit_rect(d);
            self.invalidate_keep_hits(r);
        }
        if down.is_none() || down != up {
            return Vec::new();
        }
        let Some(h) = up else { return Vec::new() };
        self.activate(h)
    }

    fn toggle_popup(&mut self, p: Popup) {
        let before = self.overlay_rect();
        self.st.popup = if self.st.popup == p { Popup::None } else { p };
        let after = self.overlay_rect();
        self.invalidate(before);
        self.invalidate(after);
    }

    fn close_popup(&mut self) {
        if self.st.popup != Popup::None {
            let before = self.overlay_rect();
            self.st.popup = Popup::None;
            self.invalidate(before);
            let g = self.st.geo();
            self.invalidate(g.menubar);
            self.invalidate(g.claude);
        }
    }

    fn set_tab(&mut self, i: u8) {
        if self.st.tab != i {
            self.st.tab_anim = Some((self.st.tab, Instant::now()));
            self.st.tab = i;
            self.st.scroll[0] = 0;
            self.rebuild_rows();
        }
        self.invalidate_right();
    }

    fn invalidate_toolbar(&mut self) {
        let g = self.st.geo();
        if let (Some(a), Some(b)) = (g.tools.first(), g.tools.last()) {
            self.invalidate(Rect::new(a.x, a.y, a.w, b.bottom() - a.y));
        }
    }

    fn toggle_log(&mut self) {
        self.st.log_vis = !self.st.log_vis;
        self.st.scroll[1] = self.st.scroll_max(1);
        save_ui(self.st.split, self.st.log_vis);
        self.invalidate_right();
        self.invalidate_toolbar();
        let g = self.st.geo();
        self.invalidate(g.tools[TOOL_LOG as usize]);
    }

    /// Окно подтверждения: что произойдёт и та же починка командой для терминала.
    fn open_fix(&mut self, id: &str, label: &str, arg: Option<String>) -> Vec<Cmd> {
        let cmd = fixes::manual(id, arg.as_deref());
        let warn = fixes::confirm_text(id);
        if warn.is_none() && cmd.is_none() {
            return vec![Cmd::RunFix { id: id.to_string(), label: label.to_string(), arg }];
        }
        self.st.dialog = Some(Dialog {
            head: format!("{label}?"),
            body: warn.unwrap_or_else(|| fixes::what_happens(id)).to_string(),
            yes_label: Some("Да, давай".into()),
            no_label: "Отмена".into(),
            fix: Some((id.to_string(), label.to_string(), arg)),
            cmd,
        });
        self.invalidate_all_dialog();
        Vec::new()
    }

    fn invalidate_all_dialog(&mut self) {
        let r = self.overlay_rect();
        self.invalidate(r);
        self.invalidate_right();
    }

    fn info_dialog(&mut self, head: &str, body: &str) {
        self.st.dialog = Some(Dialog { head: head.into(), body: body.into(), yes_label: None, no_label: "OK".into(), fix: None, cmd: None });
        self.invalidate_all_dialog();
    }

    fn activate(&mut self, h: Hit) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        match h {
            Hit::TitleBtn(0) => cmds.push(Cmd::Minimize),
            Hit::TitleBtn(1) => cmds.push(Cmd::Maximize),
            Hit::TitleBtn(2) => cmds.push(Cmd::Quit),
            Hit::Tool(0) | Hit::Check => cmds.push(Cmd::RunChecks),
            Hit::Tool(i) if (1..=N_TABS as u8).contains(&i) => self.set_tab(i - 1),
            Hit::Tool(TOOL_LOG) => self.toggle_log(),
            Hit::Tab(i) => self.set_tab(i),
            Hit::Speed => cmds.push(Cmd::RunSpeed),
            Hit::CopyResume => {
                if let Some(s) = self.st.main_session().cloned() {
                    cmds.extend(self.copy_resume(&s));
                }
            }
            Hit::SessionCopy(i) => {
                if let Some(s) = self.st.sessions.get(i as usize).cloned() {
                    cmds.extend(self.copy_resume(&s));
                }
            }
            Hit::SessionOpen(i) => {
                if let Some(s) = self.st.sessions.get(i as usize).cloned() {
                    let c = crate::diag::resume_cmd(&s);
                    if c.is_empty() {
                        self.say(if s.id.is_empty() {
                            "Эта сессия только началась — продолжать пока нечего, она и так открыта."
                        } else {
                            "У этой сессии странный номер — не стану запускать его в терминале."
                        });
                        self.close_popup();
                        return cmds;
                    }
                    cmds.push(Cmd::OpenResume(c));
                    self.say("Открываю терминал с этой сессией, мур!");
                    self.close_popup();
                }
            }
            Hit::BalloonClose => {
                self.st.balloon_closed = Some(self.st.balloon.clone());
                self.invalidate_right();
            }
            Hit::Fix(i) => {
                if let Some(f) = self.st.rows.get(i as usize).and_then(|r| r.fixes.first().cloned()) {
                    if f.id == "unit_toggle" {
                        // «Показатели»: включить/выключить без подтверждения, включённый — сразу проверить
                        if let Some(u) = f.arg.as_deref().and_then(catalog::find) {
                            let on = !unit_on(u.id);
                            crate::prefs::set_unit(u.id, on);
                            self.say(&format!(
                                "{} — {}.",
                                u.title,
                                if on {
                                    "снова проверяю"
                                } else {
                                    "больше не проверяю (с компьютером ничего не делала)"
                                }
                            ));
                            if on {
                                cmds.push(Cmd::RunChecks);
                            }
                            self.rebuild_rows();
                            self.invalidate_right();
                        }
                    } else {
                        cmds.extend(self.open_fix(&f.id, &f.label, f.arg.clone()));
                    }
                }
            }
            Hit::IgnoreAll => {
                let probs = self.st.problem_titles();
                if !probs.is_empty() {
                    crate::prefs::set_ignored(&probs, true);
                    self.say(&format!("Не считаю бедой: {}. Кот не грустит. Перезапуск приложения или «Вернуть всё» — снова слежу.", probs.len()));
                } else {
                    let all: Vec<String> = self.st.ignored.iter().cloned().collect();
                    crate::prefs::set_ignored(&all, false);
                    self.say("Снова слежу за всем.");
                }
                cmds.push(Cmd::IgnoreChanged);
            }
            Hit::Ignore(i) => {
                if let Some(title) = self.st.rows.get(i as usize).map(|r| r.title.clone()) {
                    let on = !self.st.ignored.contains(&title);
                    crate::prefs::set_ignored(std::slice::from_ref(&title), on);
                    self.say(&if on {
                        format!("{title} — больше не считаю бедой, кот не грустит. Вернуть — «Не игнорировать».")
                    } else {
                        format!("{title} — снова слежу.")
                    });
                    cmds.push(Cmd::IgnoreChanged);
                }
            }
            Hit::Row(i) => {
                if let Some(r) = self.st.rows.get(i as usize) {
                    let msg = format!("{}: {}.{}", r.title, r.value, if r.detail.is_empty() { String::new() } else { format!(" {}", r.detail) });
                    self.say(&msg);
                }
            }
            Hit::MenuItem(m, i) => {
                let act = menus().get(m as usize).and_then(|x| x.1.get(i as usize).map(|y| y.1));
                self.close_popup();
                if let Some(a) = act {
                    cmds.extend(self.menu_act(a));
                }
            }
            Hit::DlgYes => {
                let d = self.st.dialog.take();
                self.invalidate_all_dialog();
                if let Some((id, label, arg)) = d.and_then(|d| d.fix) {
                    cmds.push(Cmd::RunFix { id, label, arg });
                }
            }
            Hit::DlgNo => {
                self.st.dialog = None;
                self.invalidate_all_dialog();
            }
            Hit::DlgCopy => {
                if let Some(c) = self.st.dialog.as_ref().and_then(|d| d.cmd.clone()) {
                    self.say("Скопировала команду — вставь её в терминал (Ctrl+Shift+V).");
                    cmds.push(Cmd::CopyText(c));
                }
            }
            Hit::SpravkaSave => cmds.push(Cmd::SaveSpravka),
            Hit::SpravkaClose | Hit::SpravkaX => self.close_spravka(),
            _ => {}
        }
        cmds
    }

    fn copy_resume(&mut self, s: &Session) -> Vec<Cmd> {
        let cmd = crate::diag::resume_cmd(s);
        if cmd.is_empty() {
            self.say(if s.id.is_empty() {
                "Эта сессия только началась — у неё ещё нет номера."
            } else {
                "У этой сессии странный номер — копировать команду не стану."
            });
            return Vec::new();
        }
        self.say(&format!("Скопировал! Вставь в терминал: {cmd}"));
        self.log("Команда resume скопирована в буфер обмена.");
        vec![Cmd::CopyText(cmd)]
    }

    fn menu_act(&mut self, a: MenuAct) -> Vec<Cmd> {
        match a {
            MenuAct::TextMode => vec![Cmd::TextMode],
            MenuAct::Quit => vec![Cmd::Quit],
            MenuAct::CheckAll => vec![Cmd::RunChecks],
            MenuAct::Speed => vec![Cmd::RunSpeed],
            MenuAct::Tab(i) => {
                self.set_tab(i);
                Vec::new()
            }
            MenuAct::ToggleLog => {
                self.toggle_log();
                Vec::new()
            }
            MenuAct::Fix(i) => match fixes::GENERAL.get(i as usize) {
                Some((id, label)) => self.open_fix(id, label, None),
                None => Vec::new(),
            },
            MenuAct::Spravka => {
                self.open_spravka();
                Vec::new()
            }
            MenuAct::WidgetShow => vec![Cmd::ShowWidget],
            MenuAct::WidgetStyle(i) => {
                let id = crate::widget_look::STYLES.get(i as usize).map_or("window", |s| s.id());
                crate::prefs::update(|v| v["widget_style"] = serde_json::json!(id));
                vec![Cmd::WidgetChanged]
            }
            MenuAct::WidgetSize(i) => {
                crate::prefs::update(|v| v["widget_size"] = serde_json::json!(if i == 0 { "small" } else { "medium" }));
                let (style, _) = crate::widget_look::chosen();
                if i == 1 && crate::widget_look::fit_size(style, crate::widget_look::Size::Medium) == crate::widget_look::Size::Small {
                    self.say("«Стекло» бывает только маленьким. Средний размер — у «Окна» и «Монитора».");
                }
                vec![Cmd::WidgetChanged]
            }
            MenuAct::Lang(i) => {
                crate::i18n::set(match i {
                    1 => Some(crate::i18n::Lang::Ru),
                    2 => Some(crate::i18n::Lang::En),
                    _ => None,
                });
                // открытая справка свёрстана на прежнем языке — закрываем, остальное перерисуется
                self.st.spravka = None;
                self.invalidate_all();
                vec![Cmd::LangChanged]
            }
            MenuAct::Freq(i) => {
                let id = FREQ_IDS[(i as usize).min(2)];
                crate::prefs::update(|v| v["check_freq"] = serde_json::json!(id));
                self.log(match i {
                    0 => "Проверяю реже — бережёт батарею и трафик.",
                    2 => "Проверяю чаще.",
                    _ => "Проверяю как обычно.",
                });
                Vec::new()
            }
            MenuAct::Licenses => {
                match crate::licenses::open() {
                    Ok((dir, true)) => self.say(&format!("Открыла папку с лицензиями: {}", dir.display())),
                    Ok((dir, false)) => self.say(&format!("Лицензии лежат здесь: {}", dir.display())),
                    Err(e) => self.say(&format!("Не смогла положить лицензии: {e}")),
                }
                Vec::new()
            }
            MenuAct::About => {
                self.info_dialog(
                    &format!("Аптечка {}", env!("CARGO_PKG_VERSION")),
                    "Офлайн-диагностика: сеть и VPN, графика, система и Клод. Один лёгкий файл, без GTK. Текстовый режим: aptechka --diag",
                );
                Vec::new()
            }
        }
    }

    pub fn key_escape(&mut self) {
        if self.st.dialog.is_some() {
            self.st.dialog = None;
            self.invalidate_all_dialog();
        } else if self.st.spravka.is_some() {
            self.close_spravka();
        } else {
            self.close_popup();
        }
    }

    pub fn key_enter(&mut self) -> Vec<Cmd> {
        if self.st.dialog.is_some() {
            return self.activate(Hit::DlgYes);
        }
        if let (Popup::Menu(m), Some(Hit::MenuItem(hm, i))) = (self.st.popup, self.st.hover) {
            if hm == m {
                return self.activate(Hit::MenuItem(m, i));
            }
        }
        Vec::new()
    }

    /// Стрелки в открытом меню: вверх/вниз — пункт, влево/вправо — соседнее меню. true — обработано.
    pub fn key_menu(&mut self, dy: i32, dx: i32) -> bool {
        let Popup::Menu(m) = self.st.popup else { return false };
        let all = menus();
        let before = self.overlay_rect();
        if dx != 0 {
            let n = all.len() as i32;
            let next = ((m as i32 + dx) % n + n) % n;
            self.st.popup = Popup::Menu(next as u8);
            self.st.hover = Some(Hit::MenuItem(next as u8, 0));
        } else {
            let len = all.get(m as usize).map_or(0, |x| x.1.len()) as i32;
            if len == 0 {
                return true;
            }
            let cur = match self.st.hover {
                Some(Hit::MenuItem(hm, i)) if hm == m => i as i32,
                _ => {
                    if dy > 0 {
                        -1
                    } else {
                        len
                    }
                }
            };
            let next = ((cur + dy) % len + len) % len;
            self.st.hover = Some(Hit::MenuItem(m, next as u8));
        }
        self.hover_since = None;
        self.hide_tip();
        let after = self.overlay_rect();
        self.invalidate(before);
        self.invalidate(after);
        let g = self.st.geo();
        self.invalidate(g.menubar);
        true
    }

    /// Alt+буква открывает меню с такой первой буквой (как в Python-версии). true — нажатие обработано.
    pub fn key_alt(&mut self, ch: char) -> bool {
        if self.st.dialog.is_some() || self.st.spravka.is_some() {
            return false;
        }
        // латинская раскладка: та же клавиша, что русская буква (Alt+A = Alt+Ф «Файл»)
        const LAT: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.";
        const CYR: &str = "йцукенгшщзхъфывапролджэячсмитьбю";
        let low = ch.to_lowercase().next().unwrap_or(ch);
        // по-русски меню начинаются с русских букв, по-английски — с латинских: та же клавиша в другой раскладке
        let en = crate::i18n::lang() == crate::i18n::Lang::En;
        let want = if en {
            CYR.chars().position(|c| c == low).and_then(|i| LAT.chars().nth(i)).unwrap_or(low)
        } else {
            LAT.chars().position(|c| c == low).and_then(|i| CYR.chars().nth(i)).unwrap_or(low)
        };
        let found = menus().iter().position(|(name, _)| crate::i18n::tr(name).chars().next().and_then(|c| c.to_lowercase().next()) == Some(want));
        match found {
            Some(i) => {
                self.hide_tip();
                let before = self.overlay_rect();
                self.st.popup = Popup::Menu(i as u8);
                // меню с клавиатуры — выбор с начала, а не с пункта, где он был в прошлый раз
                self.st.hover = None;
                let after = self.overlay_rect();
                self.invalidate(before);
                self.invalidate(after);
                true
            }
            None => false,
        }
    }

    /// Стрелки, PageUp/PageDown, Home/End в справке (в строках; большие числа — до края).
    pub fn key_scroll(&mut self, lines: i32) {
        if self.st.spravka.is_some() {
            let cur = self.st.scroll[2];
            self.set_scroll(2, cur + lines * 40 * self.st.k);
        }
    }

    // ───── справка ─────

    fn open_spravka(&mut self) {
        self.hide_tip();
        self.close_popup();
        self.st.dialog = None;
        let all: Vec<Check> = self.all_checks().cloned().collect();
        let doc = spravka::build(&all, &self.st.sessions);
        let pw = self.st.sp_geo().view.w;
        let page = spravka::layout(&mut self.g.fonts, &doc, pw);
        let n = doc.number.clone();
        self.st.spravka = Some(SpravkaState { doc, page });
        self.st.scroll[2] = 0;
        self.log(&format!("Выдана справка № {n}."));
        self.invalidate_all();
    }

    fn close_spravka(&mut self) {
        if self.st.spravka.take().is_some() {
            self.invalidate_all();
        }
    }

    /// Справка картинкой для «Сохранить как картинку…»: (путь, ширина, высота, RGB). Лист рисуется
    /// в двойном размере, как в Python (page_surface(doc, 2)).
    pub fn spravka_png(&mut self) -> Option<(String, usize, usize, Vec<u8>)> {
        let sp = self.st.spravka.as_ref()?;
        let home = crate::store::home_dir()?.to_string_lossy().into_owned();
        let dir = ["Изображения", "Pictures"].iter().map(|d| format!("{home}/{d}")).find(|d| std::path::Path::new(d).is_dir()).unwrap_or(home);
        let path = format!("{dir}/Справка_{}.png", sp.doc.number);
        let (w, h, rgb) = spravka::render_rgb(&mut self.g.fonts, &sp.doc, spravka::W * 2);
        Some((path, w, h, rgb))
    }

    // ───── подсказки ─────

    // ───── специальные возможности (экранный диктор, управление программой со стороны: AT-SPI / UIA / NSAccessibility)

    /// Всё, на что можно нажать, и главный текст — для дерева специальных возможностей (main.rs, a11y).
    /// id у элемента постоянный, пока он на месте (по виду элемента и номеру), чтобы диктор не терял фокус.
    pub fn a11y_nodes(&mut self) -> Vec<A11yNode> {
        self.ensure_hits();
        let mut out = Vec::new();
        let hits = self.hits.clone();
        for (r, h) in hits {
            let Some((role, name)) = self.a11y_label(h) else { continue };
            let selected = matches!(h, Hit::Tab(i) if i == self.st.tab);
            out.push(A11yNode { id: a11y_id(h), role, name, rect: r, selected, value: String::new() });
        }
        // облачко кота (что сейчас происходит) — текстом
        let g = self.st.geo();
        if !self.st.balloon.is_empty() && self.st.balloon_closed.is_none() {
            out.push(A11yNode {
                id: 1,
                role: A11yRole::Label,
                name: self.st.balloon.clone(),
                rect: g.balloon,
                selected: false,
                value: String::new(),
            });
        }
        out
    }

    /// Нажать элемент по id (действие «нажать» от диктора или со стороны): как щелчок мышью в его середину.
    pub fn a11y_click(&mut self, id: u64) -> Vec<Cmd> {
        self.ensure_hits();
        let Some((r, _)) = self.hits.iter().rev().find(|(_, h)| a11y_id(*h) == id).cloned() else { return Vec::new() };
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        let mut cmds = self.mouse_down(x, y);
        cmds.extend(self.mouse_up(x, y));
        cmds
    }

    /// Роль и название элемента по-русски (то, что прочтёт диктор). None — не показывать (служебное).
    fn a11y_label(&self, h: Hit) -> Option<(A11yRole, String)> {
        let row = |i: u16| self.st.rows.get(i as usize);
        let status_word = |s: &str| match s {
            "ok" => "в порядке",
            "warn" => "внимание",
            "bad" => "беда",
            "checking" => "проверяю",
            "off" => "выключено",
            _ => "для сведения",
        };
        let menus = menus();
        // составные надписи переводим по частям (i18n.rs): склеенную целиком фразу в таблице не найти
        let tr = crate::i18n::t;
        Some(match h {
            Hit::TitleDrag | Hit::Split | Hit::Block | Hit::Dismiss | Hit::Thumb(_) | Hit::Track(_) => return None,
            Hit::ScrollUp(_) => (A11yRole::Button, "Прокрутить вверх".into()),
            Hit::ScrollDown(_) => (A11yRole::Button, "Прокрутить вниз".into()),
            Hit::Menu(i) => (A11yRole::MenuItem, menus.get(i as usize).map(|m| m.0.to_string())?),
            Hit::MenuItem(m, i) => (A11yRole::MenuItem, menus.get(m as usize)?.1.get(i as usize)?.0.clone()),
            Hit::Tab(i) => (A11yRole::Tab, tab_name(i as usize).to_string()),
            Hit::SessionsBtn => (A11yRole::Button, "Сессии ИИ-помощников".into()),
            Hit::SessionCopy(i) => {
                let s = self.st.sessions.get(i as usize)?;
                (A11yRole::Button, format!("Скопировать команду продолжения: {}", if s.title.is_empty() { &s.cwd } else { &s.title }))
            }
            Hit::SessionOpen(i) => {
                let s = self.st.sessions.get(i as usize)?;
                (A11yRole::Button, format!("Открыть сессию: {}", if s.title.is_empty() { &s.cwd } else { &s.title }))
            }
            Hit::Fix(i) => {
                let r = row(i)?;
                (A11yRole::Button, format!("{} — {}", tr(&r.fixes.first()?.label), tr(&r.title)))
            }
            Hit::IgnoreAll => (A11yRole::Button, self.st.ignore_all_label()?.to_string()),
            Hit::Ignore(i) => {
                let r = row(i)?;
                let l = if self.st.ignored.contains(&r.title) { "Не игнорировать" } else { "Игнорировать" };
                (A11yRole::Button, format!("{} — {}", tr(l), tr(&r.title)))
            }
            Hit::Row(i) => {
                let r = row(i)?;
                let mut t = format!("{}: {}, {}", tr(&r.title), tr(status_word(&r.status)), tr(&r.value));
                if !r.detail.is_empty() {
                    t.push_str(". ");
                    t.push_str(&tr(&r.detail));
                }
                (A11yRole::Row, t)
            }
            Hit::Art => (A11yRole::Button, "Кот-доктор (погладить)".into()),
            Hit::DlgYes => (A11yRole::Button, self.st.dialog.as_ref()?.yes_label.clone().unwrap_or_else(|| "Да".into())),
            Hit::DlgNo => (A11yRole::Button, self.st.dialog.as_ref()?.no_label.clone()),
            Hit::DlgCopy => (A11yRole::Button, "Скопировать команду".into()),
            Hit::SpravkaSave => (A11yRole::Button, "Сохранить справку".into()),
            Hit::SpravkaClose | Hit::SpravkaX => (A11yRole::Button, "Закрыть справку".into()),
            Hit::Speed => {
                let v: Vec<String> = self.st.speed_checks.iter().map(|c| format!("{} {}", tr(&c.title), tr(&c.value))).collect();
                (
                    A11yRole::Button,
                    if v.is_empty() {
                        "Измерить скорость интернета".into()
                    } else {
                        format!("Скорость интернета: {}. Измерить заново", v.join(", "))
                    },
                )
            }
            other => (A11yRole::Button, self.tip_text(other)?),
        })
    }

    fn tip_text(&self, h: Hit) -> Option<String> {
        const TOOLS: [&str; N_TABS + 3] = [
            "Проверить всё",
            "Сеть и VPN",
            "ИИ-помощники",
            "Графика",
            "Устройства",
            "Темы и расширения",
            "Система",
            "Показатели: что проверять",
            "Журнал проверок",
            "Полечить…",
        ];
        let row = |i: u16| self.st.rows.get(i as usize);
        match h {
            Hit::Tool(i) => TOOLS.get(i as usize).map(|t| t.to_string()),
            Hit::Tab(i) => Some(tab_name(i as usize).to_string()),
            Hit::Split => Some("Потяните, чтобы поменять высоту таблицы и журнала".into()),
            Hit::TitleBtn(0) => Some("Свернуть".into()),
            Hit::TitleBtn(1) => Some(if self.st.maxed { "Восстановить" } else { "Развернуть" }.into()),
            Hit::TitleBtn(2) => Some("Закрыть".into()),
            Hit::Check => Some("Проверить всё заново".into()),
            Hit::Speed => Some("Скорость интернета. Нажми, чтобы измерить заново".into()),
            Hit::CopyResume => Some("Скопировать команду, чтобы продолжить эту сессию".into()),
            Hit::BalloonClose => Some("Скрыть подсказку".into()),
            Hit::Fix(i) => row(i).and_then(|r| r.fixes.first()).map(|f| f.label.clone()),
            Hit::IgnoreAll => Some(if self.st.problem_titles().is_empty() {
                "Снова считать бедой всё, что игнорируется".into()
            } else {
                "Не считать бедой ни одну из нынешних проблем (до перезапуска приложения)".into()
            }),
            Hit::Ignore(i) => row(i).map(|r| {
                if self.st.ignored.contains(&r.title) {
                    "Снова считать это бедой".into()
                } else {
                    "Игнорировать: не считать это бедой (кот не будет грустить)".into()
                }
            }),
            Hit::Row(i) => row(i).filter(|r| !r.detail.is_empty()).map(|r| r.detail.clone()),
            _ => None,
        }
    }

    fn hide_tip(&mut self) {
        if self.st.tip.is_some() {
            let before = self.overlay_rect();
            self.st.tip = None;
            self.invalidate(before);
        }
    }

    fn show_tip(&mut self, text: String) {
        self.st.tip = Some(Tip { text, x: self.mouse.0, y: self.mouse.1 });
        let r = self.overlay_rect();
        self.invalidate(r);
    }

    /// Раз в кадр: пора ли показать подсказку под неподвижной мышью.
    pub fn tick_tip(&mut self) {
        // реплика на тычок висит 4 с, потом в облачке снова прежнее сообщение
        // (если за это время туда не написали что-то новое)
        if let Some((t0, _, _)) = &self.quip {
            if t0.elapsed() >= QUIP_TIME {
                let (_, prev, shown) = self.quip.take().unwrap();
                if self.st.balloon == shown {
                    self.say(&prev);
                }
            }
        }
        if let Some((_, t0)) = self.st.tab_anim {
            let r = self.st.geo().tabs;
            self.invalidate(r);
            if t0.elapsed() >= TAB_ANIM {
                self.st.tab_anim = None;
            }
        }
        let Some(t0) = self.hover_since else { return };
        if self.st.tip.is_some() || t0.elapsed() < TIP_DELAY {
            return;
        }
        if self.st.popup != Popup::None || self.st.dialog.is_some() || self.st.spravka.is_some() || self.st.drag.is_some() {
            return;
        }
        // мышь ещё над тем же (окно могли сдвинуть или развернуть без её движения)
        self.ensure_hits();
        if self.st.hover.is_some() && self.hit_at(self.mouse.0, self.mouse.1) != self.st.hover {
            self.hover_since = None;
            return;
        }
        if let Some(text) = self.st.hover.and_then(|h| self.tip_text(h)) {
            self.show_tip(text);
        }
    }

    /// Для проверки без окна: сценарий действий (UI_DO).
    pub fn script(&mut self, what: &str) -> Vec<Cmd> {
        self.ensure_hits();
        let mut cmds = Vec::new();
        // fix<N> — кнопка починки строки N, ign<N> — её «Игнорировать»
        if let Some(h) = what
            .strip_prefix("fix")
            .map(|n| Hit::Fix(n.parse().unwrap_or(0)))
            .or_else(|| what.strip_prefix("ign").map(|n| Hit::Ignore(n.parse().unwrap_or(0))))
        {
            if self.hits.iter().any(|(_, x)| *x == h) {
                cmds.extend(self.activate(h));
            }
        } else if let Some(n) = what.strip_prefix("tab") {
            self.set_tab(n.parse().unwrap_or(0));
            self.st.tab_anim = None; // для снимка — сразу конечное положение
        } else if let Some(n) = what.strip_prefix("tip") {
            // tip<tool3|fix0|row1|check|speed|copy|bclose> — навести мышь и сразу показать подсказку
            let hit = match n {
                "check" => Some(Hit::Check),
                "speed" => Some(Hit::Speed),
                "copy" => Some(Hit::CopyResume),
                "bclose" => Some(Hit::BalloonClose),
                _ => n
                    .strip_prefix("tool")
                    .and_then(|v| v.parse().ok())
                    .map(Hit::Tool)
                    .or_else(|| n.strip_prefix("fix").and_then(|v| v.parse().ok()).map(Hit::Fix))
                    .or_else(|| n.strip_prefix("ign").and_then(|v| v.parse().ok()).map(Hit::Ignore))
                    .or_else(|| n.strip_prefix("row").and_then(|v| v.parse().ok()).map(Hit::Row)),
            };
            if let Some(h) = hit {
                let r = self.hit_rect(h);
                self.mouse = (r.x + r.w / 2, r.y + r.h / 2);
                self.st.hover = Some(h);
                if let Some(t) = self.tip_text(h) {
                    self.show_tip(t);
                }
            }
        } else if let Some(id) = what.strip_prefix("runfix:") {
            // runfix:<id> — как нажатие кнопки лечения (для проверки в DRY_RUN); yes — «Да, давай» в окне вопроса
            cmds.extend(self.open_fix(id, id, None));
        } else if what == "yes" {
            cmds.extend(self.activate(Hit::DlgYes));
        } else if what == "spravka" {
            self.open_spravka();
        } else if let Some(n) = what.strip_prefix("spscroll") {
            // spscroll<N> — прокрутить справку на N логических пикселей
            let d: i32 = n.parse().unwrap_or(0);
            let cur = self.st.scroll[2];
            self.set_scroll(2, cur + d * self.st.k);
        } else if let Some(c) = what.strip_prefix("alt:").and_then(|t| t.chars().next()) {
            self.key_alt(c);
        } else if let Some(n) = what.strip_prefix("menu") {
            self.st.popup = Popup::Menu(n.parse().unwrap_or(0));
        } else if what == "sessions" {
            self.st.popup = Popup::Sessions;
        } else if what == "heal" {
            self.st.popup = Popup::Heal;
        } else if what == "about" {
            cmds.extend(self.menu_act(MenuAct::About));
        } else if let Some(n) = what.strip_prefix("hover") {
            // hover:<id> — подсветить строку таблицы
            let i: u16 = n.parse().unwrap_or(0);
            self.st.hover = Some(Hit::Row(i));
        }
        self.invalidate_all();
        cmds
    }
}

pub const OK_CLAUDE_ON: [&str; 2] = ["Сеть в порядке, Клод на связи. Мур~", "Клод думает рядом, а я сторожу сеть."];
pub const OK_CLAUDE_OFF: [&str; 2] =
    ["Сеть в порядке. Клод сейчас отдыхает, но до него можно дозвониться.", "Всё хорошо. Клод выключен — открой терминал, если соскучилась."];
const OK_PHRASES: [&str; 5] = [
    "Всё хорошо, мур! Можно спокойно работать.",
    "Пульс ровный, сеть дышит. Я рядом!",
    "Проверил всё-всё. Ты молодец, что заглянула :3",
    "Всё здорово! Не забудь попить водички.",
    "Сеть в порядке. Мур~",
];

const QUIP_TIME: Duration = Duration::from_secs(4);

/// Случайная фраза «всё хорошо» с учётом того, запущен ли Клод на самом деле.
fn ok_phrase(live: bool) -> String {
    let pool: Vec<&str> = OK_PHRASES.iter().chain(if live { OK_CLAUDE_ON.iter() } else { OK_CLAUDE_OFF.iter() }).copied().collect();
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as usize;
    pool.get(n % pool.len().max(1)).copied().unwrap_or("Мур~").to_string()
}

/// Подпись системы для строки состояния: «Linux 7.0.0», «Windows 11 (26100)», «macOS 15.1».
fn os_label() -> String {
    if cfg!(target_os = "linux") {
        let k = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
        format!("Linux {}", k.trim().split('-').next().unwrap_or(""))
    } else if cfg!(windows) {
        // номер сборки из «cmd /c ver»: «Microsoft Windows [Version 10.0.26100.1742]»
        let (_, v) = crate::diag::run(&["cmd", "/c", "ver"], 5);
        let build: u32 = v.split('.').nth(2).and_then(|b| b.trim_end_matches(']').parse().ok()).unwrap_or(0);
        if build >= 22000 {
            format!("Windows 11 ({build})")
        } else if build > 0 {
            format!("Windows 10 ({build})")
        } else {
            "Windows".into()
        }
    } else if cfg!(target_os = "macos") {
        format!("macOS {}", crate::diag::run(&["sw_vers", "-productVersion"], 5).1.trim())
    } else {
        String::new()
    }
}

fn default_iface() -> String {
    // интерфейс маршрута по умолчанию из /proc/net/route
    std::fs::read_to_string("/proc/net/route")
        .ok()
        .and_then(|t| {
            t.lines().skip(1).find_map(|l| {
                let mut f = l.split_whitespace();
                let iface = f.next()?;
                (f.next()? == "00000000").then(|| iface.to_string())
            })
        })
        .or_else(|| {
            // macOS: «interface: en0» из route -n get default
            cfg!(target_os = "macos")
                .then(|| crate::diag::run(&["route", "-n", "get", "default"], 3).1)
                .and_then(|o| o.lines().find_map(|l| l.trim().strip_prefix("interface: ").map(|s| s.trim().to_string())))
        })
        .unwrap_or_else(|| "—".into())
}

// ───────────────────────────── Рисование ─────────────────────────────

struct P<'a, 'b> {
    c: &'a mut Canvas<'b>,
    g: &'a mut Gfx,
    s: &'a St,
    geo: Geo,
    hits: Option<&'a mut Vec<(Rect, Hit)>>,
}

impl P<'_, '_> {
    fn u(&self, n: i32) -> i32 {
        n * self.s.k
    }
    fn fs(&self, n: i32) -> u16 {
        (n * self.s.k) as u16
    }
    fn painting(&self, r: Rect) -> bool {
        !self.c.clip.intersect(r).is_empty()
    }
    fn hit(&mut self, r: Rect, h: Hit) {
        if let Some(v) = self.hits.as_mut() {
            if !r.is_empty() {
                v.push((r, h));
            }
        }
    }
    fn hover(&self, h: Hit) -> bool {
        self.s.hover == Some(h)
    }
    fn down(&self, h: Hit) -> bool {
        self.s.hover == Some(h) && self.s.pressed == Some(h)
    }
    fn text(&mut self, x: i32, base: i32, s: &str, col: u32, px: u16, bold: bool) {
        self.g.fonts.draw(self.c, x, base, s, col, px, bold);
    }
    /// Строка слева в полосе r (по вертикали по центру).
    fn text_l(&mut self, r: Rect, pad: i32, s: &str, col: u32, px: u16, bold: bool) {
        let base = self.g.fonts.center_base(r.y, r.h, px);
        self.text(r.x + pad, base, s, col, px, bold);
    }
    fn text_c(&mut self, r: Rect, s: &str, col: u32, px: u16, bold: bool) {
        let w = self.g.fonts.width(s, px, bold);
        let base = self.g.fonts.center_base(r.y, r.h, px);
        self.text(r.x + (r.w - w) / 2, base, s, col, px, bold);
    }
    fn width(&mut self, s: &str, px: u16, bold: bool) -> i32 {
        self.g.fonts.width(s, px, bold)
    }
    fn icon(&mut self, name: &'static str, x: i32, y: i32, size: i32) {
        if let Some(pm) = self.g.icons.get(name, size as u32) {
            self.c.blit(pm, x, y);
        }
    }
    fn status_icon(&mut self, st: &str, x: i32, y: i32, size: i32, lamp: bool) {
        if let Some(pm) = self.g.icons.status(st, size as u32, lamp) {
            self.c.blit(pm, x, y);
        }
    }

    /// Кнопка XP с подписью. Возвращает true, если нажата.
    fn button(&mut self, r: Rect, label: &str, h: Hit, enabled: bool, default: bool) {
        if enabled {
            self.hit(r, h);
        }
        let down = enabled && self.down(h);
        let mut face = FACE;
        if enabled && self.hover(h) && !down {
            face = 0xDCD8D0;
        }
        let rr = if default { r.inset(1) } else { r };
        if default {
            self.c.fill(r, BLACK);
        }
        if down {
            self.c.pressed(rr, 0xC8C4BC)
        } else {
            self.c.raised(rr, face)
        }
        let px = self.fs(13);
        let shift = if down { self.s.k } else { 0 };
        let col = if enabled { BLACK } else { SHADOW };
        let tr = Rect::new(rr.x + shift, rr.y + shift, rr.w, rr.h);
        if !enabled {
            let w1 = Rect::new(tr.x + 1, tr.y + 1, tr.w, tr.h);
            self.text_c(w1, label, WHITE, px, false);
        }
        self.text_c(tr, label, col, px, false);
    }

    fn base(&mut self) {
        let g = self.geo.clone();
        let whole = Rect::new(0, 0, self.s.w, self.s.h);
        self.c.fill(whole, FACE);
        self.titlebar(&g);
        self.menubar(&g);
        self.toolbar(&g);
        self.artframe(&g);
        self.claude(&g);
        self.balloon(&g);
        self.tabs(&g);
        self.speed(&g);
        self.table(&g);
        self.log(&g);
        self.statusbar(&g);
        self.size_grip();
    }

    // ── заголовок окна ──
    fn titlebar(&mut self, g: &Geo) {
        let t = g.title;
        self.hit(t, Hit::TitleDrag);
        if !self.painting(t) {
            self.btns(g);
            return;
        }
        self.c.gradient_h(t, &[(0.0, 0x001C9E), (0.35, 0x0039C2), (0.7, 0x0056E6), (1.0, 0x046AFA)]);
        self.c.hline(t.x, t.bottom() - 1, t.w, 0x00238F);
        self.icon("heal", self.u(6), (t.h - self.u(18)) / 2, self.u(18));
        let px = self.fs(15);
        let base = self.g.fonts.center_base(t.y, t.h, px);
        self.text(self.u(30), base + 1, "Аптечка", 0x001060, px, true);
        self.text(self.u(30), base, "Аптечка", WHITE, px, true);
        self.btns(g);
    }

    fn btns(&mut self, g: &Geo) {
        let t = g.title;
        let sz = self.u(22);
        for i in 0..3u8 {
            let r = Rect::new(t.right() - self.u(6) - (3 - i as i32) * (sz + self.u(2)), (t.h - sz) / 2, sz, sz);
            let enabled = true;
            if enabled {
                self.hit(r, Hit::TitleBtn(i));
            }
            if !self.painting(r) {
                continue;
            }
            if enabled && self.down(Hit::TitleBtn(i)) {
                self.c.pressed(r, FACE)
            } else {
                self.c.raised(r, FACE)
            }
            let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
            let col = if enabled { BLACK } else { SHADOW };
            let k = self.s.k;
            match i {
                0 => self.c.fill(Rect::new(cx - 4 * k, cy + 3 * k, 8 * k, 2 * k), col),
                1 if self.s.maxed => {
                    // «восстановить»: два окошка, заднее выглядывает справа сверху
                    self.c.bevel(Rect::new(cx - 2 * k, cy - 5 * k, 8 * k, 6 * k), col, col);
                    self.c.fill(Rect::new(cx - 2 * k, cy - 5 * k, 8 * k, 2 * k), col);
                    self.c.fill(Rect::new(cx - 5 * k, cy - 2 * k, 8 * k, 7 * k), FACE);
                    self.c.bevel(Rect::new(cx - 5 * k, cy - 2 * k, 8 * k, 7 * k), col, col);
                    self.c.fill(Rect::new(cx - 5 * k, cy - 2 * k, 8 * k, 2 * k), col);
                }
                1 => {
                    // как в Python-версии: рамка и толстый верх
                    self.c.bevel(Rect::new(cx - 6 * k, cy - 5 * k, 12 * k, 10 * k), col, col);
                    self.c.fill(Rect::new(cx - 6 * k, cy - 5 * k, 12 * k, 2 * k), col);
                }
                _ => {
                    for d in -4 * k..=4 * k {
                        for w in 0..(k.max(1) + 1) {
                            self.c.put(cx + d + w, cy + d, col);
                            self.c.put(cx + d + w, cy - d, col);
                        }
                    }
                }
            }
        }
    }

    // ── меню ──
    fn menu_rects(&mut self) -> Vec<Rect> {
        let px = self.fs(15);
        let mut x = self.u(4);
        let mut out = Vec::new();
        let bar = self.geo.menubar;
        for (name, _) in menus() {
            let w = self.g.fonts.width(name, px, false) + self.u(24);
            out.push(Rect::new(x, bar.y + self.u(2), w, bar.h - self.u(3)));
            x += w + self.u(2);
        }
        out
    }

    fn menubar(&mut self, g: &Geo) {
        let bar = g.menubar;
        self.c.hline(bar.x, bar.bottom() - 1, bar.w, WHITE);
        let px = self.fs(15);
        for (i, (name, _)) in menus().into_iter().enumerate() {
            let r = self.menu_rects().get(i).copied().unwrap_or_default();
            self.hit(r, Hit::Menu(i as u8));
            if !self.painting(r) {
                continue;
            }
            let open = self.s.popup == Popup::Menu(i as u8);
            if open {
                self.c.bevel(r, SHADOW, WHITE);
            } else if self.hover(Hit::Menu(i as u8)) {
                self.c.bevel(r, WHITE, SHADOW);
            }
            let w = self.width(name, px, false);
            let base = self.g.fonts.center_base(r.y, r.h, px);
            let x = r.x + (r.w - w) / 2;
            self.text(x, base, name, BLACK, px, false);
            // подчёркнута первая буква (как в Python-версии)
            let first = name.chars().next().map(|c| c.to_string()).unwrap_or_default();
            let fw = self.width(&first, px, false);
            self.c.hline(x, base + self.u(2), fw, BLACK);
        }
    }

    // ── панель инструментов ──
    fn toolbar(&mut self, g: &Geo) {
        for (i, name) in TOOL_ICONS.iter().enumerate() {
            let r = g.tools[i];
            let h = Hit::Tool(i as u8);
            self.hit(r, h);
            if !self.painting(r) {
                continue;
            }
            // как в Python-версии: «залипает» только кнопка журнала (вкл/выкл)
            let active = i == TOOL_LOG as usize && self.s.log_vis;
            let down = self.down(h) || (i == TOOL_HEAL as usize && self.s.popup == Popup::Heal) || active;
            let face = if self.hover(h) && !down { 0xDCD8D0 } else { FACE };
            if down {
                self.c.pressed(r, if active { 0xCBC8C0 } else { 0xC8C4BC })
            } else {
                self.c.raised(r, face)
            }
            let sz = self.u(32);
            let off = if down { self.s.k } else { 0 };
            self.icon(name, r.x + (r.w - sz) / 2 + off, r.y + (r.h - sz) / 2 + off, sz);
        }
    }

    fn artframe(&mut self, g: &Geo) {
        self.hit(g.art, Hit::Art);
        if self.painting(g.frame) {
            self.c.sunken(g.frame, BLACK);
        }
    }

    // ── Claude ──
    fn group_box(&mut self, r: Rect, title: &str) {
        let top = r.y + self.u(8);
        let b = Rect::new(r.x, top, r.w, r.h - self.u(8));
        self.c.bevel(b, SHADOW, WHITE);
        self.c.bevel(Rect::new(b.x + 1, b.y + 1, b.w, b.h), WHITE, SHADOW);
        let px = self.fs(13);
        let w = self.width(title, px, false) + self.u(8);
        self.c.fill(Rect::new(r.x + self.u(8), r.y, w, self.u(16)), FACE);
        self.text(r.x + self.u(12), r.y + self.u(13), title, BLACK, px, false);
    }

    fn claude(&mut self, g: &Geo) {
        let r = g.claude;
        if !self.painting(r) {
            self.claude_hits(g);
            return;
        }
        self.group_box(r, "ИИ-агенты в терминале");
        let live: Vec<&Session> = self.s.sessions.iter().filter(|s| s.live).collect();
        let (lbl, sub, led) = if !live.is_empty() {
            let busy = live.iter().filter(|s| s.status == "busy").count();
            // кто включён: «Claude», «Claude, Codex»…
            let mut who: Vec<&str> = Vec::new();
            for s in &live {
                let n = crate::diag::agent_name(s.agent);
                if !who.contains(&n) {
                    who.push(n);
                }
            }
            (
                format!("{} · {} в терминале · {}", who.join(", "), live.len(), if busy > 0 { "думает" } else { "ждёт тебя" }),
                format!("Сессия: {} {}", crate::diag::agent_name(live[0].agent), live[0].id),
                "ok",
            )
        } else if let Some(s) = self.s.sessions.first() {
            ("Агенты выключены".to_string(), format!("Последняя: {} {}", crate::diag::agent_name(s.agent), s.id), "bad")
        } else {
            ("Агенты выключены".to_string(), "сессий пока нет".to_string(), "bad")
        };
        self.status_icon(led, r.x + self.u(10), r.y + self.u(28), self.u(16), true);
        let (copy_r, sess_r) = self.claude_btn_rects(g);
        let tx = r.x + self.u(34);
        let maxw = copy_r.x - tx - self.u(8);
        let px = self.fs(13);
        let l1 = self.g.fonts.fit(&lbl, maxw, px, false);
        let l2 = self.g.fonts.fit(&sub, maxw, px, false);
        self.text(tx, r.y + self.u(32), &l1, BLACK, px, false);
        self.text(tx, r.y + self.u(50), &l2, BLACK, px, false);
        let have = self.s.main_session().is_some();
        self.button(copy_r, "Копировать resume", Hit::CopyResume, have, false);
        self.button(sess_r, "Сессии", Hit::SessionsBtn, true, false);
        let ax = sess_r.right() - self.u(15);
        self.c.arrow(ax, sess_r.y + sess_r.h / 2, self.u(4), 1, BLACK);
    }

    fn claude_btn_rects(&mut self, g: &Geo) -> (Rect, Rect) {
        let r = g.claude;
        let px = self.fs(13);
        let h = self.u(24);
        let y = r.y + self.u(26);
        let w2 = self.width("Сессии", px, false) + self.u(46);
        let w1 = self.width("Копировать resume", px, false) + self.u(24);
        let sess = Rect::new(r.right() - self.u(8) - w2, y, w2, h);
        let copy = Rect::new(sess.x - self.u(6) - w1, y, w1, h);
        (copy, sess)
    }

    fn claude_hits(&mut self, g: &Geo) {
        let (c, s) = self.claude_btn_rects(g);
        if self.s.main_session().is_some() {
            self.hit(c, Hit::CopyResume);
        }
        self.hit(s, Hit::SessionsBtn);
    }

    // ── облачко ──
    fn balloon(&mut self, g: &Geo) {
        let r = g.balloon;
        if r.is_empty() {
            return;
        }
        let av = self.u(52);
        if self.painting(r) {
            if let Some(pm) = self.s.avatar.as_ref() {
                self.c.blit(pm, r.x, r.y + (r.h - av) / 2);
            }
        }
        let b = Rect::new(r.x + av + self.u(8), r.y, r.w - av - self.u(8), r.h);
        let close = Rect::new(b.right() - self.u(22), b.y + self.u(5), self.u(17), self.u(17));
        self.hit(close, Hit::BalloonClose);
        if !self.painting(r) {
            return;
        }
        self.c.fill(b, TIP);
        self.c.bevel(b, BLACK, BLACK);
        for (dx, dy) in [(0, 0), (b.w - 1, 0), (0, b.h - 1), (b.w - 1, b.h - 1)] {
            self.c.put(b.x + dx, b.y + dy, FACE); // скруглённые углы
        }
        let px = self.fs(13);
        let lines = self.g.fonts.wrap(&self.s.balloon, b.w - self.u(46), px, false, 3);
        let lh = self.u(16);
        let total = lh * lines.len() as i32;
        let mut y = b.y + (b.h - total) / 2;
        for l in &lines {
            let base = self.g.fonts.center_base(y, lh, px);
            self.text(b.x + self.u(10), base, l, BLACK, px, false);
            y += lh;
        }
        let down = self.down(Hit::BalloonClose);
        if down {
            self.c.pressed(close, FACE)
        } else {
            self.c.raised(close, if self.hover(Hit::BalloonClose) { 0xDCD8D0 } else { FACE })
        }
        self.text_c(close, "×", BLACK, self.fs(13), false);
    }

    // ── вкладки ──
    /// Насколько вкладка i «поднята» (0 — лежит, 1 — выбрана), с анимацией переключения.
    fn tab_up(&self, i: usize) -> f32 {
        let sel = self.s.tab as usize == i;
        let Some((prev, t0)) = self.s.tab_anim else { return if sel { 1.0 } else { 0.0 } };
        let x = (t0.elapsed().as_secs_f32() / TAB_ANIM.as_secs_f32()).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - x) * (1.0 - x) * (1.0 - x); // плавно тормозит в конце
        if sel {
            e
        } else if prev as usize == i {
            1.0 - e
        } else {
            0.0
        }
    }

    /// Вкладки как в Python-версии (gui_xp.py): лежащая — ниже на 8, темнее (#c9c5bd),
    /// поднятая — выше, шире, жирный текст, светлая и «открыта» в панель под ней.
    /// При переключении новая плавно поднимается, старая опускается.
    fn tabs(&mut self, g: &Geo) {
        let r = g.tabs;
        let px = self.fs(14);
        let counts: Vec<usize> = (0..N_TABS)
            .map(|i| match CATS.get(i) {
                Some(cat) => self
                    .s
                    .checks
                    .iter()
                    .chain(self.s.speed_checks.iter())
                    .filter(|c| c.key == cat.key() && unit_on(c.unit) && matches!(c.status.as_str(), "bad" | "warn"))
                    .count(),
                None => 0,
            })
            .collect();
        let lerp = |a: i32, b: i32, t: f32| a + ((b - a) as f32 * t).round() as i32;
        let mut rects = Vec::new();
        let mut x = r.x;
        // вкладок много: подпись только у выбранной, у остальных значок (и число проблем, если есть);
        // название — во всплывающей подсказке
        for (i, &count) in counts.iter().enumerate().take(N_TABS) {
            let up = self.tab_up(i);
            let label = if up > 0.5 {
                if count > 0 {
                    format!("{} ({})", crate::i18n::tr(tab_name(i)), count)
                } else {
                    tab_name(i).to_string()
                }
            } else if count > 0 {
                format!("({})", count)
            } else {
                String::new()
            };
            let pad = lerp(self.u(8), self.u(12), up);
            let isz = self.u(30);
            let tw = if label.is_empty() { 0 } else { self.u(6) + self.width(&label, px, up > 0.5) + 2 };
            let w = pad * 2 + isz + tw;
            let top = r.y + lerp(self.u(8), 0, up);
            let tr = Rect::new(x, top, w, r.bottom() - top);
            rects.push((tr, up, label, isz, pad));
            self.hit(tr, Hit::Tab(i as u8));
            x += w;
        }
        if !self.painting(r) {
            return;
        }
        // основание: белая линия панели, под выбранной — разрыв
        self.c.hline(r.x, r.bottom() - 1, r.w, WHITE);
        // сначала лежащие, потом поднятые — поднятая перекрывает соседей
        let mut order: Vec<usize> = (0..N_TABS).collect();
        order.sort_by(|a, b| rects[*a].1.partial_cmp(&rects[*b].1).unwrap_or(std::cmp::Ordering::Equal));
        for i in order {
            let (tr, up, ref label, isz, pad) = rects[i];
            let hover = up < 0.5 && self.hover(Hit::Tab(i as u8));
            let low = if hover { 0xDCD8D0 } else { 0xC9C5BD };
            let face = mix(low, FACE, up);
            self.c.fill(tr, face);
            // рамка: белый верх-лево, тёмный (#404040) право, внутри светлый (#dfdfdf) отблеск
            self.c.hline(tr.x, tr.y, tr.w, WHITE);
            self.c.vline(tr.x, tr.y, tr.h, WHITE);
            self.c.vline(tr.right() - 1, tr.y, tr.h, DARK);
            self.c.hline(tr.x + 1, tr.y + 1, tr.w - 2, LIGHT);
            self.c.vline(tr.x + 1, tr.y + 1, tr.h - 1, LIGHT);
            if up > 0.5 {
                self.c.hline(tr.x + 1, tr.bottom() - 1, tr.w - 2, face); // открыта вниз
            }
            let iy = tr.y + (tr.h - isz) / 2;
            self.icon(tab_icon(i), tr.x + pad, iy, isz);
            let base = self.g.fonts.center_base(tr.y, tr.h, px);
            self.text(tr.x + pad + isz + self.u(6), base, label, BLACK, px, up > 0.5);
        }
    }

    // ── скорость ──
    fn speed(&mut self, g: &Geo) {
        let r = g.speed;
        let bw = self.u(130);
        let strip = Rect::new(r.x, r.y, r.w - bw - self.u(6), r.h);
        let btn = Rect::new(r.right() - bw, r.y, bw, r.h);
        self.hit(strip, Hit::Speed);
        self.hit(btn, Hit::Check);
        if !self.painting(r) {
            return;
        }
        self.c.sunken(strip, if self.hover(Hit::Speed) { 0xF4F4F0 } else { FACE });
        let f = |v: Option<f64>| v.map(|x| format!("{x:.0}")).unwrap_or("—".into());
        let v = self.s.speed_vals;
        let mut line = format!("↓ {}   ↑ {} Мбит/с · пинг {} мс", f(v[0]), f(v[1]), f(v[2]));
        let px = self.fs(13);
        if let Some(l) = self.s.speed_live {
            let name = match l.phase {
                SpeedPhase::Ping => "пинг",
                SpeedPhase::Down => "качаю",
                SpeedPhase::Up => "отдаю",
            };
            line = format!("{line}  ({name}…)");
            // зелёные «кубики» прогресса: три этапа подряд
            let idx = match l.phase {
                SpeedPhase::Ping => 0.0,
                SpeedPhase::Down => 1.0,
                SpeedPhase::Up => 2.0,
            };
            let total = ((idx + l.fraction.clamp(0.0, 1.0)) / 3.0).clamp(0.0, 1.0);
            let pw = self.u(120);
            let pr = Rect::new(strip.right() - pw - self.u(8), strip.y + self.u(6), pw, strip.h - self.u(12));
            self.c.sunken(pr, WHITE);
            let inner = pr.inset(self.u(3));
            let (bwid, gap) = (self.u(7), self.u(2));
            let n = ((inner.w + gap) / (bwid + gap)) as f64;
            let fill = (n * total).round() as i32;
            for b in 0..fill {
                self.c.fill(Rect::new(inner.x + b * (bwid + gap), inner.y, bwid, inner.h), 0x2FB62F);
            }
        }
        self.text_l(strip, self.u(10), &line, BLACK, px, false);
        let down = self.down(Hit::Check);
        if down {
            self.c.pressed(btn, 0xC8C4BC)
        } else {
            self.c.raised(btn, if self.hover(Hit::Check) { 0xDCD8D0 } else { FACE })
        }
        let isz = self.u(20);
        let sh = if down { self.s.k } else { 0 };
        self.icon("refresh", btn.x + self.u(10) + sh, btn.y + (btn.h - isz) / 2 + sh, isz);
        let tr = Rect::new(btn.x + self.u(26) + sh, btn.y + sh, btn.w - self.u(26), btn.h);
        self.text_c(tr, "Проверить", BLACK, px, false);
    }

    // ── полоса прокрутки ──
    fn scrollbar(&mut self, r: Rect, id: u8, content: i32, view: i32, off: i32) {
        let bw = r.w;
        let up = Rect::new(r.x, r.y, bw, bw);
        let dn = Rect::new(r.x, r.bottom() - bw, bw, bw);
        let track = Rect::new(r.x, r.y + bw, bw, r.h - 2 * bw);
        self.hit(up, Hit::ScrollUp(id));
        self.hit(dn, Hit::ScrollDown(id));
        self.hit(track, Hit::Track(id));
        let tl = track.h;
        let thumb_h = if content <= view { tl } else { (tl * view / content.max(1)).max(bw).min(tl) };
        let max = (content - view).max(1);
        let ty = track.y + if content <= view { 0 } else { (tl - thumb_h) * off / max };
        let thumb = Rect::new(track.x, ty, bw, thumb_h);
        if content > view {
            self.hit(thumb, Hit::Thumb(id));
        }
        if !self.painting(r) {
            return;
        }
        self.c.fill(track, 0xE9E7E1);
        if content > view {
            self.c.raised(thumb, FACE);
        }
        for (b, dir, h) in [(up, 0u8, Hit::ScrollUp(id)), (dn, 1u8, Hit::ScrollDown(id))] {
            if self.down(h) {
                self.c.pressed(b, FACE)
            } else {
                self.c.raised(b, FACE)
            }
            self.c.arrow(b.x + b.w / 2, b.y + b.h / 2, self.u(3), dir, BLACK);
        }
    }

    // ── таблица ──
    fn table(&mut self, g: &Geo) {
        let r = g.table;
        let k = self.s.k;
        if self.painting(r) {
            self.c.sunken(r, WHITE);
        }
        let inner = r.inset(2 * k);
        let sbw = 17 * k;
        let head = Rect::new(inner.x, inner.y, inner.w, 22 * k);
        let cols = self.columns(inner.w - sbw);
        let titles = if self.s.tab == SETTINGS_TAB {
            ["Показатель", "Что проверяет", ""]
        } else {
            ["Проверка", "Статус", "Действие"]
        };
        // «Игнорировать всё» — в заголовке колонки «Действие» (действие сразу над всеми строками), когда есть
        // что игнорировать или что вернуть; иначе там просто «Действие»
        let ign_label = if self.s.tab == SETTINGS_TAB { None } else { self.s.ignore_all_label() };
        let ign = Rect::new(inner.x + cols[2].0, head.y, cols[2].1, head.h);
        if ign_label.is_some() {
            self.hit(ign, Hit::IgnoreAll);
        }
        if self.painting(head) {
            let px = self.fs(13);
            for (i, t) in titles.iter().enumerate() {
                let cr = Rect::new(inner.x + cols[i].0, head.y, cols[i].1, head.h);
                if i == 2 {
                    if let Some(l) = ign_label {
                        let l = self.g.fonts.fit(l, cr.w - 10 * k, px, false);
                        self.button(cr, &l, Hit::IgnoreAll, true, false);
                        continue;
                    }
                }
                self.c.raised(cr, FACE);
                self.text_l(cr, 8 * k, t, BLACK, px, false);
            }
            self.c.raised(Rect::new(inner.x + cols[2].0 + cols[2].1, head.y, sbw, head.h), FACE);
        }
        let view = Rect::new(inner.x, head.bottom(), inner.w - sbw, inner.bottom() - head.bottom());
        let off = self.s.scroll[0];
        let rh = 26 * k;
        let px = self.fs(13);
        let old = self.c.clip;
        self.c.clip = old.intersect(view);
        if self.s.rows.is_empty() {
            let msg = if self.s.checking { "Проверяю…" } else { "Нет данных" };
            self.text_l(Rect::new(view.x, view.y, view.w, rh), 8 * k, msg, BLACK, px, false);
        }
        for (i, row) in self.s.rows.iter().enumerate() {
            let rr = Rect::new(view.x, view.y + i as i32 * rh - off, view.w, rh);
            let vis = rr.intersect(view);
            if vis.is_empty() {
                continue;
            }
            self.hit(vis, Hit::Row(i as u16));
            // кнопка — у красных и жёлтых строк; и всегда — у действий «по просьбе» (поиск и уборка копий, эталон
            // тем, закрыть ключи от других): их строки спокойные (сведения или «в порядке»), а без кнопки
            // действие было бы недоступно
            const ON_DEMAND: [&str; 5] = ["dupes_scan", "dupes_trash", "snapshot", "footprint_lock", "footprint_history"];
            let show_fix = !row.fixes.is_empty()
                && (matches!(row.status.as_str(), "bad" | "warn")
                    || row.title.starts_with("Канал")
                    || row.key == "settings"
                    || row.fixes.first().is_some_and(|f| ON_DEMAND.contains(&f.id.as_str())));
            let mut btn = Rect::new(view.x + cols[2].0 + 4 * k, rr.y + 2 * k, cols[2].1 - 8 * k, rh - 4 * k);
            // «Игнорировать» — у красных и жёлтых строк; у игнорируемой — «Не игнорировать» (вместо починки)
            let ignored = self.s.tab != SETTINGS_TAB && self.s.ignored.contains(&row.title);
            let show_fix = show_fix && !ignored;
            let can_ignore = self.s.tab != SETTINGS_TAB && (ignored || matches!(row.status.as_str(), "bad" | "warn"));
            // рядом с починкой — маленькая квадратная «×», без починки — кнопка во всю ширину
            let ign = if can_ignore && show_fix {
                let side = btn.h;
                btn.w -= side + 4 * k;
                Rect::new(btn.right() + 4 * k, btn.y, side, side)
            } else {
                btn
            };
            if show_fix {
                self.hit(btn.intersect(view), Hit::Fix(i as u16));
            }
            if can_ignore {
                self.hit(ign.intersect(view), Hit::Ignore(i as u16));
            }
            if !self.painting(vis) {
                continue;
            }
            let hov = self.hover(Hit::Row(i as u16)) || self.hover(Hit::Fix(i as u16)) || self.hover(Hit::Ignore(i as u16));
            if hov {
                self.c.fill(rr, 0xEEF3FB);
            }
            self.c.hline(rr.x, rr.bottom() - 1, rr.w, 0xECEBE6);
            self.status_icon(&row.status, rr.x + 6 * k, rr.y + (rh - 18 * k) / 2, 18 * k, false);
            let t = self.g.fonts.fit(&row.title, cols[0].1 - 36 * k, px, false);
            self.text_l(Rect::new(rr.x + 32 * k, rr.y, rr.w, rh), 0, &t, BLACK, px, false);
            let v = self.g.fonts.fit(&row.value, cols[1].1 - 14 * k, px, false);
            self.text_l(Rect::new(rr.x + cols[1].0, rr.y, cols[1].1, rh), 8 * k, &v, status_color(&row.status), px, false);
            if show_fix {
                let label = row.fixes.first().map(|f| f.label.clone()).unwrap_or_default();
                let label = self.g.fonts.fit(&label, btn.w - 12 * k, px, false);
                self.button(btn, &label, Hit::Fix(i as u16), true, false);
            }
            if can_ignore {
                let label = if ignored {
                    "Не игнорировать"
                } else if show_fix {
                    "×"
                } else {
                    "Игнорировать"
                };
                let label = self.g.fonts.fit(label, ign.w - 6 * k, px, false);
                self.button(ign, &label, Hit::Ignore(i as u16), true, false);
            }
            if !show_fix && !can_ignore {
                self.text_l(Rect::new(rr.x + cols[2].0, rr.y, cols[2].1, rh), 8 * k, "—", BLACK, px, false);
            }
        }
        self.c.clip = old;
        let content = self.s.table_rows() * rh;
        let bar = Rect::new(inner.right() - sbw, head.bottom(), sbw, inner.bottom() - head.bottom());
        self.scrollbar(bar, 0, content, view.h, off);
    }

    /// Колонки таблицы: (сдвиг, ширина) для «Проверка», «Статус», «Действие».
    fn columns(&self, total: i32) -> [(i32, i32); 3] {
        let k = self.s.k;
        let c0 = (230 * k).min(total * 40 / 100);
        let c2 = (170 * k).min(total * 32 / 100);
        [(0, c0), (c0, total - c0 - c2), (total - c2, c2)]
    }

    // ── журнал ──
    fn log(&mut self, g: &Geo) {
        let r = g.log;
        if r.is_empty() {
            return;
        }
        // граница с таблицей (как разделитель в Python: белый верх, серый низ)
        let sp = Rect::new(r.x, g.table.bottom(), r.w, r.y - g.table.bottom());
        self.hit(sp, Hit::Split);
        if self.painting(sp) {
            let hot = self.hover(Hit::Split) || self.s.split_drag.is_some();
            self.c.fill(sp, if hot { 0xC8C4BC } else { FACE });
            self.c.hline(sp.x, sp.y + self.s.k, sp.w, WHITE);
            self.c.hline(sp.x, sp.bottom() - 1 - self.s.k, sp.w, SHADOW);
            // «насечка» по центру, чтобы было видно, что тянется
            let cx = sp.x + sp.w / 2;
            for d in [-6, -3, 0, 3, 6] {
                self.c.fill(Rect::new(cx + d * self.s.k, sp.y + sp.h / 2 - self.s.k / 2, self.s.k.max(1), self.s.k.max(1)), SHADOW);
            }
        }
        let k = self.s.k;
        if self.painting(r) {
            self.c.sunken(r, WHITE);
        }
        let inner = r.inset(2 * k);
        let sbw = 17 * k;
        let view = Rect::new(inner.x, inner.y, inner.w - sbw, inner.h);
        let lh = 16 * k;
        let off = self.s.scroll[1];
        let old = self.c.clip;
        self.c.clip = old.intersect(view);
        if self.painting(view) {
            let px = self.fs(12);
            let first = (off / lh).max(0) as usize;
            let n = (view.h / lh + 2) as usize;
            for (j, line) in self.s.log.iter().enumerate().skip(first).take(n) {
                let y = view.y + 2 * k + j as i32 * lh - off;
                let l = self.g.fonts.fit(line, view.w - 10 * k, px, false);
                let base = self.g.fonts.center_base(y, lh, px);
                self.text(view.x + 6 * k, base, &l, BLACK, px, false);
            }
        }
        self.c.clip = old;
        let content = self.s.log.len() as i32 * lh + 4 * k;
        self.scrollbar(Rect::new(inner.right() - sbw, inner.y, sbw, inner.h), 1, content, view.h, off);
    }

    // ── строка состояния ──
    fn statusbar(&mut self, g: &Geo) {
        let px = self.fs(13);
        let real: Vec<&Check> = self.s.checks.iter().chain(self.s.speed_checks.iter()).filter(|c| c.status != "info").collect();
        let ok = real.iter().filter(|c| c.status == "ok").count();
        let bad = real.iter().filter(|c| c.status == "bad").count();
        let warn = real.iter().filter(|c| c.status == "warn").count();
        let unk = self.s.checks.iter().chain(self.s.speed_checks.iter()).filter(|c| c.unchecked).count();
        let s1 = if self.s.checking {
            "Проверяю…".to_string()
        } else if unk > 0 {
            format!("Проверка завершена · в порядке: {ok} из {} · проблем: {bad} · предупреждений: {warn} · не проверено: {unk}", real.len())
        } else {
            format!("Проверка завершена · в порядке: {ok} из {} · проблем: {bad} · предупреждений: {warn}", real.len())
        };
        // проверки идут по расписанию (sched.rs): сеть часто, остальное реже — «следующая через» больше не одна
        let s2 = match self.s.last_check {
            Some(t) => {
                let tm = localtime(t);
                format!("Обновлено в {:02}:{:02}:{:02} · сеть проверяю постоянно, остальное реже", tm.hour, tm.min, tm.sec)
            }
            None => "Первая проверка…".to_string(),
        };
        let texts = [s1, s2, self.s.status3.clone()];
        for (i, t) in texts.iter().enumerate() {
            let r = g.status[i];
            if !self.painting(r) {
                continue;
            }
            // как .sunken в Python: серый/белый снаружи, тёмный/светлый внутри
            self.c.bevel(r, SHADOW, WHITE);
            self.c.bevel(r.inset(1), DARK, LIGHT);
            let fit = self.g.fonts.fit(t, r.w - 14 * self.s.k, px, false);
            self.text_l(r, 7 * self.s.k, &fit, BLACK, px, false);
        }
    }

    /// Уголок для изменения размера (как в Windows XP): точки лесенкой в правом нижнем углу.
    fn size_grip(&mut self) {
        let k = self.s.k;
        let r = Rect::new(self.s.w - 16 * k, self.s.h - 16 * k, 16 * k, 16 * k);
        if !self.painting(r) {
            return;
        }
        self.c.fill(r, FACE);
        for i in 0..3 {
            for j in 0..=i {
                let (x, y) = (r.right() - (3 + 4 * j) * k - 2 * k, r.bottom() - (3 + 4 * (i - j)) * k - 2 * k);
                self.c.fill(Rect::new(x + k, y + k, 2 * k, 2 * k), WHITE);
                self.c.fill(Rect::new(x, y, 2 * k, 2 * k), SHADOW);
            }
        }
    }

    // ── всплывающее поверх всего ──
    fn overlay(&mut self) {
        let is_open = self.s.popup != Popup::None;
        if is_open || self.s.dialog.is_some() || self.s.spravka.is_some() {
            // подложка: клик мимо закрывает меню, а при окне вопроса блокирует всё остальное
            let whole = Rect::new(0, 0, self.s.w, self.s.h);
            if self.s.dialog.is_some() || self.s.spravka.is_some() {
                self.hit(whole, Hit::Block);
            } else {
                self.hit(whole, Hit::Dismiss);
                // строка меню остаётся живой, пока меню открыто
                let bar = self.geo.menubar;
                for (i, r) in self.menu_rects().into_iter().enumerate() {
                    self.hit(r.intersect(bar), Hit::Menu(i as u8));
                }
                let g = self.geo.clone();
                self.claude_hits(&g);
                let t6 = g.tools[TOOL_HEAL as usize];
                self.hit(t6, Hit::Tool(TOOL_HEAL));
            }
        }
        self.popup();
        self.spravka();
        self.dialog();
        self.tooltip();
    }

    /// Окно «Справка от Мяудоктора»: лист бумаги с прокруткой, «Сохранить как картинку…», «Закрыть».
    fn spravka(&mut self) {
        let s = self.s;
        let Some(sp) = s.spravka.as_ref() else { return };
        let (k, g) = (s.k, s.sp_geo());
        let off = s.scroll[2];
        if self.painting(g.win.inset(-4 * k)) {
            self.c.fill(Rect::new(g.win.x + 3 * k, g.win.y + 3 * k, g.win.w, g.win.h), 0x707070);
            self.c.raised(g.win, FACE);
            self.c.gradient_h(g.title, &[(0.0, 0x0A246A), (1.0, 0xA6CAF0)]);
            let ttl = format!("Справка № {} — {}", sp.doc.number, spravka::DOCTOR);
            self.text_l(Rect::new(g.title.x, g.title.y, g.title.w - 24 * k, g.title.h), 6 * k, &ttl, WHITE, self.fs(13), true);
            self.c.sunken(g.frame, 0x9C9A94);
            let old = self.c.clip;
            self.c.clip = old.intersect(g.view);
            spravka::draw(&sp.page, self.c, &mut self.g.fonts, g.view.x, g.view.y - off);
            self.c.clip = old;
        }
        // кнопки и полоса прокрутки сами записывают области нажатий (и при пустом холсте)
        self.scrollbar(g.bar, 2, sp.page.h, g.view.h, off);
        self.button(g.close, "×", Hit::SpravkaX, true, false);
        self.button(g.save, "Сохранить как картинку…", Hit::SpravkaSave, true, false);
        self.button(g.done, "Закрыть", Hit::SpravkaClose, true, true);
    }

    /// Размер и положение подсказки (None — подсказки нет).
    fn tip_geom(&mut self) -> Option<(Rect, Vec<String>)> {
        let s = self.s;
        let tip = s.tip.as_ref()?;
        let k = s.k;
        let px = self.fs(12);
        let lines = self.g.fonts.wrap(&tip.text, 340 * k, px, false, 8);
        let mut w = 0;
        for l in &lines {
            w = w.max(self.g.fonts.width(l, px, false));
        }
        let (w, h) = (w + 12 * k, lines.len() as i32 * 16 * k + 8 * k);
        let x = (tip.x + 12 * k).min(s.w - w - 2 * k).max(0);
        let below = tip.y + 22 * k;
        let y = if below + h > s.h { (tip.y - h - 4 * k).max(0) } else { below };
        Some((Rect::new(x, y, w, h), lines))
    }

    fn tooltip(&mut self) {
        let Some((r, lines)) = self.tip_geom() else { return };
        if !self.painting(r) {
            return;
        }
        self.c.fill(r, TIP);
        self.c.bevel(r, BLACK, BLACK);
        let (k, px) = (self.s.k, self.fs(12));
        for (i, l) in lines.iter().enumerate() {
            let line = Rect::new(r.x + 6 * k, r.y + 4 * k + i as i32 * 16 * k, r.w, 16 * k);
            self.text_l(line, 0, l, BLACK, px, false);
        }
    }

    fn overlay_rect(&mut self) -> Rect {
        let mut r = self.popup_rect();
        if self.s.dialog.is_some() {
            r = r.union(self.dialog_rect());
        }
        if self.s.spravka.is_some() {
            r = r.union(self.s.sp_geo().win.inset(-4 * self.s.k));
        }
        if let Some((tr, _)) = self.tip_geom() {
            r = r.union(tr);
        }
        r
    }

    fn popup_rect(&mut self) -> Rect {
        let k = self.s.k;
        let px = self.fs(14);
        match self.s.popup {
            Popup::None => Rect::default(),
            Popup::Menu(i) => {
                let items = menus();
                let Some((_, list)) = items.get(i as usize) else { return Rect::default() };
                let mut w = 0;
                for (l, _) in list {
                    w = w.max(self.width(l, px, false));
                }
                let mr = self.menu_rects().get(i as usize).copied().unwrap_or_default();
                Rect::new(mr.x, mr.bottom(), w + 44 * k, list.len() as i32 * 24 * k + 6 * k)
            }
            Popup::Heal => {
                let mut w = 0;
                for (_, l) in fixes::GENERAL {
                    w = w.max(self.width(l, px, false));
                }
                let h = fixes::GENERAL.len() as i32 * 24 * k + 6 * k;
                let t = self.geo.tools[TOOL_HEAL as usize];
                Rect::new(t.right() + 2 * k, t.bottom() - h, w + 44 * k, h)
            }
            Popup::Sessions => {
                let (_, sr) = self.claude_btn_rects(&self.geo.clone());
                let n = self.s.sessions.len().max(1) as i32;
                let w = (500 * k).min(self.s.w - 8 * k);
                let x = (sr.right() - w).max(4 * k);
                Rect::new(x, sr.bottom() + 2 * k, w, n * 40 * k + 8 * k)
            }
        }
    }

    fn popup(&mut self) {
        let r = self.popup_rect();
        if r.is_empty() || !self.painting(r) && self.hits.is_none() {
            return;
        }
        let k = self.s.k;
        self.c.fill(Rect::new(r.x + 2 * k, r.y + 2 * k, r.w, r.h), 0x707070); // тень
        self.c.raised(r, FACE);
        let px = self.fs(14);
        match self.s.popup {
            Popup::Menu(m) => {
                let list = menus().into_iter().nth(m as usize).map(|x| x.1).unwrap_or_default();
                for (i, (label, act)) in list.iter().enumerate() {
                    let ir = Rect::new(r.x + 3 * k, r.y + 3 * k + i as i32 * 24 * k, r.w - 6 * k, 24 * k);
                    let h = Hit::MenuItem(m, i as u8);
                    self.hit(ir, h);
                    let hov = self.hover(h);
                    if hov {
                        self.c.fill(ir, SELECT);
                    }
                    let (wst, wsz) = crate::widget_look::chosen();
                    let mark = matches!(act, MenuAct::ToggleLog) && self.s.log_vis
                        || matches!(act, MenuAct::Tab(t) if *t == self.s.tab)
                        || matches!(act, MenuAct::WidgetStyle(i) if crate::widget_look::STYLES.get(*i as usize) == Some(&wst))
                        || matches!(act, MenuAct::WidgetSize(i) if (*i == 0) == (wsz == crate::widget_look::Size::Small))
                        || matches!(act, MenuAct::Freq(i) if *i == freq_now())
                        || matches!(act, MenuAct::Lang(i) if *i == lang_now());
                    let col = if hov { WHITE } else { BLACK };
                    if mark {
                        self.text_l(Rect::new(ir.x, ir.y, 20 * k, ir.h), 6 * k, "✓", col, px, false);
                    }
                    self.text_l(ir, 22 * k, label, col, px, false);
                }
            }
            Popup::Heal => {
                for (i, (_, label)) in fixes::GENERAL.iter().enumerate() {
                    let ir = Rect::new(r.x + 3 * k, r.y + 3 * k + i as i32 * 24 * k, r.w - 6 * k, 24 * k);
                    let h = Hit::MenuItem(heal_menu(), i as u8); // те же пункты, что в меню «Лечение»
                    self.hit(ir, h);
                    let hov = self.hover(h);
                    if hov {
                        self.c.fill(ir, SELECT);
                    }
                    self.text_l(ir, 22 * k, label, if hov { WHITE } else { BLACK }, px, false);
                }
            }
            Popup::Sessions => self.sessions_list(r),
            Popup::None => {}
        }
    }

    fn sessions_list(&mut self, r: Rect) {
        let k = self.s.k;
        let px = self.fs(13);
        if self.s.sessions.is_empty() {
            self.text_l(Rect::new(r.x, r.y, r.w, 40 * k), 10 * k, "сессий нет", BLACK, px, false);
            return;
        }
        for (i, s) in self.s.sessions.iter().enumerate() {
            let row = Rect::new(r.x + 4 * k, r.y + 4 * k + i as i32 * 40 * k, r.w - 8 * k, 40 * k);
            self.status_icon(if s.live { "ok" } else { "off" }, row.x + 6 * k, row.y + 14 * k, 12 * k, true);
            let t = localtime(s.mtime as i64);
            let st = match s.status.as_str() {
                "busy" => " · думает",
                "idle" => " · ждёт",
                _ => "",
            };
            let short: String = s.id.trim_start_matches("session_").chars().take(8).collect();
            let l1 = format!("{} · {short}  {:02}.{:02} {:02}:{:02}{st}", crate::diag::agent_name(s.agent), t.mday, t.mon, t.hour, t.min);
            // «~» — только если путь начинается с домашней папки целиком (не «/home/u» в «/home/user2»)
            let shown = match crate::store::home_dir().and_then(|h| std::path::Path::new(&s.cwd).strip_prefix(&h).ok().map(|r| r.to_path_buf())) {
                Some(r) if r.as_os_str().is_empty() => "~".to_string(),
                Some(r) => format!("~/{}", r.display()),
                None => s.cwd.clone(),
            };
            let l2 = format!("{}  [{shown}]", s.title.chars().take(60).collect::<String>());
            let bw = 28 * k;
            let copy = Rect::new(row.right() - 2 * bw - 8 * k, row.y + 7 * k, bw, 26 * k);
            let open = Rect::new(row.right() - bw - 4 * k, row.y + 7 * k, bw, 26 * k);
            let maxw = copy.x - row.x - 28 * k;
            let a = self.g.fonts.fit(&l1, maxw, px, false);
            let b = self.g.fonts.fit(&l2, maxw, px, false);
            self.text(row.x + 24 * k, row.y + 16 * k, &a, BLACK, px, false);
            self.text(row.x + 24 * k, row.y + 33 * k, &b, SHADOW, px, false);
            self.button(copy, "", Hit::SessionCopy(i as u8), true, false);
            // значок «копировать»: два листа друг за другом
            let (cx, cy, q) = (copy.x + copy.w / 2, copy.y + copy.h / 2, 5 * k);
            self.c.bevel(Rect::new(cx - q, cy - q + k, 2 * q - 2 * k, 2 * q), BLACK, BLACK);
            self.c.fill(Rect::new(cx - q + 3 * k, cy - q - 2 * k, 2 * q - 2 * k, 2 * q), WHITE);
            self.c.bevel(Rect::new(cx - q + 3 * k, cy - q - 2 * k, 2 * q - 2 * k, 2 * q), BLACK, BLACK);
            self.button(open, "▶", Hit::SessionOpen(i as u8), true, false);
        }
    }

    fn dialog_geom(&mut self) -> (Rect, Vec<String>, Vec<String>) {
        let k = self.s.k;
        let Some(d) = self.s.dialog.as_ref() else { return (Rect::default(), Vec::new(), Vec::new()) };
        let (head, body, cmd) = (d.head.clone(), d.body.clone(), d.cmd.clone());
        let w = (if cmd.is_some() { 520 } else { 420 } * k).min(self.s.w - 20 * k);
        let tw = w - 80 * k;
        let hl = self.g.fonts.wrap(&head, tw, self.fs(14), true, 2);
        let mut bl = self.g.fonts.wrap(&body, tw, self.fs(13), false, 6);
        if let Some(c) = cmd {
            // «вручную»: подпись и команда (строки команды помечены в начале символом \u{1})
            bl.push(String::new());
            bl.push("Или сама, в терминале:".into());
            for l in self.g.fonts.wrap(&c, tw - 12 * k, self.fs(13), false, 4) {
                bl.push(format!("\u{1}{l}"));
            }
        }
        let text_h = hl.len() as i32 * 20 * k + 6 * k + bl.len() as i32 * 17 * k;
        let h = 26 * k + 18 * k + text_h.max(40 * k) + 18 * k + 28 * k + 14 * k;
        let right = self.geo.right;
        let x = right.x + (right.w - w) / 2;
        let y = (right.y + 150 * k).min(self.s.h - h - 10 * k);
        (Rect::new(x, y, w, h), hl, bl)
    }

    fn dialog_rect(&mut self) -> Rect {
        self.dialog_geom().0
    }

    fn dialog(&mut self) {
        if self.s.dialog.is_none() {
            return;
        }
        let k = self.s.k;
        let (r, hl, bl) = self.dialog_geom();
        let Some(d) = self.s.dialog.as_ref() else { return };
        let (yes, no) = (d.yes_label.clone(), d.no_label.clone());
        let tb = Rect::new(r.x + 3 * k, r.y + 3 * k, r.w - 6 * k, 22 * k);
        // кнопки (области нажатий нужны и при пустом холсте)
        let bw = 110 * k;
        let by = r.bottom() - 14 * k - 28 * k;
        let mut bx = r.right() - 14 * k - bw;
        let no_r = Rect::new(bx, by, bw, 28 * k);
        bx -= bw + 10 * k;
        let yes_r = Rect::new(bx, by, bw, 28 * k);
        let has_cmd = d.cmd.is_some();
        let copy_r = Rect::new(r.x + 14 * k, by, 170 * k, 28 * k);
        if !self.painting(r) && self.hits.is_none() {
            return;
        }
        self.c.fill(Rect::new(r.x + 3 * k, r.y + 3 * k, r.w, r.h), 0x707070);
        self.c.raised(r, FACE);
        self.c.gradient_h(tb, &[(0.0, 0x0A246A), (1.0, 0xA6CAF0)]);
        self.text_l(tb, 6 * k, "Аптечка", WHITE, self.fs(13), true);
        self.status_icon("info", r.x + 16 * k, r.y + 44 * k, 32 * k, false);
        let mut y = r.y + 44 * k;
        for l in &hl {
            let base = self.g.fonts.center_base(y, 20 * k, self.fs(14));
            self.text(r.x + 64 * k, base, l, BLACK, self.fs(14), true);
            y += 20 * k;
        }
        y += 6 * k;
        // строки команды — на белом «утопленном» поле, как в окне терминала
        let cmd_rows: Vec<usize> = (0..bl.len()).filter(|i| bl[*i].starts_with('\u{1}')).collect();
        if let (Some(&a), Some(&b)) = (cmd_rows.first(), cmd_rows.last()) {
            let y0 = y + a as i32 * 17 * k - 2 * k;
            self.c.sunken(Rect::new(r.x + 60 * k, y0, r.w - 74 * k, (b - a + 1) as i32 * 17 * k + 4 * k), WHITE);
        }
        for l in &bl {
            let base = self.g.fonts.center_base(y, 17 * k, self.fs(13));
            match l.strip_prefix('\u{1}') {
                Some(c) => self.text(r.x + 66 * k, base, c, 0x003399, self.fs(13), false),
                None => self.text(r.x + 64 * k, base, l, BLACK, self.fs(13), false),
            }
            y += 17 * k;
        }
        if has_cmd {
            self.button(copy_r, "Копировать команду", Hit::DlgCopy, true, false);
        }
        match yes {
            Some(label) => {
                self.button(yes_r, &label, Hit::DlgYes, true, true);
                self.button(no_r, &no, Hit::DlgNo, true, false);
            }
            None => self.button(no_r, &no, Hit::DlgNo, true, true),
        }
    }
}

// ───────────────────────────── Образцы данных для снимков ─────────────────────────────

/// Включён ли показатель (строки не из каталога — всегда).
fn unit_on(id: &str) -> bool {
    id.is_empty() || catalog::find(id).map(|u| crate::prefs::unit_enabled(u.id, u.default_on)).unwrap_or(true)
}

/// Строки вкладки «Показатели»: по категориям — заголовок категории, под ним показатели с кнопкой.
fn settings_rows() -> Vec<Check> {
    let mut rows = Vec::new();
    for cat in CATS {
        let units: Vec<_> = catalog::units().iter().filter(|u| u.cat == cat).collect();
        if units.is_empty() {
            continue;
        }
        rows.push(Check {
            detail: format!("Категория «{}»: {}", cat.name(), cat.about()),
            ..chk("settings", &format!("— {} —", crate::i18n::tr(cat.name())), "info", cat.about(), &[])
        });
        for u in units {
            let on = unit_on(u.id);
            let c = chk(
                "settings",
                u.title,
                if on { "ok" } else { "off" },
                u.about,
                &[("unit_toggle", if on { "Не проверять" } else { "Проверять" }, Some(u.id))],
            );
            rows.push(Check { detail: u.about.to_string(), ..c });
        }
    }
    rows
}

fn chk(key: &'static str, title: &str, status: &str, value: &str, fixes: &[(&str, &str, Option<&str>)]) -> Check {
    Check {
        key,
        title: title.into(),
        status: status.into(),
        value: value.into(),
        detail: String::new(),
        fix: fixes.first().map(|f| f.0.to_string()),
        fixes: fixes.iter().map(|f| crate::diag::Fix { id: f.0.into(), label: f.1.into(), arg: f.2.map(|a| a.into()) }).collect(),
        unit: "",
        unchecked: false,
    }
}

/// UI_FAKE=ok — всё хорошо, UI_FAKE=fix — с поломками и кнопками «полечить».
pub fn demo_checks(kind: &str) -> Vec<Check> {
    let fix = kind == "fix";
    let v = vec![
        chk("ai", "Серверы Anthropic", "ok", "работают", &[]),
        chk("net", "Подключение", "ok", "через enp7s0", &[]),
        chk("net", "Интернет", "ok", "есть", &[]),
        chk("net", "DNS", "ok", "2 мс", &[]),
        if fix {
            chk("net", "Системный прокси", "bad", "включён, но пустой (сайты не открываются)", &[("proxy_off", "Выключить системный прокси", None)])
        } else {
            chk("net", "Системный прокси", "ok", "выключен (сеть напрямую)", &[])
        },
        chk("net", "VPN (итог)", "ok", "мир видит CA", &[]),
        chk("ai", "Связь с Claude API", "ok", "750 мс", &[]),
        chk("ai", "Связь с claude.ai", "ok", "635 мс", &[]),
        chk("net", "Мой локальный IP", "info", "192.168.2.207", &[]),
        chk("net", "Канал Happ", if fix { "warn" } else { "info" }, "выключен", &[("happ_restart", "Перезапустить Happ", None)]),
        chk("net", "Канал Outline", "info", "выключен", &[("outline_restart", "Перезапустить Outline", None)]),
        chk("gpu", "Видеокарта", "ok", "41°C · загрузка 2 %", &[]),
        chk("sys", "Процессор", "ok", "загрузка 1% (32 потока)", &[]),
        chk("sys", "Память", "ok", "15% из 92 ГБ", &[]),
        chk("sys", "Диск", "ok", "свободно 2249 ГБ из 2951", &[]),
        chk("sys", "Доступ к репозиториям", "ok", "OK (7 из 7)", &[]),
        if fix {
            let key = "gsettings|org.gnome.desktop.interface|gtk-theme";
            hinted(chk("theme", "Тема окон (GTK)", "warn", "Yaru", &[("theme_restore", "Вернуть как было", Some(key))]), "Сбилось! Было: Yaru-dark")
        } else {
            chk("theme", "Тема окон (GTK)", "ok", "Yaru-dark", &[])
        },
        chk("theme", "Иконки", "ok", "WhiteSur-dark", &[]),
        chk("theme", "Курсор", "ok", "MacTahoe-light", &[]),
        chk("theme", "Тема панели (Shell)", "ok", "Tahoe-Dark", &[]),
        chk("theme", "Док", "ok", "на месте", &[("dock_repair", "Починить док", Some("dash2dock-lite@icedman.github.com"))]),
        chk("theme", "Расширение Vitals", "ok", "работает", &[("ext_restart", "Перезапустить", Some("Vitals@CoreCoding.com"))]),
        if fix {
            hinted(
                chk("theme", "Расширение space-bar", "warn", "выключено", &[("ext_enable", "Включить", Some("space-bar@luchrioh"))]),
                "Было включено, а теперь нет.",
            )
        } else {
            chk("theme", "Расширение space-bar", "ok", "работает", &[("ext_restart", "Перезапустить", Some("space-bar@luchrioh"))])
        },
        hinted(
            chk("theme", "Эталон", "info", "Wed Oct 28 22:32:50 2026", &[("snapshot", "Запомнить текущее как норму", None)]),
            "С этим состоянием сравниваются темы.",
        ),
    ];
    v
}

fn hinted(mut c: Check, hint: &str) -> Check {
    c.detail = hint.to_string();
    c
}

/// Образец замера скорости (UI_FAKE).
pub fn demo_speed() -> Vec<Check> {
    vec![
        chk("net", "Пинг", "ok", "172 мс · джиттер 15 мс", &[]),
        chk("net", "Скорость ↓ скачивание", "ok", "158 Мбит/с", &[]),
        chk("net", "Скорость ↑ отдача", "ok", "86 Мбит/с", &[]),
    ]
}

pub fn demo_sessions() -> Vec<Session> {
    let now = now_secs() as f64;
    let mk = |id: &str, title: &str, live: bool, st: &str, ago: f64| Session {
        id: id.into(),
        cwd: "~/проекты/аптечка".into(),
        title: title.into(),
        mtime: now - ago,
        live,
        status: st.into(),
        path: String::new(),
        agent: "claude",
    };
    vec![
        mk("0f1a2b3c-cafe-4cat-b0ba-00000000c0de", "собери окно в стиле XP", true, "busy", 30.0),
        mk("0f3a9c21-77aa-4be1-9d0e-123456789abc", "почини прокси", false, "", 7200.0),
        mk("c2d4e6f8-1111-2222-3333-444455556666", "что с видеокартой", false, "", 86400.0),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Нажали кнопку починки A, пока кнопка зажата, пришли новые строки с B, отпустили — B не запускается.
    #[test]
    fn refresh_cancels_pressed_fix() {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(vec![chk("net", "A", "bad", "", &[("a", "A", None)])], false);
        u.ensure_hits();
        let r = u.hit_rect(Hit::Fix(0));
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        u.mouse_down(x, y);
        u.set_checks(vec![chk("net", "B", "bad", "", &[("b", "B", None)])], false);
        let c = u.mouse_up(x, y);
        assert!(!c.iter().any(|c| matches!(c, Cmd::RunFix { id, .. } if id == "b")));
    }

    /// «Игнорировать»: у беды без починки — кнопка во всю ширину, у беды с починкой — рядом с ней (не внахлёст),
    /// у игнорируемой строки — только «Не игнорировать».
    #[test]
    fn ignore_buttons() {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(vec![chk("net", "A", "bad", "", &[]), chk("net", "B", "warn", "", &[("b", "B", None)]), chk("net", "C", "ok", "", &[])], false);
        u.ensure_hits();
        let has = |u: &Ui, h: Hit| u.hits.iter().any(|(_, x)| *x == h);
        let (a, b) = (u.hit_rect(Hit::Ignore(0)), u.hit_rect(Hit::Ignore(1)));
        assert!(has(&u, Hit::Ignore(0)) && has(&u, Hit::Ignore(1)) && !has(&u, Hit::Ignore(2)));
        assert!(b.w < a.w && u.hit_rect(Hit::Fix(1)).intersect(b).is_empty());
        u.st.ignored.insert("B".into());
        u.hits_ok = false;
        u.ensure_hits();
        assert!(has(&u, Hit::Ignore(1)) && !has(&u, Hit::Fix(1)));
    }

    /// Новое название сессии (тот же номер) доходит до окна.
    #[test]
    fn session_title_updates() {
        let mut u = Ui::new(1, 300, 500);
        let mut ss = demo_sessions();
        u.set_sessions(ss.clone());
        ss[0].title = "changed".into();
        u.set_sessions(ss);
        assert_eq!(u.st.sessions[0].title, "changed");
    }

    /// Номера элементов специальных возможностей не повторяются.
    #[test]
    fn accessibility_ids_unique() {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(demo_checks("fix"), false);
        let nodes = u.a11y_nodes();
        let mut seen = std::collections::HashSet::new();
        assert!(nodes.iter().all(|n| seen.insert(n.id)));
    }
}
