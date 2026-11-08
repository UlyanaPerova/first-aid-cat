//! Виджет: маленькое окно на рабочем столе — живой кот и главные показатели (сеть, VPN, ИИ, система).
//!
//! `aptechka --widget` — отдельный лёгкий режим: без большого окна, таблиц и журнала. Кот — вырез вокруг
//! головы из той же картинки, 6 кадров в секунду; показатели — те же проверки, что в окне (только включённые
//! в «Показателях»), раз в минуту в фоне. Оформление — Windows XP, как у большого окна. Тычок в кота —
//! реакция и реплика в облачке; щелчок по списку — полная Аптечка; заголовок — перетащить (место
//! запоминается); крестик — закрыть. Без системной рамки, под окнами, без кнопки на панели задач.
//! Вид и размер — в настройках (меню «Вид» большого окна): «Окно», «Монитор», «Тамагочи», «Стекло»
//! (widget_look.rs); виджет замечает смену сам. `WIDGET_SHOT=файл.png` — нарисовать виджет в PNG без окна
//! (WIDGET_STYLE=monitor WIDGET_SIZE=small — другой вид).

use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId, WindowLevel};

use crate::diag::{self, Check};
use crate::gfx::Fonts;
use crate::quips::Quips;
use crate::widget_anim::{self as anim, Anim, Caps};
use crate::widget_look::{self as look, Hits, Size, Style};
use crate::widget_px::{self as px, Pieces, Sprite};
use crate::widget_tpl::TplData;

const FPS: f64 = 6.0;

/// Что нарисовано в окне: по этому решаем, нужен ли новый кадр (см. about_to_wait).
#[derive(Clone, PartialEq)]
struct Sig {
    anim: Anim,
    /// шаг светлой полосы ЭЛТ (у видов с экраном)
    crt: i64,
    quip: bool,
}

/// Неподвижная часть вида (всё, кроме оживления): пока данные те же, второй раз её не рисуем.
#[derive(Clone, PartialEq)]
struct BaseKey {
    name: String,
    cell: i32,
    k: i32,
    close: bool,
    crt: i64,
    d: TplData,
    lang: crate::i18n::Lang,
}

/// Ухо, которое тянут мышью: какое (0 — левое), когда нажали и когда отпустили.
#[derive(Clone, Copy)]
struct EarPull {
    side: usize,
    t0: Instant,
    rel: Option<Instant>,
}

/// Счётчики для WIDGET_ANIM_LOG=1: сколько раз проснулись и сколько раз перерисовали.
#[derive(Default)]
struct Stats {
    ticks: u64,
    draws: u64,
    draw_time: Duration,
    since: Option<Instant>,
}

/// Строка виджета: подпись, статус (ok/warn/bad/info), значение.
#[derive(Clone)]
pub(crate) struct Row {
    pub label: &'static str,
    pub status: String,
    pub value: String,
}

fn rank(s: &str) -> u8 {
    match s {
        "bad" => 3,
        "warn" => 2,
        "ok" => 1,
        _ => 0,
    }
}

/// Свести проверки к пяти строкам виджета.
fn summarize(all: &[Check]) -> Vec<Row> {
    let worst = |f: &dyn Fn(&Check) -> bool| -> Option<&Check> { all.iter().filter(|c| f(c) && c.status != "info").max_by_key(|c| rank(&c.status)) };
    let row = |label, c: Option<&Check>, okv: &str| match c {
        // беда — коротко, что сломалось (подробности — в большом окне)
        // («VPN (итог)» → «VPN»: служебная пометка большого окна виджету не нужна)
        Some(c) if rank(&c.status) >= 2 => Row { label, status: c.status.clone(), value: c.title.replace(" (итог)", "") },
        Some(c) => Row { label, status: c.status.clone(), value: okv.to_string() },
        None => Row { label, status: "info".into(), value: "—".into() },
    };
    let is_vpn = |c: &Check| c.title.starts_with("VPN") || c.title.starts_with("Канал");
    let vpn_on = all.iter().filter(|c| is_vpn(c)).find(|c| c.status == "ok" && !c.value.contains("выкл"));
    let vpn = match vpn_on {
        Some(c) if c.title.contains("итог") => Row { label: "VPN", status: "ok".into(), value: c.value.clone() },
        Some(c) => Row { label: "VPN", status: "ok".into(), value: c.title.trim_start_matches("Канал ").trim_start_matches("VPN ").to_string() },
        // всё «для сведения» (не используется, нет интернета, не проверить) — значение итога как есть
        None => match (worst(&|c| is_vpn(c)), all.iter().find(|c| c.title == "VPN (итог)")) {
            (None, Some(t)) => Row { label: "VPN", status: "info".into(), value: t.value.clone() },
            (w, _) => row("VPN", w, "выключен"),
        },
    };
    let bad = all.iter().filter(|c| c.status == "bad").count();
    let warn = all.iter().filter(|c| c.status == "warn").count();
    let real = all.iter().filter(|c| c.status != "info" && c.status != "checking").count();
    let total = if real == 0 {
        // проверок ещё нет (или все — «не проверить») — это не «всё в порядке»
        Row { label: "Итог", status: "info".into(), value: "ещё проверяю".into() }
    } else if bad + warn == 0 && crate::diag::unchecked_count(all) > 0 {
        // часть проверок не смогла выполниться: не зелёное «всё в порядке», а нейтральное
        Row { label: "Итог", status: "info".into(), value: "не всё проверено".into() }
    } else if bad + warn == 0 {
        Row { label: "Итог", status: "ok".into(), value: "всё в порядке".into() }
    } else {
        let v = match (bad, warn) {
            (0, w) => format!("внимание: {w}"),
            (b, 0) => format!("проблем: {b}"),
            (b, w) => format!("{b} беда · {w} предупр."),
        };
        Row { label: "Итог", status: if bad > 0 { "bad" } else { "warn" }.into(), value: v }
    };
    vec![
        row("Сеть", worst(&|c| c.key == "net" && !is_vpn(c)), "в порядке"),
        vpn,
        row("ИИ", worst(&|c| c.key == "ai"), "на связи"),
        row("Система", worst(&|c| c.key == "sys" || c.key == "gpu" || c.key == "dev"), "в порядке"),
        total,
    ]
}

/// Нагрузка: (процессор %, память %; память < 0 — не знаем). Linux — /proc, Windows — GetSystemTimes и
/// GlobalMemoryStatusEx, macOS — средняя загрузка (sysctl vm.loadavg) на число ядер.
struct Load {
    prev: Option<(u64, u64)>,
}

impl Load {
    #[cfg(target_os = "linux")]
    fn sample(&mut self) -> Option<(f64, f64)> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let f: Vec<u64> = stat.lines().next()?.split_whitespace().skip(1).filter_map(|x| x.parse().ok()).collect();
        let idle = f.get(3)? + f.get(4).unwrap_or(&0);
        // как в основной проверке: первые 8 полей (guest/guest_nice уже входят в user/nice — не считать дважды)
        let total: u64 = f.iter().take(8).sum();
        let cpu = self.cpu(idle, total);
        let mem = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb = |k: &str| mem.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<f64>().ok());
        let (t, a) = (kb("MemTotal:")?, kb("MemAvailable:")?);
        Some((cpu, 100.0 * (1.0 - a / t)))
    }

    #[cfg(windows)]
    fn sample(&mut self) -> Option<(f64, f64)> {
        #[repr(C)]
        struct MemStatus {
            len: u32,
            load: u32,
            rest: [u64; 7],
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetSystemTimes(idle: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
            fn GlobalMemoryStatusEx(m: *mut MemStatus) -> i32;
        }
        let (mut i, mut k, mut u) = (0u64, 0u64, 0u64);
        let mut m = MemStatus { len: std::mem::size_of::<MemStatus>() as u32, load: 0, rest: [0; 7] };
        // время ядра включает простой
        let ok = unsafe { GetSystemTimes(&mut i, &mut k, &mut u) != 0 && GlobalMemoryStatusEx(&mut m) != 0 };
        if !ok {
            return None;
        }
        let cpu = self.cpu(i, k + u);
        Some((cpu, m.load as f64))
    }

    #[cfg(target_os = "macos")]
    fn sample(&mut self) -> Option<(f64, f64)> {
        let (_, la) = crate::diag::run(&["sysctl", "-n", "vm.loadavg"], 3);
        let (_, n) = crate::diag::run(&["sysctl", "-n", "hw.ncpu"], 3);
        let l1: f64 = la.trim_matches(|c| c == '{' || c == '}' || c == ' ').split_whitespace().next()?.parse().ok()?;
        let cores: f64 = n.trim().parse().ok()?;
        Some(((l1 / cores * 100.0).min(100.0), -1.0))
    }

    /// Доля занятости по разнице с прошлым замером (простой, всего).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn cpu(&mut self, idle: u64, total: u64) -> f64 {
        match self.prev.replace((idle, total)) {
            // счётчики могли сброситься (сон, пересчёт) — без вычитания «в минус»
            Some((pi, pt)) if total > pt && idle >= pi => (100.0 * (1.0 - (idle - pi) as f64 / (total - pt) as f64)).clamp(0.0, 100.0),
            _ => 0.0,
        }
    }
}

