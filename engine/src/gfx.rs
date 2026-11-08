//! Рисование окна своими руками: прямоугольники, рамки в стиле Windows XP/2000,
//! текст шрифтом DejaVu Sans Condensed (кэш растров глифов по размеру), значки.
//! Всё пишется прямо в буфер окна (0x00RRGGBB), без GTK и OpenGL.

use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::outline::{DrawSettings, OutlineGlyphCollection, OutlinePen};
use skrifa::{charmap::Charmap, FontRef, GlyphId, MetadataProvider};
use std::collections::HashMap;
use tiny_skia::{Color, FillRule, GradientStop, LineCap, Paint, PathBuilder, Pixmap, Point, RadialGradient, SpreadMode, Stroke, Transform};

const FONT_REG: &[u8] = include_bytes!("../assets/DejaVuSansCondensed.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../assets/DejaVuSansCondensed-Bold.ttf");

// классические серые цвета Windows 2000/XP
pub const FACE: u32 = 0xD4D0C8;
pub const WHITE: u32 = 0xFFFFFF;
pub const LIGHT: u32 = 0xDFDFDF;
pub const SHADOW: u32 = 0x808080;
pub const DARK: u32 = 0x404040;
pub const BLACK: u32 = 0x000000;
pub const SELECT: u32 = 0x0A246A;
pub const TIP: u32 = 0xFFFFE1;

// ───────────────────────────── Прямоугольники ─────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> i32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }
    pub fn intersect(&self, o: Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y {
            Rect::default()
        } else {
            Rect::new(x, y, r - x, b - y)
        }
    }
    pub fn union(&self, o: Rect) -> Rect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn inset(&self, d: i32) -> Rect {
        Rect::new(self.x + d, self.y + d, (self.w - 2 * d).max(0), (self.h - 2 * d).max(0))
    }
}

fn mix(dst: u32, src: u32, a: u32) -> u32 {
    let ch = |s: u32, d: u32| (d * (255 - a) + s * a + 127) / 255;
    let r = ch((src >> 16) & 255, (dst >> 16) & 255);
    let g = ch((src >> 8) & 255, (dst >> 8) & 255);
    let b = ch(src & 255, dst & 255);
    (r << 16) | (g << 8) | b
}

// ───────────────────────────── Холст ─────────────────────────────

