//! Экраны в картинке: главный монитор, «Сеть», «Система», экран робота (Claude ON/OFF)
//! и часы на роботе-телевизоре. Как в прежней версии (aptechka/screens.py), но поверх
//! слоёной картинки на сетке.
//!
//! Экран рисуется в координатах старого исходника B1 (там заданы его четырёхугольники),
//! а на окно переносится линейным пересчётом «пиксель исходника → клетка → пиксель окна»
//! (art/layers/экраны.json). Видны только клетки «стекла» экрана (art/layers/экраны.png):
//! листья и провода остаются поверх текста. Помехи кинескопа: сканлайны, катящаяся
//! светлая полоса, «снег», редкие сбои.

use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Rect, SpreadMode, Transform};

const FONT: &[u8] = include_bytes!("../assets/FusionPixel.ttf");
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
impl ttf_parser::OutlineBuilder for Outline<'_> {
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
    face: &'a ttf_parser::Face<'a>,
    size: f32,
}

impl Painter<'_> {
    fn paint(c: (f32, f32, f32), a: f32) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(Color::from_rgba(c.0, c.1, c.2, a).unwrap());
        p.anti_alias = false;
        p
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
        let s = self.size / self.face.units_per_em() as f32;
        text.chars().map(|c| self.face.glyph_index(c).and_then(|g| self.face.glyph_hor_advance(g)).unwrap_or(0) as f32 * s).sum()
    }
    fn text(&mut self, x: f32, y: f32, text: &str, c: (f32, f32, f32)) {
        let s = self.size / self.face.units_per_em() as f32;
        let mut pb = PathBuilder::new();
        let mut ox = x;
        for ch in text.chars() {
            let Some(g) = self.face.glyph_index(ch) else { continue };
            self.face.outline_glyph(g, &mut Outline { pb: &mut pb, x: ox, y, s });
            ox += self.face.glyph_hor_advance(g).unwrap_or(0) as f32 * s;
        }
        if let Some(p) = pb.finish() {
            let mut paint = Self::paint(c, 1.0);
            paint.anti_alias = true; // шрифт пиксельный, но экран наклонён — мягкие края ровнее
            self.pm.fill_path(&p, &paint, FillRule::Winding, self.tr, None);
        }
    }
    fn dot(&mut self, x: f32, y: f32, st: &str, t: f64) {
        if st == "bad" && (t * 3.0) as i64 % 2 == 1 {
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
}

pub struct Screens {
    screens: Vec<Screen>,
    mask: Vec<u8>, // номер экрана в каждой клетке (0 — нет)
    cw: usize,     // ширина картинки в клетках
    map: [f32; 4], // клетка = a·пиксель исходника + b (x, затем y)
    scale: usize,
    /// строки клеток, на которых есть экраны (их перерисовываем каждый кадр)
    pub rows: Vec<bool>,
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
            });
        }
        Some(Screens { screens, mask, cw, map, scale, rows })
    }

    /// Перевод «пиксель исходника → пиксель слоя экрана» для экрана sc.
    fn src_to_layer(&self, sc: &Screen) -> Transform {
        let s = self.scale as f32;
        let [ax, bx, ay, by] = self.map;
        Transform::from_row(ax * s, 0.0, 0.0, ay * s, bx * s - sc.x0 as f32, by * s - sc.y0 as f32)
    }

    /// Нарисовать все экраны в их слои (время t, состояние st).
    pub fn draw(&mut self, t: f64, st: &ScreenState) {
        let face = ttf_parser::Face::parse(FONT, 0).expect("шрифт");
        for i in 0..self.screens.len() {
            let tr0 = self.src_to_layer(&self.screens[i]);
            let sc = &mut self.screens[i];
            sc.layer.fill(Color::TRANSPARENT);
            if sc.name == "clock" {
                clock(&mut sc.layer, tr0, &face, t);
                continue;
            }
            draw_screen(sc, tr0, &face, st, t);
        }
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

/// Часы шрифтом 3×5 на экране робота-телевизора, наклон 23° вместе с экраном.
fn clock(pm: &mut Pixmap, tr0: Transform, face: &ttf_parser::Face, t: f64) {
    let tr = tr0.pre_translate(91., 1004.).pre_rotate(23.0);
    let text = hhmm();
    let u = 3.0;
    let width: f32 = text.chars().map(|c| if c == ':' { 2.0 } else { 4.0 }).sum::<f32>() - 1.0;
    let (mut x, y) = (-width * u / 2.0, -5.0 * u / 2.0);
    let mut px = Vec::new();
    for ch in text.chars() {
        if ch == ':' {
            if (t * 2.0) as i64 % 2 == 0 {
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

fn draw_screen(sc: &mut Screen, tr0: Transform, face: &ttf_parser::Face, st: &ScreenState, t: f64) {
    let [tl, trr, bl] = sc.quad;
    let (ux, uy, vx, vy) = (trr.0 - tl.0, trr.1 - tl.1, bl.0 - tl.0, bl.1 - tl.1);
    let (w, h) = (ux.hypot(uy), vx.hypot(vy));
    // локальные координаты экрана (поворот/наклон), как _local в screens.py
    let base = tr0.pre_concat(Transform::from_row(ux / w, uy / w, vx / h, vy / h, tl.0, tl.1));
    let c = crt(&sc.name);
    let glitch = c.is_some_and(|c| (t + c.1 * 10.0) % c.2 < 0.18);
    let mut rnd = Rng(((t * 24.0) as u64).wrapping_mul(7919) + sc.name.len() as u64 * 131 + 1);
    let dx = if glitch { (if rnd.f() < 0.5 { -1.0 } else { 1.0 }) * rnd.int(3, 7) as f32 } else { 0.0 };
    let mut p = Painter { pm: &mut sc.layer, tr: base.pre_translate(dx, 0.0), face, size: 24. };
    content(&mut p, &sc.name, st, w, t);
    p.tr = base;
    // сканлайны и мерцание кинескопа
    let flick = (0.1 + 0.04 * (t * 9.0).sin() + 0.03 * (t * 23.7).sin()) as f32;
    let lines: Vec<_> = (0..h as i32).step_by(4).map(|y| (0.0, y as f32, w, 1.0)).collect();
    p.rects(&lines, (0.0, 0.1, 0.2), flick);
    let Some((period, phase, _)) = c else { return };
    // катящаяся светлая полоса
    let k = (((t / period + phase) % 1.0) * (h as f64 * 1.4) - h as f64 * 0.2) as f32;
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
fn content(p: &mut Painter, name: &str, st: &ScreenState, w: f32, t: f64) {
    // перевод (i18n.rs) — до обрезки по буквам: обрезанную фразу в таблице не найти
    let tr = crate::i18n::t;
    match name {
        "main" => {
            let m = 12.0;
            p.text(m, 30., &tr("+ АПТЕЧКА"), INK);
            p.text(w - m - 70., 30., &hhmm(), INK);
            p.rect(m, 38., w - 2. * m, 2., INK, 1.0);
            let n = ((t - st.message_t).max(0.0) * 30.0) as usize; // печатается по буквам
            let msg: String = tr(&st.message).chars().take(n).collect();
            let col = if st.mood == "bad" || st.mood == "warn" { st_color(&st.mood) } else { INK };
            for (i, ln) in p.wrap(&msg, w - 2. * m).iter().take(2).enumerate() {
                p.text(m, 68. + i as f32 * 26., ln, col);
            }
            let mut y = 132.;
            for (label, (s, val)) in [("VPN", &st.vpn), ("Клод", &st.claude)] {
                p.dot(m + 6., y - 8., s, t);
                p.text(m + 20., y, &format!("{}: {}", tr(label), tr(val)), INK);
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
                let k = 0.5 + 0.5 * (t * 3.0).sin() as f32;
                (0.1, 0.5 + 0.3 * k, 0.15)
            } else {
                (0.75, 0.12, 0.15)
            };
            p.rect(16., 44., 14., 14., col, 1.0);
            p.text(38., 60., if on { "ON" } else { "OFF" }, ink);
        }
    }
}