pub struct Widget {
    /// окно двигали — место запишем, когда перетаскивание утихнет (не на каждое событие: это десятки в
    /// секунду, и каждое ждало бы замок и запись на диск)
    pos_dirty: Option<Instant>,
    rows: Vec<Row>,
    /// последние проверки целиком (для полей шаблона: диск, процессор, кто из ИИ думает)
    checks: Vec<Check>,
    /// они же как пришли (до «Игнорировать») и какие строки игнорировались при подсчёте
    raw: Vec<Check>,
    ignored: std::collections::HashSet<String>,
    /// виджеты целиком из концептов на сетке (art/layers/виджет/целиком) и их разметка
    wholes: std::collections::HashMap<String, Sprite>,
    whole_specs: std::collections::HashMap<String, serde_json::Value>,
    /// нижний слой вида в полной детализации концепта (art/layers/виджет/слои/<вид>_<размер>@<клетка>.png):
    /// рамка и кот без живых данных; у среднего монитора его нет — тот по клеткам с ЭЛТ-экраном
    layers: std::collections::HashMap<String, Sprite>,
    /// обычный (не пиксельный) шрифт — для «Стекла», в его концепте надписи гладкие
    sans: Fonts,
    /// данные как в концепте (для сравнения с ним: WIDGET_CONCEPT=1)
    fixed: Option<TplData>,
    /// размер окна (пиксели) и во сколько раз увеличен пиксельный холст
    win_size: (i32, i32),
    scale: i32,
    load: Load,
    load_now: Option<(f64, f64)>,
    fonts: Fonts,
    pixel: Fonts,
    /// масштаб интерфейса
    k: i32,
    style: Style,
    size: Size,
    /// где кот, крестик и список на последнем кадре
    hits: Hits,
    /// кот с кружкой (вид «Стекло»): ok, warn, bad
    mugs: [Sprite; 3],
    /// когда последний раз перечитывали вид из настроек
    prefs_t: Instant,
    /// файл программы и время его изменения при запуске: Аптечку обновили — виджет перезапускается новой версией
    exe: Option<(std::path::PathBuf, Option<std::time::SystemTime>)>,
    t0: Instant,
    rx: Option<Receiver<Vec<Check>>>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    next: Instant,
    /// Окно закрыто другими окнами (Occluded) или свёрнуто: картинку не перерисовываем, данные берём.
    hidden: bool,
    /// Linux: закрыт ли виджет окнами — спрашиваем сами (cover.rs), раз в секунду; Occluded там не приходит
    #[cfg(all(unix, not(target_os = "macos")))]
    cover: Option<Option<crate::cover::Cover>>,
    covered: bool,
    #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
    cover_t: Instant,
    /// размер колесом мыши: ступеней клетки к обычной (ui.json widget_zoom, ZOOM_MIN..=ZOOM_MAX) и прокрутка,
    /// ещё не набравшая ступень
    zoom: i32,
    wheel: f64,
    cursor: PhysicalPosition<f64>,
    /// нажата кнопка «закрыть» (отпускание над ней — закрыть виджет)
    close_down: bool,
    /// мышь над виджетом — показать крестик «закрыть»
    hover: bool,
    /// последняя просьба «показать» (файл widget.show в папке сеанса) и до какого времени держать поверх окон
    show_seen: i64,
    on_top_until: Option<Instant>,
    /// картинка не совпала с окном — перестроить окно к следующему кадру
    refit: bool,
    quips: Quips,
    /// реплика кота в облачке и до какого времени её показывать
    quip: Option<(String, Instant)>,
    /// оживление включено (ui.json widget_anim, WIDGET_ANIM=0 выключает)
    anim_on: bool,
    /// что умеет текущий вид (по разметке) и клетка его картинки
    caps: Caps,
    cell: i32,
    /// у вида есть ЭЛТ-экран со ползущей полосой (она тоже кадры)
    crt_on: bool,
    /// последний нарисованный кадр, время рисования и «грязь» (мышь, данные): нужен новый кадр
    drawn: Option<Sig>,
    last_draw: Instant,
    dirty: bool,
    base: Option<(BaseKey, px::Px, Hits)>,
    ear: Option<EarPull>,
    /// куда нажимать, чтобы потянуть ухо (точки окна)
    ear_rects: Vec<crate::gfx::Rect>,
    load_t: Instant,
    /// для снимков: время оживления и ухо, заданные снаружи
    t_fixed: Option<f64>,
    ear_fixed: Option<(usize, u8)>,
    stats: Stats,
    log: bool,
}

impl Widget {
    pub fn new(k: i32) -> Widget {
        let (style, size) = look::chosen();
        let mug = |s: &str| Sprite::load(&format!("виджет/кружка_{s}.png")).unwrap_or(Sprite { w: 1, h: 1, px: vec![0] });
        let mut layers = std::collections::HashMap::new();
        let (mut wholes, mut whole_specs) = (std::collections::HashMap::new(), std::collections::HashMap::new());
        let art = std::env::var("ART").unwrap_or_default();
        for st in look::STYLES {
            for sz in [Size::Small, Size::Medium] {
                let base = format!("{}_{}", st.id(), sz.id());
                if let Some(spec) =
                    crate::art::read(&art, &format!("виджет/целиком/{base}.json")).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                {
                    // кот в виджете всегда обычный (о беде говорят значки и цвет итога) — картинка одна
                    if let Some(sp) = Sprite::load(&format!("виджет/целиком/{base}_ok.png")) {
                        wholes.insert(format!("{base}_ok"), sp);
                    }
                    whole_specs.insert(base.clone(), spec);
                }
                for c in 2..=4 {
                    if let Some(sp) = Sprite::load(&format!("виджет/слои/{base}@{c}.png")) {
                        layers.insert(format!("{base}@{c}"), sp);
                    }
                    // значки состояния из концепта (номер поля в разметке)
                    for fi in 0..16 {
                        let key = format!("{base}_f{fi}@{c}");
                        if let Some(sp) = Sprite::load(&format!("виджет/слои/{key}.png")) {
                            layers.insert(key, sp);
                        }
                    }
                }
            }
        }
        let mut w = Widget {
            pos_dirty: None,
            rows: summarize(&[]),
            checks: Vec::new(),
            raw: Vec::new(),
            ignored: Default::default(),
            #[cfg(all(unix, not(target_os = "macos")))]
            cover: None,
            covered: false,
            #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
            cover_t: Instant::now(),
            zoom: zoom_pref(),
            wheel: 0.0,
            wholes,
            whole_specs,
            layers,
            sans: Fonts::new(),
            fixed: None,
            win_size: (1, 1),
            scale: 1,
            load: Load { prev: None },
            load_now: None,
            fonts: Fonts::new(),
            pixel: Fonts::pixel(),
            k,
            style,
            size,
            hits: Hits::default(),
            mugs: [mug("ok"), mug("warn"), mug("bad")],
            prefs_t: Instant::now(),
            exe: exe_stamp(),
            t0: Instant::now(),
            rx: None,
            window: None,
            surface: None,
            next: Instant::now(),
            hidden: false,
            cursor: PhysicalPosition::new(0.0, 0.0),
            close_down: false,
            hover: false,
            show_seen: show_request(),
            on_top_until: None,
            refit: false,
            quips: Quips::new(),
            quip: None,
            anim_on: look::anim_enabled(),
            caps: Caps::default(),
            cell: 3,
            crt_on: false,
            drawn: None,
            last_draw: Instant::now(),
            dirty: true,
            base: None,
            ear: None,
            ear_rects: Vec::new(),
            load_t: Instant::now(),
            t_fixed: None,
            ear_fixed: None,
            stats: Stats::default(),
            log: std::env::var_os("WIDGET_ANIM_LOG").is_some(),
        };
        w.relayout();
        w
    }

    /// Размер окна в пикселях.
    pub fn size(&self) -> (i32, i32) {
        self.win_size
    }