/// Окно пикселей с обрезкой: всё, что вне `clip`, молча пропускается.
pub struct Canvas<'a> {
    buf: &'a mut [u32],
    stride: usize,
    w: i32,
    h: i32,
    pub clip: Rect,
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u32], stride: usize, w: i32, h: i32) -> Canvas<'a> {
        Canvas { buf, stride, w, h, clip: Rect::new(0, 0, w, h) }
    }

    /// Пустой холст: ничего не рисует (так считают раскладку и области нажатий).
    pub fn none() -> Canvas<'static> {
        Canvas { buf: &mut [], stride: 0, w: 0, h: 0, clip: Rect::default() }
    }

    pub fn set_clip(&mut self, r: Rect) {
        self.clip = r.intersect(Rect::new(0, 0, self.w, self.h));
    }

    fn clipped(&self, r: Rect) -> Rect {
        r.intersect(self.clip)
    }

    pub fn fill(&mut self, r: Rect, col: u32) {
        let r = self.clipped(r);
        for y in r.y..r.bottom() {
            let o = y as usize * self.stride;
            if let Some(row) = self.buf.get_mut(o + r.x as usize..o + r.right() as usize) {
                row.fill(col);
            }
        }
    }

    pub fn put(&mut self, x: i32, y: i32, col: u32) {
        if self.clip.contains(x, y) {
            if let Some(p) = self.buf.get_mut(y as usize * self.stride + x as usize) {
                *p = col;
            }
        }
    }

    pub fn blend(&mut self, x: i32, y: i32, col: u32, a: u32) {
        if a == 0 || !self.clip.contains(x, y) {
            return;
        }
        if let Some(p) = self.buf.get_mut(y as usize * self.stride + x as usize) {
            *p = mix(*p, col, a);
        }
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, col: u32) {
        self.fill(Rect::new(x, y, w, 1), col);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, col: u32) {
        self.fill(Rect::new(x, y, 1, h), col);
    }

    /// Рамка в 1 пиксель: верх и лево — цветом tl, низ и право — цветом br.
    pub fn bevel(&mut self, r: Rect, tl: u32, br: u32) {
        self.hline(r.x, r.y, r.w, tl);
        self.vline(r.x, r.y, r.h, tl);
        self.hline(r.x, r.bottom() - 1, r.w, br);
        self.vline(r.right() - 1, r.y, r.h, br);
    }

    /// Объёмная кнопка/панель: белый верх-лево, тёмный низ-право (двойная рамка).
    pub fn raised(&mut self, r: Rect, face: u32) {
        self.fill(r, face);
        self.bevel(r, WHITE, DARK);
        self.bevel(r.inset(1), LIGHT, SHADOW);
    }

    /// Нажатая кнопка.
    pub fn pressed(&mut self, r: Rect, face: u32) {
        self.fill(r, face);
        self.bevel(r, DARK, WHITE);
        self.bevel(r.inset(1), SHADOW, LIGHT);
    }

    /// Утопленная панель (поле ввода, таблица, строка состояния).
    pub fn sunken(&mut self, r: Rect, face: u32) {
        self.fill(r, face);
        self.bevel(r, SHADOW, WHITE);
        self.bevel(r.inset(1), DARK, FACE);
    }

    /// Плавный переход слева направо по остановкам (доля, цвет).
    pub fn gradient_h(&mut self, r: Rect, stops: &[(f32, u32)]) {
        let rc = self.clipped(r);
        for x in rc.x..rc.right() {
            let t = if r.w > 1 { (x - r.x) as f32 / (r.w - 1) as f32 } else { 0.0 };
            let col = grad_at(stops, t);
            self.fill(Rect::new(x, rc.y, 1, rc.h), col);
        }
    }

    /// Положить предумноженный RGBA-рисунок.
    pub fn blit(&mut self, pm: &Pixmap, x: i32, y: i32) {
        let r = self.clipped(Rect::new(x, y, pm.width() as i32, pm.height() as i32));
        if r.is_empty() {
            return;
        }
        let data = pm.data();
        let pw = pm.width() as usize;
        for yy in r.y..r.bottom() {
            for xx in r.x..r.right() {
                let k = ((yy - y) as usize * pw + (xx - x) as usize) * 4;
                let Some(px) = data.get(k..k + 4) else { continue };
                let a = px[3] as u32;
                if a == 0 {
                    continue;
                }
                let Some(d) = self.buf.get_mut(yy as usize * self.stride + xx as usize) else { continue };
                if a == 255 {
                    *d = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
                } else {
                    let inv = 255 - a;
                    let (dr, dg, db) = ((*d >> 16) & 255, (*d >> 8) & 255, *d & 255);
                    let r_ = px[0] as u32 + (dr * inv + 127) / 255;
                    let g_ = px[1] as u32 + (dg * inv + 127) / 255;
                    let b_ = px[2] as u32 + (db * inv + 127) / 255;
                    *d = (r_.min(255) << 16) | (g_.min(255) << 8) | b_.min(255);
                }
            }
        }
    }

    /// Маленький закрашенный треугольник-стрелка (для полос прокрутки и «Сессии ▾»).
    pub fn arrow(&mut self, cx: i32, cy: i32, size: i32, dir: u8, col: u32) {
        // dir: 0 вверх, 1 вниз
        for i in 0..size {
            let half = if dir == 0 { i } else { size - 1 - i };
            let y = cy - size / 2 + i;
            self.hline(cx - half, y, half * 2 + 1, col);
        }
    }
}

fn grad_at(stops: &[(f32, u32)], t: f32) -> u32 {
    let mut prev = match stops.first() {
        Some(s) => *s,
        None => return 0,
    };
    if t <= prev.0 {
        return prev.1;
    }
    for &s in &stops[1..] {
        if t <= s.0 {
            let k = if s.0 > prev.0 { (t - prev.0) / (s.0 - prev.0) } else { 1.0 };
            return mix(prev.1, s.1, (k * 255.0) as u32);
        }
        prev = s;
    }
    prev.1
}

