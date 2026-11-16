//! Экраны в картинке: главный монитор, «Сеть», «Система», экран робота (Claude ON/OFF)
//! и часы на роботе-телевизоре. Как в прежней версии (aptechka/screens.py), но поверх
//! слоёной картинки на сетке.
//!
//! Экран рисуется в координатах старого исходника B1 (там заданы его четырёхугольники),
//! а на окно переносится линейным пересчётом «пиксель исходника → клетка → пиксель окна»
//! (art/layers/экраны.json). Видны только клетки «стекла» экрана (art/layers/экраны.png):
//! листья и провода остаются поверх текста. Помехи кинескопа: сканлайны, катящаяся
//! светлая полоса, «снег», редкие сбои.

use crate::gfx::Face;
use skrifa::outline::OutlinePen;
use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Rect, SpreadMode, Transform};

// тот же пиксельный шрифт, что у окна (один экземпляр в программе)
const FONT: &[u8] = crate::gfx::FONT_PIXEL;
const INK: (f32, f32, f32) = (0.04, 0.15, 0.25);
const DIGITS: [[&str; 5]; 10] = [
    ["111", "101", "101", "101", "111"],
    ["010", "110", "010", "010", "111"],
    ["111", "001", "111", "100", "111"],
    ["111", "001", "111", "001", "111"],
    ["101", "101", "111", "001", "001"],
    ["111", "100", "111", "001", "111"],
    ["111", "100", "111", "101", "111"],
    ["111", "001", "010", "010", "010"],
    ["111", "101", "111", "101", "111"],
    ["111", "101", "111", "001", "111"],
];

fn st_color(st: &str) -> (f32, f32, f32) {
    match st {
        "ok" => (0.05, 0.55, 0.29),
        "warn" => (0.7, 0.45, 0.0),
        "bad" => (0.78, 0.1, 0.22),
        "info" => (0.11, 0.37, 0.63),
        "checking" => (0.35, 0.3, 0.75),
        _ => (0.4, 0.45, 0.5),
    }
}

/// Помехи кинескопа: (период полосы, сдвиг фазы, период сбоя)
fn crt(name: &str) -> Option<(f64, f64, f64)> {
    match name {
        "main" => Some((7.0, 0.0, 9.7)),
        "net" => Some((5.5, 0.37, 11.3)),
        "sys" => Some((6.3, 0.71, 13.1)),
        _ => None,
    }
}

/// Что показывать на экранах. Заполняется из диагностики.
#[derive(Clone)]
pub struct ScreenState {
    /// сообщение кота (печатается по буквам) и с какого момента
    pub message: String,
    pub message_t: f64,
    /// настроение: ok / warn / bad — красит сообщение
    pub mood: String,
    pub vpn: (String, String),
    pub claude: (String, String),
    /// скорость: (вниз, вверх, пинг)
    pub speed: Option<(f64, f64, f64)>,
    pub net_rows: Vec<(String, String, String)>,
    pub sys_rows: Vec<(String, String, String)>,
    /// идёт замер скорости: этап (0 пинг, 1 скачивание, 2 отдача) и текущее значение (мс или Мбит/с) —
    /// экран «Сеть» становится спидометром со стрелкой
    pub speed_live: Option<(u8, f64)>,
}

impl Default for ScreenState {
    fn default() -> Self {
        let chk = || ("checking".to_string(), "…".to_string());
        ScreenState {
            message: "Проверяю, мур…".into(),
            message_t: 0.0,
            mood: "ok".into(),
            vpn: chk(),
            claude: chk(),
            speed: None,
            net_rows: vec![],
            sys_rows: vec![],
            speed_live: None,
        }
    }
}

/// Простой генератор случайных чисел для «снега».
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn int(&mut self, a: i64, b: i64) -> i64 {
        a + (self.next() % ((b - a + 1).max(1) as u64)) as i64
    }
}

struct Outline<'a> {
    pb: &'a mut PathBuilder,
    x: f32,
    y: f32,
    s: f32,
}
impl OutlinePen for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.x + x * self.s, self.y - y * self.s)
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.x + x * self.s, self.y - y * self.s)
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.x + x1 * self.s, self.y - y1 * self.s, self.x + x * self.s, self.y - y * self.s)
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.x + x1 * self.s,
            self.y - y1 * self.s,
            self.x + x2 * self.s,
            self.y - y2 * self.s,
            self.x + x * self.s,
            self.y - y * self.s,
        )
    }
    fn close(&mut self) {
        self.pb.close()
    }
}

/// Рисовалка одного экрана.
struct Painter<'a> {
    pm: &'a mut Pixmap,
    tr: Transform,
    face: &'a Face,
    size: f32,
}