    /// Нарисовать пиксельный холст выбранного вида (Hits — в пикселях холста).
    fn canvas(&mut self) -> (px::Px, Hits) {
        let d = match &self.fixed {
            Some(f) => f.clone(),
            None => self.tpl_data(),
        };
        // кот всегда обычный (грустная и испуганная мордочки не нужны — о беде говорят значки и цвет итога)
        let key = format!("{}_{}_ok", self.style.id(), self.size.id());
        let show_close = (self.hover || std::env::var_os("WIDGET_HOVER").is_some()) && self.style != Style::Window;
        // есть виджет целиком (концепт на сетке) — он; иначе вид рисуется кодом
        let base = format!("{}_{}", self.style.id(), self.size.id());
        if let (Some(spec), Some(sp)) = (self.whole_specs.get(&base), self.wholes.get(&key).or_else(|| self.wholes.get(&format!("{base}_ok")))) {
            // клетка картинки ≈ 1,5 точки экрана (на экране ×2 — 3 пикселя), текст и значки — ×k (чёткий шрифт)
            // (у концепта с крупной сеткой — "cell_scale" в описании: стекло ×4/3, чтобы было ростом с остальные)
            let (base_cell, _) = self.zoom_level();
            let cell = ((base_cell as f64) * spec["cell_scale"].as_f64().unwrap_or(1.0)).round().max(1.0) as i32;
            let t = self.anim_time();
            let caps = if self.anim_on { Caps::of(spec) } else { Caps::default() };
            self.caps = caps;
            self.cell = cell;
            self.crt_on = spec.get("crt").is_some();
            // светлая полоса ЭЛТ ползёт шагами по 1/6 с (как раньше кадры), а не каждый отсчёт
            let crt = if spec.get("crt").is_some() { (t * FPS) as i64 } else { 0 };
            let key = BaseKey { name: base.clone(), cell, k: self.k, close: show_close, crt, d: d.clone(), lang: crate::i18n::lang() };
            let (mut p, hits) = match self.base.as_ref().filter(|(k, _, _)| *k == key) {
                Some((_, p, h)) => (p.clone(), *h),
                None => {
                    let look = px::Look {
                        screen_dim: self.style == Style::Monitor,
                        lift: 0,
                        t: crt as f64 / FPS,
                        pieces: &self.layers,
                        text_scale: base_cell as f64 / self.k.max(1) as f64 / 1.5,
                        name: base.clone(),
                    };
                    let layer = self.layers.get(&format!("{base}@{cell}"));
                    let (p, h) = px::whole(
                        sp,
                        layer,
                        spec,
                        &d,
                        &mut self.pixel,
                        &mut self.sans,
                        show_close && self.style != Style::Window,
                        cell,
                        self.k.max(1),
                        &look,
                    );
                    self.base = Some((key, p.clone(), h));
                    (p, h)
                }
            };
            if caps.any() {
                let a = self.frame_anim(t, cell, caps);
                anim::apply(&mut p, spec, cell, &a);
            }
            // места ушей — в точках окна (холст увеличен в scale раз)
            let s = self.scale.max(1);
            self.ear_rects = if caps.ears > 0 { anim::ear_hits(spec, cell).into_iter().map(|r| px::up(r, s)).collect() } else { Vec::new() };
            return (p, hits);
        }
        // запасной путь (у вида нет картинки «целиком»): рисуется кодом, без спрайта кота
        let pieces = Pieces { cat: None, mug: &self.mugs[d.state.min(2)] };
        px::draw(self.style, self.size, &d, &pieces, &mut self.pixel, show_close)
    }

    /// Время оживления, секунды (в снимках — заданное снаружи).
    fn anim_time(&self) -> f64 {
        self.t_fixed.unwrap_or_else(|| self.t0.elapsed().as_secs_f64())
    }

    /// Кадр оживления в момент t: покойная жизнь вида плюс ухо, которое сейчас тянут (кот при этом жмурится).
    fn frame_anim(&self, t: f64, cell: i32, caps: Caps) -> Anim {
        let mut a = Anim::idle(t, cell, caps);
        let pull = match (self.ear_fixed, self.ear) {
            (Some((side, lvl)), _) => Some((side, lvl)),
            (None, Some(e)) => {
                let now = Instant::now();
                let end = e.rel.unwrap_or(now);
                let held = end.duration_since(e.t0).as_secs_f64().min(anim::EAR_HOLD_MAX);
                let since = e.rel.map(|r| now.duration_since(r).as_secs_f64());
                Some((e.side, anim::ear_step(anim::ear_env(held, since))))
            }
            _ => None,
        };
        // смущение держится и после того, как ухо вернулось (SHY_AFTER)
        let shy_tail = self.ear_fixed.is_none()
            && self.ear.is_some_and(|e| e.side < caps.ears && e.rel.is_some_and(|r| r.elapsed().as_secs_f64() < anim::SHY_AFTER));
        if shy_tail {
            a.shy = true;
        }
        if let Some((side, lvl)) = pull.filter(|(s, l)| *l > 0 && *s < caps.ears) {
            a.ear[side] = lvl;
            // жмурится, как в приложении: нарисованная мордочка «shy»
            a.shy = true;
        }
        a
    }

    /// Что должно быть нарисовано сейчас (для сравнения с нарисованным).
    fn sig(&self) -> Sig {
        let t = self.anim_time();
        let crt = if self.crt_on { (t * FPS) as i64 } else { 0 };
        let anim = if self.caps.any() { self.frame_anim(t, self.cell, self.caps) } else { Anim::default() };
        Sig { anim, crt, quip: self.quip.as_ref().is_some_and(|(_, until)| Instant::now() < *until) }
    }

    /// Нажали на ухо? (0 — левое, 1 — правое)
    fn ear_at(&self, x: i32, y: i32) -> Option<usize> {
        self.ear_rects.iter().position(|r| r.contains(x, y))
    }

    fn release_ear(&mut self) {
        if let Some(e) = self.ear.as_mut().filter(|e| e.rel.is_none()) {
            e.rel = Some(Instant::now());
            self.dirty = true;
        }
    }

    /// Пересчитать размер окна под вид и масштаб (холст не меняется от данных — только от вида).
    fn relayout(&mut self) {
        let (p, _) = self.canvas();
        // виджет из концепта рисуется сразу в пикселях экрана (клетка ≈1,5 точки) — без увеличения;
        // нарисованный кодом — подгонка под высоту ~240 точек
        let whole = self.whole_specs.contains_key(&format!("{}_{}", self.style.id(), self.size.id()));
        self.scale = if whole { self.zoom_level().1 } else { px::scale_for(p.h, self.k) };
        self.win_size = (p.w * self.scale, p.h * self.scale);
    }