// ───────────────────────────── Шрифт ─────────────────────────────

struct Glyph {
    w: i32,
    h: i32,
    left: i32,
    top: i32,
    adv: f32,
    cov: Vec<u8>,
}

/// Шрифт TrueType (skrifa): размеры — в единицах шрифта, перевод в пиксели делает вызывающий.
#[derive(Clone)]
pub struct Face {
    charmap: Charmap<'static>,
    outlines: OutlineGlyphCollection<'static>,
    hmetrics: GlyphMetrics<'static>,
    pub upm: f32,
}

/// Габарит глифа в единицах шрифта (из заголовка глифа таблицы glyf).
pub struct GlyphBox {
    pub x_min: f32,
    pub y_min: f32,
    pub x_max: f32,
    pub y_max: f32,
}

impl Face {
    pub fn parse(data: &'static [u8]) -> Option<Face> {
        let font = FontRef::new(data).ok()?;
        let loc = LocationRef::default();
        let upm = font.metrics(Size::unscaled(), loc).units_per_em.max(1) as f32;
        Some(Face { charmap: font.charmap(), outlines: font.outline_glyphs(), hmetrics: font.glyph_metrics(Size::unscaled(), loc), upm })
    }

    pub fn glyph_index(&self, ch: char) -> Option<GlyphId> {
        self.charmap.map(ch)
    }

    pub fn advance(&self, g: GlyphId) -> Option<f32> {
        self.hmetrics.advance_width(g)
    }

    /// None — у глифа нет контура (пробел и т. п.).
    pub fn bounding_box(&self, g: GlyphId) -> Option<GlyphBox> {
        let b = self.hmetrics.bounds(g)?;
        (b.x_max > b.x_min && b.y_max > b.y_min).then_some(GlyphBox { x_min: b.x_min, y_min: b.y_min, x_max: b.x_max, y_max: b.y_max })
    }

    pub fn outline(&self, g: GlyphId, pen: &mut impl OutlinePen) -> Option<()> {
        let o = self.outlines.get(g)?;
        o.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), pen).ok().map(|_| ())
    }
}

struct Outline {
    pb: PathBuilder,
    s: f32,
    ox: f32,
    oy: f32,
}

impl OutlinePen for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(self.ox + x * self.s, self.oy - y * self.s);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pb.line_to(self.ox + x * self.s, self.oy - y * self.s);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.pb.quad_to(self.ox + x1 * self.s, self.oy - y1 * self.s, self.ox + x * self.s, self.oy - y * self.s);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.pb.cubic_to(
            self.ox + x1 * self.s,
            self.oy - y1 * self.s,
            self.ox + x2 * self.s,
            self.oy - y2 * self.s,
            self.ox + x * self.s,
            self.oy - y * self.s,
        );
    }
    fn close(&mut self) {
        self.pb.close();
    }
}

/// Два начертания (обычное и жирное) и кэш растров глифов по (начертание, символ, размер).
pub struct Fonts {
    faces: [Face; 2],
    cache: HashMap<(u8, char, u16), Glyph>,
    /// без сглаживания (пиксельный шрифт)
    crisp: bool,
}

pub(crate) const FONT_PIXEL: &[u8] = include_bytes!("../assets/FusionPixel.ttf");

fn raster(face: &Face, ch: char, px: u16, crisp: bool) -> Glyph {
    let s = px as f32 / face.upm;
    let blank = |adv: f32| Glyph { w: 0, h: 0, left: 0, top: 0, adv, cov: Vec::new() };
    let Some(gid) = face.glyph_index(ch) else { return blank(px as f32 * 0.55) };
    let adv = face.advance(gid).map(|a| a * s).unwrap_or(px as f32 * 0.55);
    let Some(bb) = face.bounding_box(gid) else { return blank(adv) };
    let x0 = (bb.x_min * s).floor() as i32 - 1;
    let top = (bb.y_max * s).ceil() as i32 + 1;
    let x1 = (bb.x_max * s).ceil() as i32 + 1;
    let bot = (bb.y_min * s).floor() as i32 - 1;
    let (w, h) = (x1 - x0, top - bot);
    if w <= 0 || h <= 0 {
        return blank(adv);
    }
    let mut ol = Outline { pb: PathBuilder::new(), s, ox: -x0 as f32, oy: top as f32 };
    if face.outline(gid, &mut ol).is_none() {
        return blank(adv);
    }
    let (Some(path), Some(mut pm)) = (ol.pb.finish(), Pixmap::new(w as u32, h as u32)) else { return blank(adv) };
    let mut paint = Paint::default();
    paint.set_color_rgba8(255, 255, 255, 255);
    paint.anti_alias = !crisp;
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
    let cov = pm.data().chunks(4).map(|p| if crisp && p[3] > 0 { 255 } else { p[3] }).collect();
    Glyph { w, h, left: x0, top, adv, cov }
}