impl Painter<'_> {
    fn paint(c: (f32, f32, f32), a: f32) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(Color::from_rgba(c.0, c.1, c.2, a).unwrap());
        p.anti_alias = false;
        p
    }
    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: (f32, f32, f32)) {
        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.line_to(x1, y1);
        if let Some(path) = pb.finish() {
            let stroke = tiny_skia::Stroke { width, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
            self.pm.stroke_path(&path, &Self::paint(c, 1.0), &stroke, self.tr, None);
        }
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: (f32, f32, f32), a: f32) {
        if let Some(r) = Rect::from_xywh(x, y, w, h) {
            self.pm.fill_rect(r, &Self::paint(c, a), self.tr, None);
        }
    }
    fn rects(&mut self, rs: &[(f32, f32, f32, f32)], c: (f32, f32, f32), a: f32) {
        let mut pb = PathBuilder::new();
        for &(x, y, w, h) in rs {
            if let Some(r) = Rect::from_xywh(x, y, w, h) {
                pb.push_rect(r);
            }
        }
        if let Some(p) = pb.finish() {
            self.pm.fill_path(&p, &Self::paint(c, a), FillRule::Winding, self.tr, None);
        }
    }
    fn width(&self, text: &str) -> f32 {
        let s = self.size / self.face.upm;
        text.chars().map(|c| self.face.glyph_index(c).and_then(|g| self.face.advance(g)).unwrap_or(0.0) * s).sum()
    }
    fn text(&mut self, x: f32, y: f32, text: &str, c: (f32, f32, f32)) {
        let s = self.size / self.face.upm;
        let mut pb = PathBuilder::new();
        let mut ox = x;
        for ch in text.chars() {
            let Some(g) = self.face.glyph_index(ch) else { continue };
            self.face.outline(g, &mut Outline { pb: &mut pb, x: ox, y, s });
            ox += self.face.advance(g).unwrap_or(0.0) * s;
        }
        if let Some(p) = pb.finish() {
            let mut paint = Self::paint(c, 1.0);
            paint.anti_alias = true; // шрифт пиксельный, но экран наклонён — мягкие края ровнее
            self.pm.fill_path(&p, &paint, FillRule::Winding, self.tr, None);
        }
    }
    fn dot(&mut self, x: f32, y: f32, st: &str, t: f64) {
        if st == "bad" && blink3(t) {
            return; // мигает
        }
        self.rect(x - 5., y - 5., 10., 10., st_color(st), 1.0);
    }
    fn wrap(&self, text: &str, maxw: f32) -> Vec<String> {
        let mut lines = Vec::new();
        let mut cur = String::new();
        for word in text.split_whitespace() {
            let test = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if self.width(&test) > maxw && !cur.is_empty() {
                lines.push(std::mem::replace(&mut cur, word.to_string()));
            } else {
                cur = test;
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }
}

struct Screen {
    name: String,
    id: u8,
    quad: [(f32, f32); 3],
    /// рамка экрана в пикселях окна
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    layer: Pixmap,
    /// содержимое без помех кинескопа (текст, значки): дорогое, перерисовывается только при смене ключа
    content: Pixmap,
    ckey: Option<u64>,
    /// ключ всего слоя: всё, от чего зависят его пиксели
    fkey: Option<u64>,
}

pub struct Screens {
    screens: Vec<Screen>,
    mask: Vec<u8>, // номер экрана в каждой клетке (0 — нет)
    cw: usize,     // ширина картинки в клетках
    map: [f32; 4], // клетка = a·пиксель исходника + b (x, затем y)
    scale: usize,
    /// строки клеток, на которых есть экраны (их перерисовываем каждый кадр)
    pub rows: Vec<bool>,
    /// счётчики для журнала: слоёв перерисовано / проверено
    pub redrawn: u64,
    pub checked: u64,
}

fn hhmm() -> String {
    if let Ok(c) = std::env::var("CLOCK") {
        return c;
    }
    let tm = crate::ui::localtime(crate::ui::now_secs());
    format!("{:02}:{:02}", tm.hour, tm.min)
}

impl Screens {
    /// dir — art/layers; cw, ch — размер картинки в клетках; scale — клетка в пикселях окна.
    pub fn load(dir: &str, cw: usize, ch: usize, scale: usize) -> Option<Screens> {
        let meta: serde_json::Value = serde_json::from_str(&crate::art::read_string(dir, "экраны.json")?).ok()?;
        let m = meta["map"].as_array()?;
        let map = [m[0].as_f64()? as f32, m[1].as_f64()? as f32, m[2].as_f64()? as f32, m[3].as_f64()? as f32];
        // маска: R-канал png
        let dec = png::Decoder::new(std::io::Cursor::new(crate::art::read(dir, "экраны.png")?));
        let mut rd = dec.read_info().ok()?;
        let mut buf = vec![0; rd.output_buffer_size()];
        let info = rd.next_frame(&mut buf).ok()?;
        let chn = info.color_type.samples();
        let mask: Vec<u8> = buf[..cw * ch * chn].chunks(chn).map(|p| p[0]).collect();
        let mut rows = vec![false; ch];
        let mut screens = Vec::new();
        for (i, s) in meta["screens"].as_array()?.iter().enumerate() {
            let id = (i + 1) as u8;
            let q = s["quad"].as_array()?;
            let pt = |k: usize| (q[k][0].as_f64().unwrap() as f32, q[k][1].as_f64().unwrap() as f32);
            let (mut x0, mut y0, mut x1, mut y1) = (cw, ch, 0, 0);
            for y in 0..ch {
                for x in 0..cw {
                    if mask[y * cw + x] == id {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x + 1);
                        y1 = y1.max(y + 1);
                        rows[y] = true;
                    }
                }
            }
            if x1 == 0 {
                continue;
            }
            let (w, h) = ((x1 - x0) * scale, (y1 - y0) * scale);
            screens.push(Screen {
                name: s["name"].as_str()?.to_string(),
                id,
                quad: [pt(0), pt(1), pt(2)],
                x0: x0 * scale,
                y0: y0 * scale,
                w,
                h,
                layer: Pixmap::new(w as u32, h as u32)?,
                content: Pixmap::new(w as u32, h as u32)?,
                ckey: None,
                fkey: None,
            });
        }
        Some(Screens { screens, mask, cw, map, scale, rows, redrawn: 0, checked: 0 })
    }

    /// Перевод «пиксель исходника → пиксель слоя экрана» для экрана sc.
    fn src_to_layer(&self, sc: &Screen) -> Transform {
        let s = self.scale as f32;
        let [ax, bx, ay, by] = self.map;
        Transform::from_row(ax * s, 0.0, 0.0, ay * s, bx * s - sc.x0 as f32, by * s - sc.y0 as f32)
    }

    /// Нарисовать экраны в их слои (время t, состояние st). Слой перерисовывается, только если изменилось
    /// то, что на нём видно (ключ). Возвращает true, если хоть один слой изменился.
    pub fn draw(&mut self, t: f64, st: &ScreenState) -> bool {
        let mut face = None;
        let mut any = false;
        for i in 0..self.screens.len() {
            self.checked += 1;
            let tr0 = self.src_to_layer(&self.screens[i]);
            let sc = &mut self.screens[i];
            let key = if sc.name == "clock" { clock_key(t) } else { screen_key(sc, st, t).1 };
            if sc.fkey == Some(key) {
                continue;
            }
            sc.fkey = Some(key);
            self.redrawn += 1;
            any = true;
            let face = face.get_or_insert_with(|| Face::parse(FONT).expect("шрифт"));
            if sc.name == "clock" {
                sc.layer.fill(Color::TRANSPARENT);
                clock(&mut sc.layer, tr0, face, t);
                continue;
            }
            draw_screen(sc, tr0, face, st, t);
        }
        any
    }

    /// Наложить экраны на кадр окна (0x00RRGGBB, ширина строки bw): только по клеткам стекла.
    /// (ox, oy) — где в окне левый верхний угол картинки.
    pub fn overlay(&self, buf: &mut [u32], bw: usize, ox: usize, oy: usize) {
        let s = self.scale;
        for sc in &self.screens {
            let data = sc.layer.data(); // RGBA, предумноженный
            for ly in 0..sc.h {
                let wy = sc.y0 + ly;
                let cy = wy / s;
                for lx in 0..sc.w {
                    let wx = sc.x0 + lx;
                    if self.mask[cy * self.cw + wx / s] != sc.id {
                        continue;
                    }
                    let k = (ly * sc.w + lx) * 4;
                    let a = data[k + 3] as u32;
                    if a == 0 {
                        continue;
                    }
                    let Some(d) = buf.get_mut((oy + wy) * bw + ox + wx) else { continue };
                    let (dr, dg, db) = ((*d >> 16) & 255, (*d >> 8) & 255, *d & 255);
                    let inv = 255 - a;
                    let r = data[k] as u32 + (dr * inv + 127) / 255;
                    let g = data[k + 1] as u32 + (dg * inv + 127) / 255;
                    let b = data[k + 2] as u32 + (db * inv + 127) / 255;
                    *d = (r.min(255) << 16) | (g.min(255) << 8) | b.min(255);
                }
            }
        }
    }
}

fn hash_of<T: std::hash::Hash>(v: T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// Мигание точки «плохо» (3 Гц) и двоеточия часов (2 Гц): фаза как число.
/// «Анимация картинки» выключена: экраны показывают только данные (текст, значки, часы), а помехи кинескопа, мигание,
/// бегущая полоса и печать по буквам стоят в одном спокойном виде. Иначе при редкой перерисовке (раз в секунду) они
/// выглядели как зависшая анимация: полоса и сбои прыгали рывками.
static STILL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_still(on: bool) {
    STILL.store(on, std::sync::atomic::Ordering::Relaxed);
}
fn still() -> bool {
    STILL.load(std::sync::atomic::Ordering::Relaxed)
}
/// Время для помех: при выключенной анимации стоит на месте.
fn fx_t(t: f64) -> f64 {
    if still() {
        0.0
    } else {
        t
    }
}

fn blink3(t: f64) -> bool {
    !still() && (t * 3.0) as i64 % 2 == 1
}
fn colon_on(t: f64) -> bool {
    still() || (t * 2.0) as i64 % 2 == 0
}
/// Плавная пульсация «CLAUDE ON» и качание стрелки на пинге: ступени, неразличимые глазом (≤ 1–2 уровня цвета).
fn pulse(t: f64, rate: f64, steps: f32) -> f32 {
    let t = fx_t(t);
    let k = 0.5 + 0.5 * (t * rate).sin() as f32;
    (k * steps).round() / steps
}
/// Мерцание сканлайнов: прозрачность с шагом 1/64 (разница в цвете меньше уровня).
fn flicker(t: f64) -> f32 {
    let t = fx_t(t);
    (((0.1 + 0.04 * (t * 9.0).sin() + 0.03 * (t * 23.7).sin()) * 64.0).round() / 64.0) as f32
}
/// Сдвиг полосы по строкам слоя: целые пиксели.
fn band_pos(t: f64, period: f64, phase: f64, h: f64) -> f32 {
    (((t / period + phase) % 1.0) * (h * 1.4) - h * 0.2).round() as f32
}

fn clock_key(t: f64) -> u64 {
    hash_of((hhmm(), colon_on(t)))
}

/// Ключ экрана: (ключ содержимого, ключ всего слоя). Ключ слоя меняется ровно тогда, когда меняются пиксели:
/// содержимое (текст, значки, стрелка) + помехи кинескопа (мерцание, полоса, «снег», сбой).
fn screen_key(sc: &Screen, st: &ScreenState, t: f64) -> (u64, u64) {
    let c = crt(&sc.name);
    let glitch = c.is_some_and(|c| glitch_at(c, t));
    let dx = glitch_dx(sc, t, glitch);
    fn any_bad<'a>(mut rows: impl Iterator<Item = &'a String>) -> bool {
        rows.any(|s| s == "bad")
    }
    let blink = |bad: bool| bad && blink3(t);
    let ck = match sc.name.as_str() {
        "main" => {
            let n = typed(st, t);
            let bad = any_bad([&st.vpn.0, &st.claude.0].into_iter());
            hash_of((
                "main",
                hhmm(),
                n,
                &st.message,
                &st.mood,
                &st.vpn,
                &st.claude,
                st.speed.map(|(a, b, c)| (a.to_bits(), b.to_bits(), c.to_bits())),
                blink(bad),
                dx.to_bits(),
            ))
        }
        "net" => match st.speed_live {
            Some((ph, val)) => {
                let k = if ph == 0 { (pulse(t, 6.0, 512.0) * 512.0) as i32 } else { 0 };
                hash_of(("gauge", &st.net_rows.len(), ph, val.to_bits(), k, dx.to_bits()))
            }
            None => {
                let bad = any_bad(st.net_rows.iter().take(6).map(|r| &r.0));
                hash_of(("net", &st.net_rows, blink(bad), dx.to_bits()))
            }
        },
        "sys" => {
            let bad = any_bad(st.sys_rows.iter().take(4).map(|r| &r.0));
            hash_of(("sys", &st.sys_rows, blink(bad), dx.to_bits()))
        }
        _ => {
            let on = st.claude.0 == "ok";
            hash_of(("robot", on, if on { (pulse(t, 3.0, 32.0) * 32.0) as i32 } else { 0 }, dx.to_bits()))
        }
    };
    let [tl, trr, bl] = sc.quad;
    let h = (bl.0 - tl.0).hypot(bl.1 - tl.1) as f64;
    let _ = trr;
    let fk = match c {
        Some((period, phase, _)) => {
            hash_of((ck, flicker(t).to_bits(), band_pos(fx_t(t), period, phase, h).to_bits(), (fx_t(t) * 24.0) as u64, glitch, still()))
        }
        None => hash_of((ck, flicker(t).to_bits())),
    };
    (ck, fk)
}

fn glitch_at(c: (f64, f64, f64), t: f64) -> bool {
    !still() && (t + c.1 * 10.0) % c.2 < 0.18
}
fn glitch_rng(sc: &Screen, t: f64) -> Rng {
    Rng(((fx_t(t) * 24.0) as u64).wrapping_mul(7919) + sc.name.len() as u64 * 131 + 1)
}
fn glitch_dx(sc: &Screen, t: f64, glitch: bool) -> f32 {
    if !glitch {
        return 0.0;
    }
    let mut rnd = glitch_rng(sc, t);
    (if rnd.f() < 0.5 { -1.0 } else { 1.0 }) * rnd.int(3, 7) as f32
}
/// Сколько букв сообщения уже напечатано (не больше длины перевода).
fn typed(st: &ScreenState, t: f64) -> usize {
    let n = if still() { usize::MAX } else { ((t - st.message_t).max(0.0) * 30.0) as usize };
    n.min(crate::i18n::t(&st.message).chars().count())
}

/// Часы шрифтом 3×5 на экране робота-телевизора, наклон 23° вместе с экраном.
fn clock(pm: &mut Pixmap, tr0: Transform, face: &Face, t: f64) {
    let tr = tr0.pre_translate(91., 1004.).pre_rotate(23.0);
    let text = hhmm();
    let u = 3.0;
    let width: f32 = text.chars().map(|c| if c == ':' { 2.0 } else { 4.0 }).sum::<f32>() - 1.0;
    let (mut x, y) = (-width * u / 2.0, -5.0 * u / 2.0);
    let mut px = Vec::new();
    for ch in text.chars() {
        if ch == ':' {
            if colon_on(t) {
                px.push((x, y + u, u, u));
                px.push((x, y + 3. * u, u, u));
            }
            x += 2. * u;
            continue;
        }
        let Some(dg) = ch.to_digit(10) else { continue };
        for (row, bits) in DIGITS[dg as usize].iter().enumerate() {
            for (col, bit) in bits.chars().enumerate() {
                if bit == '1' {
                    px.push((x + col as f32 * u, y + row as f32 * u, u, u));
                }
            }
        }
        x += 4. * u;
    }
    let mut p = Painter { pm, tr, face, size: 24. };
    p.rects(&px, (0.86, 0.96, 1.0), 1.0);
}

/// Содержимое надо перерисовать (ключ содержимого изменился)?
fn tiny_ckey(sc: &Screen, st: &ScreenState, t: f64) -> bool {
    sc.ckey != Some(screen_key(sc, st, t).0)
}

fn draw_screen(sc: &mut Screen, tr0: Transform, face: &Face, st: &ScreenState, t: f64) {
    let [tl, trr, bl] = sc.quad;
    let (ux, uy, vx, vy) = (trr.0 - tl.0, trr.1 - tl.1, bl.0 - tl.0, bl.1 - tl.1);
    let (w, h) = (ux.hypot(uy), vx.hypot(vy));
    // локальные координаты экрана (поворот/наклон), как _local в screens.py
    let base = tr0.pre_concat(Transform::from_row(ux / w, uy / w, vx / h, vy / h, tl.0, tl.1));
    let c = crt(&sc.name);
    let glitch = c.is_some_and(|c| glitch_at(c, t));
    let mut rnd = glitch_rng(sc, t);
    let dx = glitch_dx(sc, t, glitch);
    if glitch {
        rnd.f(); // как при выборе сдвига: «снег» не должен смениться
        rnd.int(3, 7);
    }
    if tiny_ckey(sc, st, t) {
        let ck = screen_key(sc, st, t).0;
        sc.content.fill(Color::TRANSPARENT);
        let mut p = Painter { pm: &mut sc.content, tr: base.pre_translate(dx, 0.0), face, size: 24. };
        content(&mut p, &sc.name, st, w, h, t);
        sc.ckey = Some(ck);
    }
    sc.layer.data_mut().copy_from_slice(sc.content.data());
    let mut p = Painter { pm: &mut sc.layer, tr: base, face, size: 24. };
    // сканлайны и мерцание кинескопа
    let flick = flicker(t);
    let lines: Vec<_> = (0..h as i32).step_by(4).map(|y| (0.0, y as f32, w, 1.0)).collect();
    p.rects(&lines, (0.0, 0.1, 0.2), flick);
    let Some((period, phase, _)) = c else { return };
    // катящаяся светлая полоса (без анимации её нет: застывшая посреди экрана полоса выглядела бы поломкой)
    let k = if still() { -h * 10.0 } else { band_pos(t, period, phase, h as f64) };
    let band = h * 0.2;
    let stops = vec![
        GradientStop::new(0.0, Color::from_rgba(1., 1., 1., 0.).unwrap()),
        GradientStop::new(0.35, Color::from_rgba(1., 1., 1., 0.28).unwrap()),
        GradientStop::new(0.5, Color::from_rgba(1., 1., 1., 0.5).unwrap()),
        GradientStop::new(0.65, Color::from_rgba(1., 1., 1., 0.28).unwrap()),
        GradientStop::new(1.0, Color::from_rgba(1., 1., 1., 0.).unwrap()),
    ];
    if let (Some(sh), Some(r)) = (
        LinearGradient::new(Point::from_xy(0., k - band), Point::from_xy(0., k + band), stops, SpreadMode::Pad, Transform::identity()),
        Rect::from_xywh(0., k - band, w, band * 2.),
    ) {
        let mut paint = Paint::default();
        paint.shader = sh;
        p.pm.fill_rect(r, &paint, p.tr, None);
    }
    // «снег»
    let u = 3.0;
    let n = (w * h / if glitch { 120. } else { 380. }) as usize;
    let (gw, gh) = ((w / u) as i64, (h / u) as i64);
    let mut buckets: [Vec<(f32, f32, f32, f32)>; 4] = Default::default();
    for _ in 0..n {
        let x = rnd.int(0, gw - 1) as f32 * u;
        let y = rnd.int(0, gh - 1) as f32 * u;
        let kind = rnd.int(0, 9);
        let b = if kind < 3 {
            0
        } else if kind < 6 {
            1
        } else if kind < 8 {
            2
        } else {
            3
        };
        buckets[b].push((x, y, u, u));
    }
    let shades = [((1., 1., 1.), 0.38), ((1., 1., 1.), 0.55), ((0., 0.08, 0.15), 0.14), ((0., 0.08, 0.15), 0.24)];
    for (b, (col, a)) in buckets.iter().zip(shades) {
        p.rects(b, col, a);
    }
    if glitch {
        for _ in 0..rnd.int(2, 4) {
            let y = rnd.int(0, h as i64 - 1) as f32;
            let a = (0.25 + 0.25 * rnd.f()) as f32;
            p.rect(0., y, w, if rnd.f() < 0.5 { 2. } else { 3. }, (1., 1., 1.), a);
        }
        let y = rnd.int(0, h as i64 - 1) as f32;
        let th = rnd.int(6, 14) as f32;
        p.rect(0., y, w, th, (0., 0.05, 0.1), 0.18);
    }
}

/// Содержимое экрана: текст, значки, строки.
/// Спидометр на экране «Сеть» во время замера (как в Python-версии, screens.py _gauge): полукруг из точек
/// красный → жёлтый → зелёный, стрелка (на пинге качается, на скорости — по логарифму до 1000 Мбит/с) и подпись.
#[allow(clippy::too_many_arguments)]
fn gauge(p: &mut Painter, cx: f32, cy: f32, r: f32, phase: u8, val: f64, t: f64) {
    use std::f32::consts::PI;
    for i in 0..=30 {
        let k = i as f32 / 30.0;
        let ang = PI + PI * k;
        let st = if k > 0.55 {
            "ok"
        } else if k > 0.25 {
            "warn"
        } else {
            "bad"
        };
        p.rect(cx + r * ang.cos() - 3., cy + r * ang.sin() - 3., 6., 6., st_color(st), 1.0);
    }
    let k = if phase == 0 { pulse(t, 6.0, 512.0) } else { ((val.max(1.0)).log10() / 3.0).min(1.0) as f32 };
    let ang = PI + PI * k;
    p.line(cx, cy, cx + (r - 10.) * ang.cos(), cy + (r - 10.) * ang.sin(), 4., INK);
    let tr = crate::i18n::t;
    let (name, unit) = match phase {
        0 => (tr("пинг"), tr("мс")),
        1 => (tr("↓ скачка"), tr("Мбит/с")),
        _ => (tr("↑ отдача"), tr("Мбит/с")),
    };
    let label = if val > 0.0 { format!("{name} {val:.0} {unit}") } else { name };
    p.text(cx - r, cy + 30., &label, INK);
}

fn content(p: &mut Painter, name: &str, st: &ScreenState, w: f32, h: f32, t: f64) {
    // перевод (i18n.rs) — до обрезки по буквам: обрезанную фразу в таблице не найти
    let tr = crate::i18n::t;
    match name {
        "main" => {
            let m = 12.0;
            p.text(m, 30., &tr("+ АПТЕЧКА"), INK);
            p.text(w - m - 70., 30., &hhmm(), INK);
            p.rect(m, 38., w - 2. * m, 2., INK, 1.0);
            let n = typed(st, t); // печатается по буквам
            let msg: String = tr(&st.message).chars().take(n).collect();
            let col = if st.mood == "bad" || st.mood == "warn" { st_color(&st.mood) } else { INK };
            for (i, ln) in p.wrap(&msg, w - 2. * m).iter().take(2).enumerate() {
                p.text(m, 68. + i as f32 * 26., ln, col);
            }
            let mut y = 132.;
            for (label, (s, val)) in [("VPN", &st.vpn), ("Клод", &st.claude)] {
                p.dot(m + 6., y - 8., s, t);
                // значение — перевод, потом обрезка под ширину экрана
                let val: String = tr(val).chars().take(22).collect();
                p.text(m + 20., y, &format!("{}: {val}", tr(label)), INK);
                y += 26.;
            }
            let f = |v: Option<f64>| v.map(|v| format!("{v:.0}")).unwrap_or("—".into());
            let (d, u, pg) = match st.speed {
                Some((d, u, pg)) => (Some(d), Some(u), Some(pg)),
                None => (None, None, None),
            };
            p.text(m, y, &tr(&format!("↓{} ↑{} Мбит/с  пинг {}", f(d), f(u), f(pg))), INK);
        }
        "net" => {
            let m = 14.0;
            p.text(m, 32., &tr("СЕТЬ"), INK);
            if let Some((phase, val)) = st.speed_live {
                gauge(p, w / 2., h * 0.72, w.min(h) * 0.38, phase, val, t);
                return;
            }
            let mut y = 62.;
            for (s, label, val) in st.net_rows.iter().take(6) {
                p.dot(m + 6., y - 8., s, t);
                let line: String = format!("{} {}", tr(label), tr(val)).chars().take(22).collect();
                p.text(m + 20., y, &line, INK);
                y += 26.;
            }
        }
        "sys" => {
            let m = 10.0;
            let mut y = 30.;
            for (s, label, val) in st.sys_rows.iter().take(4) {
                p.dot(m + 5., y - 8., s, t);
                // строка с x = m + 15 до правого края стекла. Не влезает (например «10% 41°»)
                // — сначала убираем пробел перед температурой, и только потом мельче:
                // иначе шрифт прыгает каждый раз, когда загрузка переходит через 10 %
                let room = w - m - 15.;
                let mut line: String = format!("{} {}", tr(label), tr(val)).chars().take(16).collect();
                p.size = 24.;
                if p.width(&line) > room {
                    if let Some(i) = line.rfind(' ') {
                        if i > label.len() {
                            line.remove(i);
                        }
                    }
                }
                if p.width(&line) > room {
                    p.size = 18.;
                }
                p.text(m + 15., y, &line, INK);
                p.size = 24.;
                y += 27.;
            }
        }
        _ => {
            // робот: CLAUDE ON / OFF
            let on = st.claude.0 == "ok";
            let ink = (0.1, 0.25, 0.05);
            p.text(14., 30., "CLAUDE", ink);
            let col = if on {
                let k = pulse(t, 3.0, 32.0);
                (0.1, 0.5 + 0.3 * k, 0.15)
            } else {
                (0.75, 0.12, 0.15)
            };
            p.rect(16., 44., 14., 14., col, 1.0);
            p.text(38., 60., if on { "ON" } else { "OFF" }, ink);
        }
    }
}

#[cfg(test)]
mod font_tests {
    use super::*;

    #[test]
    fn painter_text_matches_fixture() {
        let face = Face::parse(FONT).unwrap();
        let strs = ["ON OFF 0123456789", "Аптечка — всё ок ✓", "36.6° ↓↑ …", "\u{E000}x"];
        let mut pm = Pixmap::new(420, 40 * strs.len() as u32).unwrap();
        let mut widths = String::new();
        for (i, s) in strs.iter().enumerate() {
            let mut p = Painter { pm: &mut pm, tr: Transform::identity(), face: &face, size: 24. };
            p.text(4., 30. + 40. * i as f32, s, (1.0, 1.0, 1.0));
            widths += &format!("{}\n", p.width(s).to_bits());
        }
        let pix: Vec<u8> = pm.data().chunks(4).map(|p| p[3]).collect();
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fonts");
        if std::env::var("UPDATE_FONT_FIXTURE").is_ok() {
            std::fs::write(format!("{dir}/painter_widths.txt"), &widths).unwrap();
            std::fs::write(format!("{dir}/painter.bin"), &pix).unwrap();
        }
        assert_eq!(widths, std::fs::read_to_string(format!("{dir}/painter_widths.txt")).unwrap());
        let want = std::fs::read(format!("{dir}/painter.bin")).unwrap();
        assert_eq!(pix.len(), want.len());
        let max = pix.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        let n = pix.iter().zip(&want).filter(|(a, b)| a != b).count();
        eprintln!("экраны: макс. разница {max}, отличных пикселей {n}");
        assert!(max <= 1 && n <= 50, "текст экранов изменился: макс {max}, пикселей {n}");
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn load() -> Screens {
        let dec = png::Decoder::new(std::io::Cursor::new(crate::art::read("", "экраны.png").unwrap()));
        let rd = dec.read_info().unwrap();
        let (cw, ch) = (rd.info().width as usize, rd.info().height as usize);
        Screens::load("", cw, ch, 2).expect("экраны")
    }

    fn state() -> ScreenState {
        let rows = |s: &str| vec![(s.to_string(), "Имя".to_string(), "значение".to_string()), ("ok".into(), "Б".into(), "1".into())];
        ScreenState {
            message: "Всё хорошо, мур, проверил".into(),
            message_t: 1.0,
            mood: "warn".into(),
            vpn: ("bad".into(), "выкл".into()),
            claude: ("ok".into(), "работает".into()),
            speed: Some((100.0, 20.0, 15.0)),
            net_rows: rows("bad"),
            sys_rows: rows("warn"),
            speed_live: None,
        }
    }

    fn layers(s: &Screens) -> Vec<Vec<u8>> {
        s.screens.iter().map(|x| x.layer.data().to_vec()).collect()
    }

    fn fresh(s: &Screens) -> Screens {
        let mut f = Screens { screens: Vec::new(), mask: s.mask.clone(), cw: s.cw, map: s.map, scale: s.scale, rows: vec![], redrawn: 0, checked: 0 };
        for x in &s.screens {
            f.screens.push(Screen {
                name: x.name.clone(),
                id: x.id,
                quad: x.quad,
                x0: x.x0,
                y0: x.y0,
                w: x.w,
                h: x.h,
                layer: Pixmap::new(x.w as u32, x.h as u32).unwrap(),
                content: Pixmap::new(x.w as u32, x.h as u32).unwrap(),
                ckey: None,
                fkey: None,
            });
        }
        f
    }

    #[test]
    fn cache_equals_fresh_render() {
        std::env::set_var("CLOCK", "12:34");
        let mut st = state();
        let mut cached = load();
        for i in 0..400 {
            let t = 1.0 + i as f64 / 12.0 + (i % 5) as f64 * 0.013;
            if i == 150 {
                st.speed_live = Some((0, 0.0));
            }
            if i == 200 {
                st.speed_live = Some((1, 250.0));
            }
            if i == 250 {
                st.speed_live = None;
                st.claude.0 = "bad".into();
            }
            cached.draw(t, &st);
            let mut f = fresh(&cached);
            f.draw(t, &st);
            assert!(layers(&cached) == layers(&f), "слои разошлись на t={t}");
        }
        eprintln!("перерисовано {} из {}", cached.redrawn, cached.checked);
        assert!(cached.redrawn < cached.checked);
    }

    #[test]
    fn keys_change_when_visible() {
        std::env::set_var("CLOCK", "12:34");
        let s = load();
        let st = state();
        let find = |n: &str| s.screens.iter().find(|x| x.name == n).unwrap();
        // двоеточие часов: смена каждые 0.5 с
        assert_eq!(clock_key(0.1), clock_key(0.4));
        assert_ne!(clock_key(0.4), clock_key(0.6));
        // печать: новая буква — другой ключ содержимого, без новой буквы — тот же
        let main = find("main");
        let ck = |t: f64| screen_key(main, &st, t).0;
        assert_ne!(ck(1.0 + 0.05), ck(1.0 + 0.1));
        assert_eq!(ck(1.0 + 0.1 + 0.001), ck(1.0 + 0.1 + 0.02));
        // после конца печати ключ постоянен внутри одной фазы мигания
        assert_eq!(ck(10.01), ck(10.0 + 1.0 / 3.0 - 0.01));
        // мигание: край фазы
        assert_ne!(ck(10.01), ck(10.0 + 1.0 / 3.0 + 0.01));
        // без «плохих» точек мигания нет
        let mut ok = state();
        ok.vpn.0 = "ok".into();
        assert_eq!(screen_key(main, &ok, 10.01).0, screen_key(main, &ok, 10.0 + 1.0 / 3.0 + 0.01).0);
        // стрелка: на скорости стоит, пока значение то же; значение сменилось — другой ключ
        let net = find("net");
        let mut live = state();
        live.speed_live = Some((1, 100.0));
        assert_eq!(screen_key(net, &live, 3.0).0, screen_key(net, &live, 3.5).0);
        let a = screen_key(net, &live, 3.0).0;
        live.speed_live = Some((1, 200.0));
        assert_ne!(screen_key(net, &live, 3.0).0, a);
        // на пинге качается
        live.speed_live = Some((0, 0.0));
        assert_ne!(screen_key(net, &live, 3.0).0, screen_key(net, &live, 3.2).0);
        // робот: ON пульсирует, OFF — нет
        let robot = s.screens.iter().find(|x| !["main", "net", "sys", "clock"].contains(&x.name.as_str())).unwrap();
        assert_ne!(screen_key(robot, &st, 0.0).0, screen_key(robot, &st, 0.5).0);
        let mut off = state();
        off.claude.0 = "bad".into();
        assert_eq!(screen_key(robot, &off, 0.0).0, screen_key(robot, &off, 0.5).0);
    }
}