    /// Поля шаблона: пять строк как в концепте (Сеть, VPN, ИИ, ЦП, Диск), итог, время, уровни.
    fn tpl_data(&self) -> TplData {
        let row = |l: &str| self.rows.iter().find(|r| r.label == l).cloned();
        let st = |r: &Option<Row>| r.as_ref().map_or("info".to_string(), |r| r.status.clone());
        let net = row("Сеть");
        let net_t = match &net {
            Some(r) if rank(&r.status) >= 2 => format!("Сеть — {}", r.value),
            _ => "Сеть — есть".into(),
        };
        let vpn = row("VPN");
        let vpn_on = vpn.as_ref().is_some_and(|r| r.status == "ok");
        let ai = row("ИИ");
        // кто из ИИ-агентов сейчас думает (строка «…: сессии — 1 в терминале · думает»)
        let thinking = self.checks.iter().find(|c| c.value.contains("думает")).map(|c| c.title.split(':').next().unwrap_or("ИИ").trim().to_string());
        let ai_t = match (&ai, thinking) {
            (Some(r), _) if rank(&r.status) >= 2 => format!("ИИ — {}", r.value),
            (_, Some(who)) => format!("ИИ — {who} думает"),
            _ => "ИИ — на связи".into(),
        };
        let num = |s: &str, after: &str| -> Option<f64> {
            let i = s.find(after)? + after.len();
            s[i..].trim_start().split(|c: char| !c.is_ascii_digit() && c != '.').next()?.parse().ok()
        };
        let cpu_chk = self.checks.iter().find(|c| c.title == "Процессор");
        let cpu = self.load_now.map(|l| l.0).or_else(|| cpu_chk.and_then(|c| num(&c.value, "занят на")));
        let cpu_t = cpu.map_or("ЦП —".to_string(), |v| format!("ЦП {v:.0}%"));
        let cpu_st = cpu_chk.map_or("info".to_string(), |c| c.status.clone());
        // «Диск» (Linux, macOS) или «Диск C:» (Windows): «свободно 220 ГБ из 951» / «свободно 220 ГБ (17%)»
        let disk = self.checks.iter().find(|c| c.title.starts_with("Диск"));
        // единица после числа: «свободно 574 МБ из 7 ГБ» — мегабайты (было «574 ГБ»); всё считаем в ГБ
        let unit_k = |s: &str, after: &str| -> f64 {
            let Some(i) = s.find(after) else { return 1.0 };
            let rest = s[i + after.len()..].trim_start().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ',').trim_start();
            if rest.starts_with("МБ") {
                1.0 / 1024.0
            } else if rest.starts_with("ТБ") {
                1024.0
            } else {
                1.0
            }
        };
        let free = disk.and_then(|c| num(&c.value, "свободно").map(|v| v * unit_k(&c.value, "свободно")));
        let total = disk.and_then(|c| num(&c.value, "из").map(|v| v * unit_k(&c.value, "из")));
        let free_pct = disk.and_then(|c| num(&c.value, "("));
        let disk_t = free.map_or("Диск —".to_string(), |v| {
            if v < 1.0 {
                format!("Диск — {:.0} МБ", v * 1024.0)
            } else {
                format!("Диск — {v:.0} ГБ")
            }
        });
        let disk_used = match (free, total, free_pct) {
            (Some(f), Some(t), _) if t > 0.0 => Some(100.0 * (1.0 - f / t)),
            (_, _, Some(p)) => Some(100.0 - p),
            _ => None,
        };
        let tot = row("Итог");
        let total_st = st(&tot);
        let total_t = if total_st == "ok" {
            "Всё в порядке".to_string()
        } else {
            let worst = self.rows.iter().filter(|r| r.label != "Итог" && rank(&r.status) >= 2).max_by_key(|r| rank(&r.status));
            match worst {
                Some(w) => format!("{}: {}", if w.status == "bad" { "Беда" } else { "Внимание" }, w.value),
                None => tot.map(|r| r.value).unwrap_or_else(|| "Проверяю…".into()),
            }
        };
        let tm = crate::ui::localtime(crate::ui::now_secs());
        TplData {
            rows: [
                (net_t, st(&net)),
                // («нет данных» — страну выхода узнать не удалось: это не «выключен»)
                (
                    if vpn_on {
                        "VPN — вкл".into()
                    } else if vpn.as_ref().is_some_and(|r| r.value.contains("не используется")) {
                        "VPN — не используется".into()
                    } else if st(&vpn) == "info" {
                        "VPN — нет данных".into()
                    } else {
                        "VPN — выкл".into()
                    },
                    if vpn_on { "ok".into() } else { st(&vpn) },
                ),
                (ai_t, st(&ai)),
                (cpu_t, cpu_st),
                (disk_t, disk.map_or("info".into(), |c| c.status.clone())),
            ],
            state: match total_st.as_str() {
                "bad" => 2,
                "warn" => 1,
                _ => 0,
            },
            total: (total_t, total_st),
            clock: format!("{:02}:{:02}", tm.hour, tm.min),
            cpu,
            disk_used,
        }
    }

    fn size_now(&self) -> (i32, i32) {
        self.window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width as i32, s.height as i32)
            })
            .unwrap_or((0, 0))
    }

    /// Аптечка попросила показать виджет («Показать на рабочем столе», а он уже открыт): поднять поверх
    /// окон на 5 секунд и вернуть на экран, если уехал за край.
    fn check_show_request(&mut self) {
        let req = show_request();
        if req != self.show_seen {
            self.show_seen = req;
            if let Some(w) = &self.window {
                w.set_window_level(WindowLevel::AlwaysOnTop);
                w.focus_window();
                if let (Ok(pos), Some(mon)) = (w.outer_position(), w.current_monitor().or_else(|| w.primary_monitor())) {
                    let (mp, ms, ws) = (mon.position(), mon.size(), w.outer_size());
                    // виджет больше экрана (маленький монитор, крупный масштаб) — верхняя граница не меньше нижней
                    let x = pos.x.clamp(mp.x, (mp.x + ms.width as i32 - ws.width as i32).max(mp.x));
                    let y = pos.y.clamp(mp.y, (mp.y + ms.height as i32 - ws.height as i32).max(mp.y));
                    if (x, y) != (pos.x, pos.y) {
                        w.set_outer_position(PhysicalPosition::new(x, y));
                    }
                }
                self.on_top_until = Some(Instant::now() + Duration::from_secs(5));
            }
        }
        if self.on_top_until.is_some_and(|t| Instant::now() >= t) {
            self.on_top_until = None;
            if let Some(w) = &self.window {
                w.set_window_level(WindowLevel::AlwaysOnBottom);
            }
        }
    }

    /// Вид или размер сменили в настройках — перестроить окно.
    /// Закрыт ли виджет другими окнами целиком (Linux: спрашиваем оконный менеджер раз в секунду). Пока закрыт —
    /// оживление спит: его всё равно никто не видит.
    fn check_covered(&mut self) {
        #[cfg(any(windows, all(unix, not(target_os = "macos"))))]
        {
            if self.cover_t.elapsed() < Duration::from_secs(1) {
                return;
            }
            self.cover_t = Instant::now();
            let Some(w) = self.window.as_ref() else { return };
            // Linux: спрашиваем оконный менеджер (cover.rs); Windows: какое окно видно в точках виджета
            #[cfg(all(unix, not(target_os = "macos")))]
            let now = {
                let (Ok(p), s) = (w.outer_position(), w.outer_size()) else { return };
                let cover = self.cover.get_or_insert_with(crate::cover::Cover::new);
                cover.as_ref().and_then(|c| c.covered((p.x, p.y, s.width as i32, s.height as i32))).unwrap_or(false)
            };
            #[cfg(windows)]
            let now = covered_windows(w).unwrap_or(false);
            if now != self.covered {
                self.covered = now;
                self.dirty = true;
                if self.log {
                    eprintln!("[виджет] {}", if now { "закрыт окнами — оживление спит" } else { "снова виден" });
                }
            }
        }
    }

    /// Строки и итог из последних проверок: игнорируемые — «для сведения»; плюс последний замер скорости
    /// (его делает окно) — итог как в окне.
    fn recount(&mut self) {
        self.checks = crate::diag::apply_ignored(&self.raw);
        let mut all = self.raw.clone();
        all.extend(crate::sched::load_speed());
        self.rows = summarize(&crate::diag::apply_ignored(&all));
    }

    fn reload_prefs(&mut self) {
        self.check_show_request();
        // в окне переключили «Анимация виджета»
        let on = look::anim_enabled();
        if on != self.anim_on {
            self.anim_on = on;
            self.dirty = true;
        }
        // в окне нажали «Игнорировать» — итог виджета пересчитать сразу, не дожидаясь новых проверок
        let ig = crate::prefs::ignored();
        if ig != self.ignored && !self.raw.is_empty() {
            self.ignored = ig;
            self.recount();
            self.dirty = true;
        }
        let (style, size) = look::chosen();
        if (style, size) == (self.style, self.size) {
            return;
        }
        self.style = style;
        self.size = size;
        let before = self.size_now();
        self.relayout();
        if self.size() != before {
            self.fit_window();
        }
    }

    /// Ступени размера (клетка, во сколько раз увеличить холст), от мелкой к крупной, и какая из них обычная.
    /// Клетка крупнее обычной — только если для неё есть детальный слой вида (иначе картинка рисовалась бы из
    /// грубой «целиком» и надписи считались бы в других единицах); дальше — обычный холст ровно ×2.
    fn zoom_levels(&self) -> (Vec<(i32, i32)>, usize) {
        let b0 = ((3 * self.k + 1) / 2).max(2);
        let base = format!("{}_{}", self.style.id(), self.size.id());
        let cs = self.whole_specs.get(&base).and_then(|s| s["cell_scale"].as_f64()).unwrap_or(1.0);
        let layer = |b: i32| self.layers.contains_key(&format!("{base}@{}", ((b as f64) * cs).round() as i32));
        let any = (2..=4).any(|c| self.layers.contains_key(&format!("{base}@{c}")));
        let ok = |b: i32| b == b0 || if any { layer(b) } else { b < b0 };
        let (lo, hi) = ((b0 * 2 + 2) / 3, b0 * 2);
        let mut v: Vec<(i32, i32)> = Vec::new();
        for m in 1..=2 {
            for b in 2..=hi {
                let size = b * m;
                if ok(b) && (lo..=hi).contains(&size) && !v.iter().any(|(vb, vm)| vb * vm == size) {
                    v.push((b, m));
                }
            }
        }
        v.sort_by_key(|(b, m)| b * m);
        let def = v.iter().position(|l| *l == (b0, 1)).unwrap_or(0);
        (v, def)
    }

    /// Нынешняя ступень размера (клетка, увеличение).
    fn zoom_level(&self) -> (i32, i32) {
        let (v, def) = self.zoom_levels();
        let i = (def as i32 + self.zoom).clamp(0, v.len() as i32 - 1) as usize;
        v.get(i).copied().unwrap_or((((3 * self.k + 1) / 2).max(2), 1))
    }

    /// Колесо мыши над виджетом: крупнее или мельче на одну ступень (пиксели остаются целыми — чётко).
    fn wheel_zoom(&mut self, dy: f64) {
        self.wheel += dy;
        if self.wheel.abs() < 1.0 {
            return;
        }
        let step = self.wheel.signum() as i32;
        self.wheel = 0.0;
        let (v, def) = self.zoom_levels();
        let z = (self.zoom + step).clamp(-(def as i32), v.len() as i32 - 1 - def as i32);
        if z == self.zoom {
            return;
        }
        self.zoom = z;
        crate::prefs::update(|v| v["widget_zoom"] = serde_json::json!(z));
        self.relayout();
        self.fit_window();
        self.dirty = true;
    }

    /// Окно и буфер — под текущий размер картинки.
    fn fit_window(&mut self) {
        let (w, h) = self.size();
        if let Some(win) = &self.window {
            let _ = win.request_inner_size(PhysicalSize::new(w as u32, h as u32));
        }
        if let (Some(s), Some(nw), Some(nh)) = (self.surface.as_mut(), NonZeroU32::new(w as u32), NonZeroU32::new(h as u32)) {
            let _ = s.resize(nw, nh);
        }
    }

    fn step(&mut self, _t: f64) {
        if let Some(rx) = &self.rx {
            let mut last = None;
            while let Ok(c) = rx.try_recv() {
                last = Some(c);
            }
            if let Some(c) = last {
                self.raw = c;
                self.ignored = crate::prefs::ignored();
                self.recount();
            }
        }
    }

    /// Нарисовать виджет в буфер (размер — self.size()) выбранным видом: пиксельный холст ×scale.
    pub fn paint(&mut self, buf: &mut [u32]) {
        let (p, hits) = self.canvas();
        let s = self.scale;
        let (w, h) = self.win_size;
        buf.fill(0);
        if p.w * s == w && p.h * s == h {
            px::blow_up(&p, s, buf, w);
        } else {
            // картинка не совпала с окном — перестроить окно под неё к следующему кадру (не оставлять пустым)
            self.refit = true;
        }
        self.hits = Hits { close: px::up(hits.close, s), cat: px::up(hits.cat, s), list: px::up(hits.list, s) };
        self.quip_over(buf, w, h);
    }

    /// Реплика кота — жёлтым облачком у кота.
    fn quip_over(&mut self, buf: &mut [u32], w: i32, h: i32) {
        let quip = self.quip.as_ref().filter(|(_, until)| Instant::now() < *until).map(|(q, _)| q.clone());
        if let Some(q) = quip.as_deref() {
            // реплика кота — жёлтым облачком у кота
            let mut c = crate::gfx::Canvas::new(buf, w as usize, w, h);
            // у монитора облачко — на экране, а не на коте: иначе закрывает мордочку, когда ухо тянут
            let at = if self.style == Style::Monitor { self.hits.list } else { self.hits.cat };
            look::quip_box(&mut c, &mut self.fonts, self.k, at, q);
            for p in buf.iter_mut() {
                if *p >> 24 == 0 && *p & 0xFF_FFFF != 0 {
                    *p |= 0xFF00_0000;
                }
            }
        }
    }

    /// Тычок в кота: клетка картинки под курсором → реакция и реплика.
    fn poke(&mut self, px: f64, py: f64) {
        // кот в виджете — пиксельный спрайт: тычок — реплика
        let _ = (px, py);
        if self.no_quips() {
            return;
        }
        if let Some(q) = self.quips.pick("волосы", "возмущение") {
            self.quip = Some((q.to_string(), Instant::now() + Duration::from_secs(4)));
            self.dirty = true;
        }
    }

    /// Маленькие виджеты — без реплик: облачко закрыло бы смущённую мордочку (кот там крошечный).
    fn no_quips(&self) -> bool {
        self.size == Size::Small
    }

    /// Реплика на ухо — из группы «ухо» (те же фразы, что у кота в окне).
    fn poke_ear(&mut self) {
        if self.no_quips() {
            return;
        }
        if let Some(q) = self.quips.pick("ухо_левое", "смущение") {
            self.quip = Some((q.to_string(), Instant::now() + Duration::from_secs(4)));
            self.dirty = true;
        }
    }

    fn start_checks(&mut self) {
        // общее расписание с окном Аптечки (sched.rs): проверяет кто-то один, второй читает его результат
        self.rx = Some(crate::sched::start());
    }

    /// Файл программы заменили (обновление) с момента запуска.
    fn updated(&self) -> bool {
        let Some((path, was)) = &self.exe else { return false };
        let now = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        now.is_some() && now != *was
    }

    fn save_pos(&self) {
        let Some(w) = self.window.as_ref() else { return };
        // свёрнутое окно Windows «стоит» в (-32000, -32000) — такое место не запоминаем
        if w.is_minimized() == Some(true) {
            return;
        }
        if let Some(p) = w.outer_position().ok().filter(|p| p.x > -20000 && p.y > -20000) {
            crate::prefs::update(|v| v["widget_pos"] = serde_json::json!([p.x, p.y]));
        }
    }

    fn redraw(&mut self) {
        let t0 = Instant::now();
        self.draw();
        self.stats.draws += 1;
        self.stats.draw_time += t0.elapsed();
        self.drawn = Some(self.sig());
        self.last_draw = Instant::now();
        self.dirty = false;
    }

    fn draw(&mut self) {
        #[cfg(windows)]
        {
            // Windows: «слоёное» окно с прозрачностью по пикселям (форма — по шаблону)
            let (w, h) = self.size();
            let mut buf = vec![0u32; (w * h) as usize];
            self.paint(&mut buf);
            // цвета под профиль монитора (широкий охват — иначе перенасыщено)
            if let Some(cm) = crate::colorman::get() {
                cm.apply_premul(&mut buf);
            }
            if let Some(win) = &self.window {
                layered_present(win, &buf, w, h);
            }
            return;
        }
        #[allow(unreachable_code)]
        let Some(mut surface) = self.surface.take() else {
            return;
        };
        if let Ok(mut buf) = surface.buffer_mut() {
            let (w, h) = self.size();
            if buf.len() == (w * h) as usize {
                self.paint(&mut buf);
                if let Some(cm) = crate::colorman::get() {
                    cm.apply_premul(&mut buf);
                }
                let _ = buf.present();
            }
        }
        self.surface = Some(surface);
    }
}