impl Fonts {
    pub fn new() -> Fonts {
        let parse = |d: &'static [u8]| Face::parse(d).expect("шрифт DejaVu вшит в программу");
        Fonts { faces: [parse(FONT_REG), parse(FONT_BOLD)], cache: HashMap::new(), crisp: false }
    }

    /// Пиксельный шрифт (FusionPixel, кириллица есть), чёткий: размер — кратный 12.
    pub fn pixel() -> Fonts {
        let f = || Face::parse(FONT_PIXEL).expect("пиксельный шрифт вшит в программу");
        Fonts { faces: [f(), f()], cache: HashMap::new(), crisp: true }
    }

    fn glyph(&mut self, bold: bool, ch: char, px: u16) -> &Glyph {
        let b = bold as usize;
        let face = &self.faces[b & 1];
        let crisp = self.crisp;
        self.cache.entry((b as u8, ch, px)).or_insert_with(|| {
            let mut g = raster(face, ch, px, crisp);
            if crisp && ch == ' ' {
                g.adv = (px as f32 / 3.0).round(); // у пиксельного шрифта пробел слишком широкий
            }
            g
        })
    }

    // Текст переводится здесь, при показе (i18n.rs): ширина, рисование, обрезка и перенос считают
    // одну и ту же — уже переведённую — строку.
    pub fn width(&mut self, s: &str, px: u16, bold: bool) -> i32 {
        let s = crate::i18n::tr(s);
        let mut w = 0.0;
        for ch in s.chars() {
            w += self.glyph(bold, ch, px).adv;
        }
        w.round() as i32
    }

    /// Базовая линия, при которой строка шрифта px стоит по центру полосы высотой h.
    pub fn center_base(&self, y: i32, h: i32, px: u16) -> i32 {
        y + (h + (px as f32 * 0.73) as i32) / 2
    }

    /// Нарисовать строку; x — левый край, base — базовая линия.
    pub fn draw(&mut self, c: &mut Canvas, x: i32, base: i32, s: &str, col: u32, px: u16, bold: bool) {
        if c.clip.is_empty() || base + px as i32 / 3 < c.clip.y || base - px as i32 > c.clip.bottom() {
            return;
        }
        let s = crate::i18n::tr(s);
        let mut pen = x as f32;
        for ch in s.chars() {
            let g = self.glyph(bold, ch, px);
            let gx = pen.round() as i32 + g.left;
            let gy = base - g.top;
            if g.w > 0 && gx < c.clip.right() && gx + g.w > c.clip.x {
                for yy in 0..g.h {
                    let Some(row) = g.cov.get((yy * g.w) as usize..((yy + 1) * g.w) as usize) else { continue };
                    for (xx, &a) in row.iter().enumerate() {
                        c.blend(gx + xx as i32, gy + yy, col, a as u32);
                    }
                }
            }
            pen += g.adv;
        }
    }

    /// Обрезать строку многоточием, чтобы влезла в maxw пикселей.
    pub fn fit(&mut self, s: &str, maxw: i32, px: u16, bold: bool) -> String {
        let s = &*crate::i18n::tr(s);
        if self.width(s, px, bold) <= maxw {
            return s.to_string();
        }
        let dots = self.width("…", px, bold);
        let mut out = String::new();
        let mut w = 0;
        for ch in s.chars() {
            let cw = self.width(&ch.to_string(), px, bold);
            if w + cw + dots > maxw {
                break;
            }
            w += cw;
            out.push(ch);
        }
        out.push('…');
        out
    }

    /// Перенос по словам в строки шириной до maxw; лишнее после max_lines — многоточием.
    pub fn wrap(&mut self, s: &str, maxw: i32, px: u16, bold: bool, max_lines: usize) -> Vec<String> {
        let s = crate::i18n::tr(s);
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();
        for word in s.split_whitespace() {
            let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
            if self.width(&cand, px, bold) <= maxw {
                cur = cand;
                continue;
            }
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            // слово длиннее строки режем по буквам
            let mut piece = String::new();
            for ch in word.chars() {
                let mut t = piece.clone();
                t.push(ch);
                if self.width(&t, px, bold) > maxw && !piece.is_empty() {
                    lines.push(std::mem::take(&mut piece));
                }
                piece.push(ch);
            }
            cur = piece;
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        if lines.len() > max_lines && max_lines > 0 {
            lines.truncate(max_lines);
            if let Some(last) = lines.last_mut() {
                let l = format!("{last}…");
                *last = self.fit(&l, maxw, px, bold);
                if !last.ends_with('…') {
                    last.push('…');
                }
            }
        }
        lines
    }
}

// ───────────────────────────── Значки ─────────────────────────────

/// Уменьшение усреднением по площади (предумноженный RGBA): чётко и без «лесенки».
pub fn box_scale(src: &Pixmap, size: u32) -> Option<Pixmap> {
    let mut out = Pixmap::new(size, size)?;
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let (rx, ry) = (sw / size as f32, sh / size as f32);
    let sd = src.data();
    let od = out.data_mut();
    for y in 0..size as usize {
        for x in 0..size as usize {
            let (x0, x1) = ((x as f32 * rx) as usize, (((x + 1) as f32 * rx).ceil() as usize).min(sw as usize));
            let (y0, y1) = ((y as f32 * ry) as usize, (((y + 1) as f32 * ry).ceil() as usize).min(sh as usize));
            let mut acc = [0u32; 4];
            let mut n = 0u32;
            for yy in y0..y1.max(y0 + 1) {
                for xx in x0..x1.max(x0 + 1) {
                    let k = (yy * sw as usize + xx) * 4;
                    if let Some(p) = sd.get(k..k + 4) {
                        for c in 0..4 {
                            acc[c] += p[c] as u32;
                        }
                        n += 1;
                    }
                }
            }
            let k = (y * size as usize + x) * 4;
            if let (true, Some(o)) = (n > 0, od.get_mut(k..k + 4)) {
                for (o, a) in o.iter_mut().zip(acc) {
                    *o = a.checked_div(n).unwrap_or(0) as u8;
                }
            }
        }
    }
    Some(out)
}

pub fn decode_png(bytes: &[u8]) -> Option<Pixmap> {
    let mut rd = png::Decoder::new(bytes).read_info().ok()?;
    let mut buf = vec![0; rd.output_buffer_size()];
    let info = rd.next_frame(&mut buf).ok()?;
    let ch = info.color_type.samples();
    let mut pm = Pixmap::new(info.width, info.height)?;
    for (i, p) in pm.data_mut().chunks_mut(4).enumerate() {
        let s = buf.get(i * ch..i * ch + ch)?;
        let a = if ch == 4 { s[3] as u32 } else { 255 };
        let (r, g, b) = if ch >= 3 { (s[0], s[1], s[2]) } else { (s[0], s[0], s[0]) };
        p[0] = (r as u32 * a / 255) as u8;
        p[1] = (g as u32 * a / 255) as u8;
        p[2] = (b as u32 * a / 255) as u8;
        p[3] = a as u8;
    }
    Some(pm)
}

macro_rules! xp_icon {
    ($n:literal) => {
        ($n, include_bytes!(concat!("../../assets/xp/", $n, ".png")) as &'static [u8])
    };
}

const ICON_FILES: &[(&str, &[u8])] = &[
    xp_icon!("ai"),
    xp_icon!("check"),
    xp_icon!("dev"),
    xp_icon!("gpu"),
    xp_icon!("heal"),
    xp_icon!("log"),
    xp_icon!("net"),
    xp_icon!("refresh"),
    xp_icon!("settings"),
    xp_icon!("speed"),
    xp_icon!("sys"),
    xp_icon!("theme"),
];

pub struct Icons {
    src: HashMap<&'static str, Pixmap>,
    cache: HashMap<(&'static str, u32), Pixmap>,
    status: HashMap<(u8, u32), Pixmap>,
}

impl Icons {
    pub fn new() -> Icons {
        let src = ICON_FILES.iter().filter_map(|(n, b)| decode_png(b).map(|p| (*n, p))).collect();
        Icons { src, cache: HashMap::new(), status: HashMap::new() }
    }

    /// Значок XP нужного размера (кэшируется).
    pub fn get(&mut self, name: &'static str, size: u32) -> Option<&Pixmap> {
        if !self.cache.contains_key(&(name, size)) {
            let scaled = box_scale(self.src.get(name)?, size)?;
            self.cache.insert((name, size), scaled);
        }
        self.cache.get(&(name, size))
    }

    /// Круглый значок статуса: ✗ красный, ! жёлтый, ✓ зелёный, i синий (как StatusIcon в Python).
    pub fn status(&mut self, st: &str, size: u32, lamp: bool) -> Option<&Pixmap> {
        let code: u8 = match st {
            "bad" => 0,
            "warn" => 1,
            "ok" => 2,
            "info" => 3,
            "checking" => 4,
            _ => 5,
        } + if lamp { 8 } else { 0 };
        if let std::collections::hash_map::Entry::Vacant(e) = self.status.entry((code, size)) {
            let pm = draw_status(code & 7, lamp, size)?;
            e.insert(pm);
        }
        self.status.get(&(code, size))
    }
}

fn draw_status(code: u8, lamp: bool, size: u32) -> Option<Pixmap> {
    // рисуем в 4-кратном размере и усредняем: ровные края без лесенки
    let big = size * 4;
    let mut pm = Pixmap::new(big, big)?;
    let col: [f32; 3] = match code {
        0 => [0.85, 0.08, 0.10],
        1 => [0.98, 0.78, 0.10],
        2 => [0.12, 0.66, 0.18],
        3 => [0.10, 0.35, 0.85],
        4 => [0.55, 0.50, 0.85],
        _ => [0.60, 0.60, 0.60],
    };
    let c = big as f32 / 2.0;
    let r = c - big as f32 * 0.06;
    let rgb = |v: [f32; 3], k: f32| Color::from_rgba(v[0] * k, v[1] * k, v[2] * k, 1.0).unwrap_or(Color::BLACK);
    let mut paint = Paint::default();
    paint.anti_alias = true;
    paint.set_color(rgb(col, 0.6));
    let mut pb = PathBuilder::new();
    pb.push_circle(c, c, r);
    let circle = pb.finish()?;
    pm.fill_path(&circle, &paint, FillRule::Winding, Transform::identity(), None);
    let light = [(col[0] + 0.45).min(1.0), (col[1] + 0.45).min(1.0), (col[2] + 0.45).min(1.0)];
    if let Some(sh) = RadialGradient::new(
        Point::from_xy(c - r * 0.35, c - r * 0.4),
        Point::from_xy(c, c),
        r,
        vec![GradientStop::new(0.0, rgb(light, 1.0)), GradientStop::new(1.0, rgb(col, 1.0))],
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        paint.shader = sh;
        let mut pb = PathBuilder::new();
        pb.push_circle(c, c, r - big as f32 * 0.04);
        if let Some(inner) = pb.finish() {
            pm.fill_path(&inner, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    if !lamp {
        let k = r * 0.42;
        let mut line = Paint::default();
        line.anti_alias = true;
        let (white, black) = (code != 1, code == 1);
        line.set_color(if black { Color::BLACK } else { Color::WHITE });
        let _ = white;
        let stroke = Stroke { width: (r * 0.28).max(2.0), line_cap: LineCap::Round, ..Stroke::default() };
        let mut pb = PathBuilder::new();
        match code {
            0 => {
                pb.move_to(c - k, c - k);
                pb.line_to(c + k, c + k);
                pb.move_to(c + k, c - k);
                pb.line_to(c - k, c + k);
            }
            2 => {
                pb.move_to(c - k, c);
                pb.line_to(c - k * 0.2, c + k * 0.75);
                pb.line_to(c + k * 1.05, c - k * 0.8);
            }
            1 => {
                pb.move_to(c, c - k * 1.0);
                pb.line_to(c, c + k * 0.25);
                pb.move_to(c, c + k * 0.85);
                pb.line_to(c, c + k * 0.86);
            }
            3 => {
                pb.move_to(c, c - k * 0.95);
                pb.line_to(c, c - k * 0.94);
                pb.move_to(c, c - k * 0.3);
                pb.line_to(c, c + k * 0.95);
            }
            _ => {}
        }
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &line, &stroke, Transform::identity(), None);
        }
    }
    box_scale(&pm, size)
}

#[cfg(test)]
mod font_tests {
    use super::*;

    const STRINGS: &[&str] = &[
        "Аптечка: всё в порядке",
        "First Aid Cat 0123456789",
        "Ёё Щщ Ъъ ЫЬЭЮЯ — … ✓ ↓↑ 36.6°C",
        "Диск C: 87% (занято) — проверка…",
        "\u{E000}\u{1F600}x", // нет в шрифте: пустой шаг
    ];

    fn configs() -> Vec<(bool, bool, u16)> {
        let mut v = Vec::new();
        for px in [12u16, 13, 24, 26] {
            v.push((false, false, px));
            v.push((false, true, px));
        }
        v.push((true, false, 12));
        v.push((true, false, 24));
        v
    }

    /// Рисует все строки столбиком; возвращает (ширины, яркость пикселей).
    fn render() -> (Vec<i32>, Vec<u8>) {
        let mut widths = Vec::new();
        let mut pix = Vec::new();
        for (pixel, bold, px) in configs() {
            let mut f = if pixel { Fonts::pixel() } else { Fonts::new() };
            let (w, lh) = (480usize, px as i32 * 3 / 2);
            let h = lh as usize * STRINGS.len();
            let mut buf = vec![0u32; w * h];
            let mut c = Canvas::new(&mut buf, w, w as i32, h as i32);
            for (i, s) in STRINGS.iter().enumerate() {
                widths.push(f.width(s, px, bold));
                for ch in s.chars() {
                    widths.push(f.width(&ch.to_string(), px, bold));
                }
                f.draw(&mut c, 2, i as i32 * lh + px as i32, s, 0xFFFFFF, px, bold);
            }
            pix.extend(buf.iter().map(|p| (*p & 0xFF) as u8));
        }
        (widths, pix)
    }

    #[test]
    fn text_rendering_matches_fixture() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fonts");
        let t0 = std::time::Instant::now();
        let (widths, pix) = render();
        eprintln!("отрисовка всех строк (холодный кэш, 10 шрифтов): {:?}", t0.elapsed());
        let wtxt = widths.iter().map(|w| w.to_string()).collect::<Vec<_>>().join("\n");
        if std::env::var("UPDATE_FONT_FIXTURE").is_ok() {
            std::fs::write(format!("{dir}/widths.txt"), &wtxt).unwrap();
            std::fs::write(format!("{dir}/text.bin"), &pix).unwrap();
        }
        let want_w = std::fs::read_to_string(format!("{dir}/widths.txt")).unwrap();
        assert_eq!(wtxt, want_w, "ширины глифов изменились");
        let want = std::fs::read(format!("{dir}/text.bin")).unwrap();
        assert_eq!(pix.len(), want.len());
        let diffs: Vec<u8> = pix.iter().zip(&want).map(|(a, b)| a.abs_diff(*b)).collect();
        let max = diffs.iter().copied().max().unwrap_or(0);
        let n = diffs.iter().filter(|d| **d > 0).count();
        eprintln!("текст: макс. разница {max}, отличных пикселей {n}");
        assert!(max <= 1 && n <= 50, "картинка текста изменилась: макс {max}, пикселей {n}");
    }
}