/// Путь к файлу программы и время его изменения. На Linux заменённый файл виден как «… (deleted)» —
/// отрезаем, чтобы смотреть на новый файл по тому же пути.
fn exe_stamp() -> Option<(std::path::PathBuf, Option<std::time::SystemTime>)> {
    let p = crate::procs::self_exe()?;
    let m = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
    Some((p, m))
}

/// Открыть полную Аптечку (если она уже открыта — та сама покажет окно).
fn open_main() {
    if let Some(exe) = crate::procs::self_exe() {
        crate::procs::spawn_detached(&mut std::process::Command::new(exe));
    }
}

impl ApplicationHandler for Widget {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // масштаб — как у монитора (на экране ×2 виджет вдвое крупнее), если не задан UI_K
        if std::env::var("UI_K").is_err() {
            let k = el.primary_monitor().map(|m| m.scale_factor().round() as i32).unwrap_or(1).clamp(1, 4);
            self.k = k;
        }
        // размер окна — под этот масштаб (иначе картинка не совпадёт с окном и виджет будет пустым)
        self.relayout();
        let (w, h) = self.size();
        let attrs = Window::default_attributes()
            .with_title(crate::i18n::t("Аптечка — виджет"))
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnBottom)
            .with_inner_size(PhysicalSize::new(w as u32, h as u32));
        // macOS: прозрачное окно и картинка с альфой (форма — по шаблону); Windows — «слоёное» окно, см. layered_present
        #[cfg(target_os = "macos")]
        let attrs = {
            softbuffer::MACOS_ALPHA.store(true, std::sync::atomic::Ordering::Relaxed);
            attrs.with_transparent(true)
        };
        #[cfg(target_os = "linux")]
        let attrs = {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};
            // «служебное» окно: без кнопки на панели задач и в Alt+Tab
            // прозрачное окно (форма — по шаблону: ушки, кот над монитором); GNOME сводит окна с прозрачностью
            attrs.with_x11_window_type(vec![WindowType::Utility]).with_name("io.github.aptechka", "aptechka-widget").with_transparent(true)
        };
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_skip_taskbar(true)
        };
        // сохранённое место — только если виджет на нём будет виден хотя бы на 80×80 на каком-то мониторе
        // (отключили второй монитор, сменили разрешение — иначе виджет «пропадал» за краем экрана)
        let pos = crate::prefs::load()
            .get("widget_pos")
            .and_then(|v| Some((v[0].as_i64()?, v[1].as_i64()?)))
            .filter(|&(x, y)| on_screen(el, x, y, w as i64, h as i64));
        let attrs = match pos {
            Some((x, y)) => attrs.with_position(PhysicalPosition::new(x as i32, y as i32)),
            None => attrs,
        };
        let Ok(window) = el.create_window(attrs) else { return };
        let window = Rc::new(window);
        let (Ok(ctx), Some(nw), Some(nh)) = (softbuffer::Context::new(window.clone()), NonZeroU32::new(w as u32), NonZeroU32::new(h as u32)) else {
            return;
        };
        let Ok(mut surface) = softbuffer::Surface::new(&ctx, window.clone()) else { return };
        let _ = surface.resize(nw, nh);
        self.surface = Some(surface);
        self.window = Some(window);
        self.start_checks();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        match ev {
            WindowEvent::Occluded(h) => {
                self.hidden = h;
                if !h {
                    // снова видно — кадр сразу
                    self.next = Instant::now();
                }
            }
            WindowEvent::CloseRequested => {
                if self.pos_dirty.take().is_some() {
                    self.save_pos();
                }
                el.exit()
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                self.dirty |= !self.hover;
                self.hover = true;
            }
            WindowEvent::CursorLeft { .. } => {
                self.dirty |= self.hover;
                self.hover = false;
                self.release_ear();
            }
            WindowEvent::CursorEntered { .. } => {
                self.dirty |= !self.hover;
                self.hover = true;
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let h = self.hits;
                let (x, y) = (self.cursor.x as i32, self.cursor.y as i32);
                if h.close.contains(x, y) {
                    self.close_down = true;
                } else if let Some(side) = self.ear_at(x, y) {
                    // ухо: тянется, пока кнопка нажата; кот жмурится и отвечает репликой, как на тычок
                    self.ear = Some(EarPull { side, t0: Instant::now(), rel: None });
                    self.dirty = true;
                    self.poke_ear();
                } else if h.cat.contains(x, y) {
                    self.poke(self.cursor.x, self.cursor.y);
                } else if h.list.contains(x, y) {
                    open_main();
                } else if let Some(w) = &self.window {
                    // остальное — «ручка»: тянуть виджет
                    let _ = w.drag_window();
                }
            }
            // правая кнопка — открыть Аптечку (там же меню «Вид» с видом виджета)
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => open_main(),
            // колесо — размер виджета (вверх — крупнее)
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y as f64,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.wheel_zoom(dy);
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.release_ear();
                if std::mem::take(&mut self.close_down) {
                    let close = self.hits.close;
                    if close.contains(self.cursor.x as i32, self.cursor.y as i32) {
                        el.exit();
                    }
                }
            }
            WindowEvent::Moved(_) => self.pos_dirty = Some(Instant::now()),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next {
            self.stats.ticks += 1;
            // загрузка процессора — раз в 2 с
            if self.load_t.elapsed() >= Duration::from_secs(2) {
                self.load_t = now;
                self.load_now = self.load.sample();
            }
            let t = self.t0.elapsed().as_secs_f64();
            self.step(t);
            if self.pos_dirty.is_some_and(|d| d.elapsed() >= Duration::from_millis(600)) {
                self.pos_dirty = None;
                self.save_pos();
            }
            if self.prefs_t.elapsed() >= Duration::from_secs(1) {
                self.prefs_t = Instant::now();
                self.reload_prefs();
                if self.updated() {
                    // программу заменили новой версией: запускаем новый виджет на том же месте, этот закрываем
                    self.save_pos();
                    if let Some((path, _)) = &self.exe {
                        let mut c = std::process::Command::new(path);
                        c.arg("--widget").env("APTECHKA_REPLACES", std::process::id().to_string());
                        if crate::procs::spawn_detached(&mut c) {
                            el.exit();
                        }
                    }
                }
            }
            if std::mem::take(&mut self.refit) {
                self.relayout();
                self.fit_window();
                self.dirty = true;
            }
            // ухо держат слишком долго — отпускаем; вернулось совсем — забываем
            if let Some(e) = self.ear {
                if e.rel.is_none() && e.t0.elapsed().as_secs_f64() >= anim::EAR_HOLD_MAX {
                    self.release_ear();
                } else if e.rel.is_some_and(|r| r.elapsed().as_secs_f64() > anim::SHY_AFTER.max(0.5)) {
                    self.ear = None;
                }
            }
            // Windows не присылает Occluded для свёрнутого окна — спрашиваем (на X11 это запрос к серверу, там нет)
            let minimized = cfg!(windows) && self.window.as_ref().and_then(|w| w.is_minimized()) == Some(true);
            self.check_covered();
            let idle = self.hidden || minimized || self.covered;
            if idle {
                self.dirty = true;
            } else if let Some(w) = self.window.as_ref() {
                // новый кадр — только если что-то изменилось (оживление шагает целыми точками: чаще всего ничего)
                // или раз в секунду (часы, данные, ЭЛТ)
                if self.dirty || self.last_draw.elapsed() >= Duration::from_secs(1) || self.drawn.as_ref() != Some(&self.sig()) {
                    w.request_redraw();
                }
            }
            if self.log {
                let since = *self.stats.since.get_or_insert(now);
                if since.elapsed() >= Duration::from_secs(10) {
                    let s = &self.stats;
                    let (el_s, ms) = (since.elapsed().as_secs_f64(), s.draw_time.as_secs_f64() * 1000.0 / s.draws.max(1) as f64);
                    eprintln!("[виджет] за {el_s:.0} с: отсчётов {}, кадров {}, рисование {ms:.1} мс/кадр", s.ticks, s.draws);
                    self.stats = Stats { since: Some(now), ..Stats::default() };
                }
            }
            // прячется — раз в секунду; иначе ритм оживления, если оно есть, а нет — раз в секунду
            let mut tick = if idle || !(self.caps.any() || self.crt_on) { 1.0 } else { 1.0 / anim::FPS };
            // моргание быстрее шага: проснуться точно к смене его кадра
            if !idle && self.caps.blink && self.t_fixed.is_none() {
                if let Some(e) = anim::blink_next_edge(self.anim_time()) {
                    tick = tick.min(e + 0.001);
                }
            }
            self.next = now + Duration::from_secs_f64(tick);
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.next));
    }
}

/// Виден ли прямоугольник (x, y, w, h) на каком-нибудь мониторе хотя бы на 80×80 точек.
/// Мониторы не известны (некоторые оконные системы не говорят) — считаем, что виден.
fn on_screen(el: &ActiveEventLoop, x: i64, y: i64, w: i64, h: i64) -> bool {
    let mons: Vec<_> = el.available_monitors().collect();
    if mons.is_empty() {
        return true;
    }
    mons.iter().any(|m| {
        let (p, s) = (m.position(), m.size());
        let ix = (x + w).min(p.x as i64 + s.width as i64) - x.max(p.x as i64);
        let iy = (y + h).min(p.y as i64 + s.height as i64) - y.max(p.y as i64);
        ix >= 80.min(w) && iy >= 80.min(h)
    })
}

/// Ступень размера из настроек (ui.json widget_zoom: на сколько ступеней от обычной; пределы — zoom_levels).
/// На macOS виджет системный (WidgetKit), размер колесом там не нужен.
fn zoom_pref() -> i32 {
    if cfg!(target_os = "macos") {
        return 0;
    }
    crate::prefs::load().get("widget_zoom").and_then(|v| v.as_i64()).unwrap_or(0).clamp(-4, 4) as i32
}

/// Окно просит открытый виджет показаться поверх окон: метка в папке сеанса (не в ui.json — это не
/// настройка, и постоянный файл с fsync на каждый щелчок незачем). Наносекунды — два щелчка в одну
/// секунду тоже разные просьбы.
pub fn request_show() {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let _ = crate::store::write_private_quick(&crate::store::runtime_dir().join("widget.show"), t.to_string().as_bytes());
}

fn show_request() -> i64 {
    crate::store::read_capped(&crate::store::runtime_dir().join("widget.show"), 64)
        .ok()
        .and_then(|t| t.trim().parse::<u128>().ok())
        .map_or(0, |n| (n % i64::MAX as u128) as i64)
}

/// Замок «виджет один» (widget.lock в папке сеанса). None — уже открыт другой виджет.
/// Перезапуск новой версией (APTECHKA_REPLACES) ждёт до 5 с, пока старый закроется и отпустит замок.
/// Замок не создаётся (испорчена папка) — по-старому, по списку процессов: лучше так, чем без виджета.
fn one_widget() -> Option<Option<crate::store::lock::Held>> {
    use crate::store::lock::{take, Take};
    let p = crate::store::runtime_dir().join("widget.lock");
    let old: i64 = std::env::var("APTECHKA_REPLACES").ok().and_then(|v| v.parse().ok()).unwrap_or(-1);
    let until = Instant::now() + Duration::from_secs(if old >= 0 { 5 } else { 0 });
    loop {
        match take(&p) {
            Take::Got(h) => return Some(Some(h)),
            Take::Busy if Instant::now() < until => std::thread::sleep(Duration::from_millis(100)),
            Take::Busy => return None,
            Take::Broken(_) => {
                let me = std::process::id() as i64;
                let other = crate::procs::list().iter().any(|p| p.pid != me && p.pid != old && p.is_our("--widget"));
                return (!other).then_some(None);
            }
        }
    }
}

pub fn run() {
    // виджет один: если уже открыт (ярлык нажали дважды, меню и ярлык разом) — второй не нужен. Замок, а не
    // поиск по списку процессов: два запуска разом не могут ни оба уйти, ни оба остаться. Держится до выхода.
    let Some(_one) = one_widget() else {
        eprintln!("[виджет] уже открыт");
        return;
    };
    let el = match EventLoop::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[виджет] нет экрана: {e}");
            return;
        }
    };
    // масштаб интерфейса: на экранах ×2 — вдвое крупнее
    let k = std::env::var("UI_K").ok().and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 4);
    // сырые события мыши и клавиатуры со всего экрана виджету не нужны (свои нажатия приходят как обычно):
    // иначе каждое движение мыши где угодно будит его — сотню раз в секунду, даже когда он спит
    el.listen_device_events(winit::event_loop::DeviceEvents::Never);
    let mut w = Widget::new(k);
    let _ = el.run_app(&mut w);
}

/// WIDGET_SHOT: нарисовать виджет в PNG без окна (с настоящими проверками или UI_FAKE).
pub fn shot(path: &str) {
    let k = std::env::var("UI_K").ok().and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 4);
    let mut w = Widget::new(k);
    let checks = match std::env::var("UI_FAKE") {
        Ok(kind) => crate::ui::demo_checks(&kind),
        Err(_) => diag::run_all(),
    };
    w.rows = summarize(&checks);
    w.checks = checks.clone();
    // WIDGET_MAC=папка — widget.json для виджета macOS (что Аптечка пишет на Маке), для проверки на любой системе
    if let Ok(dir) = std::env::var("WIDGET_MAC") {
        let _ = std::fs::create_dir_all(&dir);
        let d = std::path::Path::new(&dir);
        let _ = std::fs::write(d.join("widget.json"), mac_state(&checks, std::env::var_os("WIDGET_CONCEPT").is_some(), Some(d)));
        eprintln!("[виджет] widget.json для macOS → {dir}");
        return;
    }
    w.load_now = {
        w.load.sample();
        std::thread::sleep(Duration::from_millis(300));
        w.load.sample()
    };
    for i in 0..12 {
        w.step(i as f64 / FPS);
    }
    if let Ok(s) = std::env::var("WIDGET_STYLE") {
        w.style = Style::from_id(&s);
    }
    if let Ok(s) = std::env::var("WIDGET_SIZE") {
        w.size = look::fit_size(w.style, Size::from_id(&s));
    }
    // WIDGET_CONCEPT=1 — данные ровно как на концептах ChatGPT (чтобы сравнить снимок с концептом точка в точку)
    // (WIDGET_CONCEPT=warn или bad — те же строки, но ИИ «внимание», VPN «беда»: проверить перекраску значков)
    if let Ok(kind) = std::env::var("WIDGET_CONCEPT") {
        let ok = |s: &str| (s.to_string(), "ok".to_string());
        w.fixed = Some(TplData {
            rows: [ok("Сеть — есть"), ok("VPN — вкл"), ok("ИИ — Claude думает"), ok("ЦП 12%"), ok("Диск — 220 ГБ")],
            total: ok("Всё в порядке"),
            clock: "17:52".into(),
            cpu: Some(12.0),
            disk_used: Some(45.0),
            state: 0,
        });
        if let Some(f) = w.fixed.as_mut().filter(|_| kind == "warn" || kind == "bad") {
            f.rows[2].1 = "warn".into();
            if kind == "bad" {
                f.rows[1] = ("VPN — выкл".into(), "bad".into());
                f.total = ("Беда: VPN".into(), "bad".into());
                f.state = 2;
            } else {
                f.total = ("Внимание: ИИ".into(), "warn".into());
                f.state = 1;
            }
        }
    }
    // WIDGET_ANIM_T=секунды — кадр оживления в этот момент; WIDGET_EAR=l|r[:ступень 0..4] — потянутое ухо
    w.t_fixed = std::env::var("WIDGET_ANIM_T").ok().and_then(|v| v.parse().ok());
    if let Ok(v) = std::env::var("WIDGET_EAR") {
        let (side, lvl) = v.split_once(':').unwrap_or((&v, "4"));
        w.ear_fixed = Some((usize::from(side == "r"), lvl.parse().unwrap_or(4)));
    }
    w.relayout();
    // WIDGET_POKE=1 — тычок в середину кота перед снимком, чтобы увидеть реакцию и реплику
    let (ww, hh) = w.size();
    let mut buf = vec![0u32; (ww * hh) as usize];
    w.paint(&mut buf);
    if std::env::var("WIDGET_POKE").is_ok() {
        let r = w.hits.cat;
        w.poke((r.x + r.w / 2) as f64, (r.y + r.h / 3) as f64);
        for i in 12..20 {
            w.step(i as f64 / FPS + 0.4);
        }
        w.paint(&mut buf);
    }
    // WIDGET_SHOT_CM=1 — снимок уже в цветах монитора (как на экране), для проверки управления цветом
    if std::env::var_os("WIDGET_SHOT_CM").is_some() {
        if let Some(cm) = crate::colorman::get() {
            cm.apply_premul(&mut buf);
        }
    }
    save_png(path, &buf, ww, hh);
    eprintln!("[виджет] снимок {ww}×{hh} → {path}");
}

/// PNG с прозрачностью (буфер — ARGB с предумноженной альфой).
fn save_png(path: &str, buf: &[u32], ww: i32, hh: i32) {
    let mut rgb = Vec::with_capacity(buf.len() * 4);
    for p in buf {
        let a = p >> 24;
        let un = |c: u32| ((c & 255) * 255).checked_div(a).map_or(0, |v| v.min(255) as u8);
        rgb.extend_from_slice(&[un(p >> 16), un(p >> 8), un(*p), a as u8]);
    }
    let Ok(f) = std::fs::File::create(path) else { return };
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), ww as u32, hh as u32);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let _ = e.write_header().and_then(|mut wr| wr.write_image_data(&rgb));
}

/// macOS: записать состояние для виджета WidgetKit — в контейнер расширения (его «дом» в песочнице)
/// и копию в ~/Library/Application Support/aptechka. Формат — macos/widget/AptechkaWidget.swift (State).
/// Виджет на Маке собирает сам SwiftUI (родное «стекло», шрифт и значки системы, отступы WidgetKit):
/// отсюда — только данные. lines — пять строк ровно как в виджете на Linux и Windows («Сеть — есть», статус),
/// total — итог («Всё в порядке» / «Беда: VPN»).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn export_mac(all: &[Check]) {
    let home = crate::store::home_dir().unwrap_or_default();
    // контейнер создаёт сама система при первом запуске виджета — пишем, только если он уже есть
    let data = home.join("Library/Containers/io.github.aptechka.widget/Data");
    // виджет собирает SwiftUI (кот + строки): картинки целиком не нужны — только данные
    let j = mac_state(all, false, None);
    if data.is_dir() {
        let _ = crate::store::write_private(&data.join("widget.json"), j.as_bytes());
        for old in ["widget_small.png", "widget_medium.png"] {
            let _ = std::fs::remove_file(data.join(old));
        }
    }
    // копия в ~/Library/Application Support/aptechka — это и есть папка настроек: private_dir() создаёт её 0700 и
    // проверяет владельца (чужая папка или ссылка — не пишем)
    let _ = crate::store::write_private(&crate::store::private_dir().join("widget.json"), j.as_bytes());
}

/// widget.json для виджета macOS. concept — данные как на концепте (для сверки: WIDGET_MAC=… WIDGET_CONCEPT=1).
/// dir — нарисовать туда и картинки widget_small.png / widget_medium.png: «Стекло» во всю плашку
/// (Retina: малая 324×324, средняя 688×324 пикселя; Swift вписывает их в плашку целиком, без обрезки).
fn mac_state(all: &[Check], concept: bool, dir: Option<&std::path::Path>) -> String {
    use std::cell::RefCell;
    thread_local! {
        static W: RefCell<Option<Widget>> = const { RefCell::new(None) };
    }
    let rows = summarize(all);
    let state = if all.iter().any(|c| c.status == "bad") {
        "bad"
    } else if all.iter().any(|c| c.status == "warn") {
        "warn"
    } else if crate::diag::unchecked_count(all) > 0 {
        // ни бед, ни предупреждений, но часть проверок не смогла выполниться (Swift показывает нейтральный значок)
        "unknown"
    } else {
        "ok"
    };
    let d = W.with(|cell| {
        let mut slot = cell.borrow_mut();
        let w = slot.get_or_insert_with(|| Widget::new(2));
        w.rows = rows.clone();
        w.checks = all.to_vec();
        w.load_now = w.load.sample();
        let d = if concept { mac_concept_data().unwrap_or_else(|| w.tpl_data()) } else { w.tpl_data() };
        if let Some(dir) = dir {
            w.style = Style::Glass;
            w.fixed = concept.then(|| d.clone());
            for size in [Size::Small, Size::Medium] {
                w.size = size;
                let (pw, ph) = if size == Size::Small { (324, 324) } else { (688, 324) };
                let (p, _) = w.canvas();
                let (sw, sh) = (p.w, p.h);
                let mut src = vec![0u32; (sw * sh) as usize];
                px::blow_up(&p, 1, &mut src, sw);
                let buf = fill_plaque(&src, sw, sh, pw, ph, size == Size::Medium);
                let tmp = dir.join(format!("widget_{}.tmp.png", size.id()));
                save_png(&tmp.to_string_lossy(), &buf, pw, ph);
                let _ = std::fs::rename(&tmp, dir.join(format!("widget_{}.png", size.id())));
            }
            w.fixed = None;
        }
        d
    });
    let st = if concept { "ok" } else { state };
    let tr = crate::i18n::t;
    serde_json::json!({
        "updated": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
        "state": st,
        "style": "glass",
        // текст — уже на языке интерфейса (i18n.rs): SwiftUI-виджет показывает его как есть
        "lang": crate::i18n::lang().id(),
        "rows": rows.iter().map(|r| serde_json::json!({"label": tr(r.label), "status": r.status, "value": tr(&r.value)})).collect::<Vec<_>>(),
        "lines": d.rows.iter().map(|(t, s)| serde_json::json!({"text": tr(t), "status": s})).collect::<Vec<_>>(),
        "total": {"text": tr(&d.total.0), "status": d.total.1},
    })
    .to_string()
}

/// Картинка виджета (ARGB, предумноженная) → плашка pw×ph без прозрачности. Скругление стекла заполняется
/// ближайшим цветом изнутри, его рамка (RIM точек) срезается, середина вписывается в плашку по высоте
/// (чуть уменьшается: на Маке плашка шире и ниже рисунка), остаток по ширине — продолжение фона стекла.
/// left — прижать к левому краю (у среднего кот касается левого края стекла — там продолжать нечем).
fn fill_plaque(src: &[u32], sw: i32, sh: i32, pw: i32, ph: i32, left: bool) -> Vec<u32> {
    const RIM: i32 = 10;
    // 1) без прозрачности: полупрозрачное — ближайшая непрозрачная точка к середине строки
    let mut op = src.to_vec();
    for y in 0..sh {
        for x in 0..sw {
            if src[(y * sw + x) as usize] >> 24 != 255 {
                let step = if x < sw / 2 { 1 } else { -1 };
                let mut xx = x;
                while xx > 0 && xx < sw - 1 && src[(y * sw + xx) as usize] >> 24 != 255 {
                    xx += step;
                }
                op[(y * sw + x) as usize] = src[(y * sw + xx) as usize] | 0xFF00_0000;
            }
        }
    }
    // 2) середина без рамки, вписанная по высоте
    let (iw, ih) = (sw - 2 * RIM, sh - 2 * RIM);
    let k = (ph as f64 / ih as f64).min(pw as f64 / iw as f64);
    let dw = (iw as f64 * k).round() as i32;
    let x0 = if left { 0 } else { (pw - dw) / 2 };
    let at = |x: f64, y: f64| -> [f64; 3] {
        // двулинейно по середине
        let (x, y) = (x.clamp(0.0, (iw - 1) as f64), y.clamp(0.0, (ih - 1) as f64));
        let (xi, yi) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - xi as f64, y - yi as f64);
        let g = |xx: i32, yy: i32| {
            let v = op[((yy.min(ih - 1) + RIM) * sw + xx.min(iw - 1) + RIM) as usize];
            [((v >> 16) & 255) as f64, ((v >> 8) & 255) as f64, (v & 255) as f64]
        };
        let (a, b, c, d) = (g(xi, yi), g(xi + 1, yi), g(xi, yi + 1), g(xi + 1, yi + 1));
        let mut o = [0.0; 3];
        for i in 0..3 {
            o[i] = a[i] * (1.0 - fx) * (1.0 - fy) + b[i] * fx * (1.0 - fy) + c[i] * (1.0 - fx) * fy + d[i] * fx * fy;
        }
        o
    };
    // продолжение за правым краем рисунка: его последний столбец, сглаженный по высоте (иначе края плашек
    // строк тянулись горизонтальными полосками)
    let edge: Vec<[f64; 3]> = {
        let col: Vec<[f64; 3]> = (0..ph).map(|y| at((dw - 1) as f64 / k, (y as f64 + 0.5) / k - 0.5)).collect();
        let r = 24i32;
        (0..ph)
            .map(|y| {
                let (a, b) = ((y - r).max(0), (y + r).min(ph - 1));
                let n = (b - a + 1) as f64;
                let mut s = [0.0; 3];
                for yy in a..=b {
                    for i in 0..3 {
                        s[i] += col[yy as usize][i] / n;
                    }
                }
                s
            })
            .collect()
    };
    let mut out = vec![0u32; (pw * ph) as usize];
    for y in 0..ph {
        for x in 0..pw {
            if x - x0 >= dw {
                let c = edge[y as usize];
                let ch = |v: f64| (v.round() as i64).clamp(0, 255) as u32;
                out[(y * pw + x) as usize] = 0xFF00_0000 | ch(c[0]) << 16 | ch(c[1]) << 8 | ch(c[2]);
                continue;
            }
            // за краем рисунка — его крайний столбец (фон стекла)
            let xs = (x - x0).clamp(0, dw - 1);
            let c = at((xs as f64 + 0.5) / k - 0.5, (y as f64 + 0.5) / k - 0.5);
            let ch = |v: f64| (v.round() as i64).clamp(0, 255) as u32;
            out[(y * pw + x) as usize] = 0xFF00_0000 | ch(c[0]) << 16 | ch(c[1]) << 8 | ch(c[2]);
        }
    }
    out
}

/// Данные как на концепте — для сверки виджета macOS с концептом.
fn mac_concept_data() -> Option<TplData> {
    let ok = |s: &str| (s.to_string(), "ok".to_string());
    Some(TplData {
        rows: [ok("Сеть — есть"), ok("VPN — вкл"), ok("ИИ — Claude думает"), ok("ЦП 12%"), ok("Диск — 220 ГБ")],
        total: ok("Всё в порядке"),
        clock: "17:52".into(),
        cpu: Some(12.0),
        disk_used: Some(45.0),
        state: 0,
    })
}

/// Windows: закрыт ли виджет другими окнами целиком. В 9×9 точках его прямоугольника спрашиваем, какое окно
/// там сверху (WindowFromPoint, верхнее окно-предок). Видна хоть одна точка самого виджета — не закрыт.
/// Прозрачные места «слоёного» окна пропускают насквозь — такие точки просто не засчитываются.
/// None — не узнать (нет описателя окна).
#[cfg(windows)]
pub(crate) fn covered_windows(win: &Window) -> Option<bool> {
    use core::ffi::c_void;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    type H = *mut c_void;
    #[repr(C)]
    struct Rc {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    }
    #[repr(C)]
    struct Pt {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetWindowRect(h: H, r: *mut Rc) -> i32;
        fn WindowFromPoint(p: Pt) -> H;
        fn GetAncestor(h: H, flags: u32) -> H;
    }
    const GA_ROOT: u32 = 2;
    let RawWindowHandle::Win32(h) = win.window_handle().ok()?.as_raw() else { return None };
    let me = h.hwnd.get() as H;
    let mut r = Rc { l: 0, t: 0, r: 0, b: 0 };
    // SAFETY: me — живое окно этого процесса, r — наша структура RECT
    if unsafe { GetWindowRect(me, &mut r) } == 0 || r.r <= r.l || r.b <= r.t {
        return None;
    }
    let (w, h) = (r.r - r.l, r.b - r.t);
    for i in 0..9 {
        for j in 0..9 {
            let p = Pt { x: r.l + (w - 1) * i / 8, y: r.t + (h - 1) * j / 8 };
            // SAFETY: обычные вызовы user32 с точкой экрана; описатели только сравниваем
            let top = unsafe { GetAncestor(WindowFromPoint(p), GA_ROOT) };
            if top == me {
                return Some(false);
            }
        }
    }
    Some(true)
}

/// Windows: показать буфер (ARGB, предумноженная альфа) «слоёным» окном — прозрачные места окна
/// не видны и пропускают щелчки насквозь, как у виджетов любой формы.
#[cfg(windows)]
fn layered_present(win: &Window, buf: &[u32], w: i32, h: i32) {
    use core::ffi::c_void;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    type H = *mut c_void;
    #[repr(C)]
    struct Bih {
        size: u32,
        w: i32,
        h: i32,
        planes: u16,
        bits: u16,
        compr: u32,
        img: u32,
        xpm: i32,
        ypm: i32,
        used: u32,
        imp: u32,
    }
    #[repr(C)]
    struct Blend {
        op: u8,
        flags: u8,
        alpha: u8,
        fmt: u8,
    }
    #[repr(C)]
    struct Pt {
        x: i32,
        y: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetDC(h: H) -> H;
        fn ReleaseDC(h: H, dc: H) -> i32;
        fn GetWindowLongPtrW(h: H, i: i32) -> isize;
        fn SetWindowLongPtrW(h: H, i: i32, v: isize) -> isize;
        fn UpdateLayeredWindow(h: H, dst: H, pos: *const Pt, size: *const Pt, src: H, sp: *const Pt, key: u32, b: *const Blend, flags: u32) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateCompatibleDC(dc: H) -> H;
        fn CreateDIBSection(dc: H, bmi: *const Bih, usage: u32, bits: *mut *mut c_void, sect: H, off: u32) -> H;
        fn SelectObject(dc: H, o: H) -> H;
        fn DeleteObject(o: H) -> i32;
        fn DeleteDC(dc: H) -> i32;
    }
    let Ok(handle) = win.window_handle() else { return };
    let RawWindowHandle::Win32(h32) = handle.as_raw() else { return };
    let hwnd = h32.hwnd.get() as H;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_LAYERED: isize = 0x80000;
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if ex & WS_EX_LAYERED == 0 {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED);
        }
        let screen = GetDC(std::ptr::null_mut());
        let mem = CreateCompatibleDC(screen);
        let bih = Bih { size: 40, w, h: -h, planes: 1, bits: 32, compr: 0, img: 0, xpm: 0, ypm: 0, used: 0, imp: 0 };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bmp = CreateDIBSection(screen, &bih, 0, &mut bits, std::ptr::null_mut(), 0);
        if !bmp.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(buf.as_ptr(), bits as *mut u32, buf.len());
            let old = SelectObject(mem, bmp);
            let size = Pt { x: w, y: h };
            let src = Pt { x: 0, y: 0 };
            let blend = Blend { op: 0, flags: 0, alpha: 255, fmt: 1 }; // AC_SRC_OVER, AC_SRC_ALPHA
            UpdateLayeredWindow(hwnd, screen, std::ptr::null(), &size, mem, &src, 0, &blend, 2); // ULW_ALPHA
            SelectObject(mem, old);
            DeleteObject(bmp);
        }
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
    }
}

#[cfg(test)]
mod unchecked_tests {
    use super::*;
    use crate::diag::{Check, INFO};

    fn total(all: &[Check]) -> (String, String) {
        let r = summarize(all).pop().unwrap();
        (r.status, r.value)
    }

    #[test]
    fn total_not_green_when_something_unchecked() {
        let ok = Check::new("sys", "Память", "ok", "40%");
        let cant = Check::new("sys", "Диск", INFO, "не проверить").unknown();
        assert_eq!(total(std::slice::from_ref(&ok)), ("ok".into(), "всё в порядке".into()));
        assert_eq!(total(&[ok.clone(), cant.clone()]), ("info".into(), "не всё проверено".into()));
        // обычная строка «для сведения» итог не портит
        assert_eq!(total(&[ok.clone(), Check::new("net", "Часовой пояс", INFO, "UTC")]).0, "ok");
        // беда сильнее
        assert_eq!(total(&[Check::new("sys", "ЦП", "bad", "99%"), cant]).0, "bad");
    }

    #[test]
    fn mac_state_unknown() {
        let all = [Check::new("sys", "Память", "ok", "40%"), Check::new("sys", "Диск", INFO, "не проверить").unknown()];
        let j: serde_json::Value = serde_json::from_str(&mac_state(&all, false, None)).unwrap();
        assert_eq!(j["state"], "unknown");
    }
}

#[cfg(test)]
mod lock_tests {
    /// Второй виджет не запускается, пока первый держит замок; после выхода первого — запускается.
    #[cfg(unix)]
    #[test]
    fn one_widget_lock() {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("aptechka_onew_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::set_permissions(&root, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        let old = std::env::var_os("XDG_RUNTIME_DIR");
        std::env::set_var("XDG_RUNTIME_DIR", &root);
        let first = super::one_widget();
        assert!(matches!(first, Some(Some(_))), "первый берёт замок");
        assert!(super::one_widget().is_none(), "второй при живом первом — нет");
        drop(first);
        assert!(super::one_widget().is_some(), "первый закрылся — можно");
        match old {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
