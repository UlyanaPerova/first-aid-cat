//! Виджеты целиком в пикселях: всё рисуется на маленьком холсте «пиксель в пиксель» (рамки, экраны,
//! кнопки, ушки, значки, пиксельный шрифт FusionPixel в родном размере 12) и увеличивается целым
//! множителем без сглаживания — одна чёткость у всего, края ровные, без ореолов. Котов даёт пиксельный
//! спрайт (art/layers/виджет/спрайты, приведены к настоящей сетке; у «Стекла» — кот с кружкой).
//! Виды повторяют концепты ChatGPT: «Окно» (XP), «Монитор» (ЭЛТ, кот выглядывает сверху), «Тамагочи»
//! (корпус с ушками), «Стекло» (светлая панель).

use crate::gfx::{Canvas, Fonts, Rect};
use crate::widget_look::{Hits, Size, Style};
use crate::widget_tpl::TplData;

/// Спрайт: ARGB, старший байт 0 — прозрачно.
pub struct Sprite {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u32>,
}

impl Sprite {
    /// Из вшитых картинок (art/layers/…); None — нет файла.
    pub fn load(rel: &str) -> Option<Sprite> {
        let art = std::env::var("ART").unwrap_or_default();
        crate::art::read(&art, rel)?;
        let (w, h, px) = crate::scene::load_png(&art, rel);
        Some(Sprite { w: w as i32, h: h as i32, px })
    }
}

/// Холст в «пикселях картинки»: цвет и маска непрозрачности (форма виджета).
#[derive(Clone)]
pub struct Px {
    pub w: i32,
    pub h: i32,
    pub(crate) rgb: Vec<u32>,
    pub(crate) on: Vec<bool>,
    /// непрозрачность точки (255 — целиком; меньше — мягкий край «Стекла»)
    pub(crate) a: Vec<u8>,
    /// во сколько раз крупнее рисовать значки и текст (виджет из концепта рисуется сразу в пикселях экрана)
    unit: i32,
}

const O: u32 = 0xFF00_0000;

impl Px {
    /// Пустой холст (для тестов оживления).
    #[cfg(test)]
    pub(crate) fn blank(w: i32, h: i32) -> Px {
        Px::new(w, h)
    }
    /// Те же ли точки (цвет, форма, края).
    #[cfg(test)]
    pub(crate) fn same(&self, o: &Px) -> bool {
        self.w == o.w && self.h == o.h && self.rgb == o.rgb && self.on == o.on && self.a == o.a
    }
    fn new(w: i32, h: i32) -> Px {
        Px { w, h, rgb: vec![0; (w * h) as usize], on: vec![false; (w * h) as usize], a: vec![0; (w * h) as usize], unit: 1 }
    }
    fn put(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && x < self.w && y < self.h {
            let i = (y * self.w + x) as usize;
            self.rgb[i] = c & 0xFF_FFFF;
            self.on[i] = true;
            self.a[i] = 255;
        }
    }
    fn hline(&mut self, x: i32, y: i32, w: i32, c: u32) {
        for i in 0..w.max(0) {
            self.put(x + i, y, c);
        }
    }
    pub(crate) fn fill(&mut self, r: Rect, c: u32) {
        for y in r.y..r.bottom() {
            self.hline(r.x, y, r.w, c);
        }
    }
    /// Отступ строки y у скругления радиусом rad (пиксельная «лесенка» по кругу).
    fn inset(rad: i32, y: i32, h: i32) -> i32 {
        let dy = if y < rad {
            rad - y
        } else if y >= h - rad {
            y - (h - rad) + 1
        } else {
            0
        };
        if dy == 0 {
            return 0;
        }
        let r = rad as f32 - 0.5;
        let d = dy as f32 - 0.5;
        (r - (r * r - d * d).max(0.0).sqrt()).round() as i32
    }
    /// Скруглённый прямоугольник: заливка (вертикальный градиент top→bot).
    fn rrect(&mut self, r: Rect, rad: i32, top: u32, bot: u32) {
        for y in 0..r.h {
            let ins = Self::inset(rad, y, r.h);
            self.hline(r.x + ins, r.y + y, r.w - 2 * ins, mix(top, bot, y, r.h));
        }
    }
    /// Контур скруглённого прямоугольника в 1 пиксель.
    fn rline(&mut self, r: Rect, rad: i32, c: u32) {
        let mut prev = Self::inset(rad, 0, r.h);
        for y in 0..r.h {
            let ins = Self::inset(rad, y, r.h);
            let (a, b) = (r.x + ins, r.right() - 1 - ins);
            if y == 0 || y == r.h - 1 {
                self.hline(a, r.y + y, b - a + 1, c);
            } else {
                // ступенька: дотянуть до прошлой строки, чтобы контур был сплошным
                let span = (prev - ins).max(1);
                self.hline(a, r.y + y, span, c);
                self.hline(b - span + 1, r.y + y, span, c);
            }
            let next = if y + 1 < r.h { Self::inset(rad, y + 1, r.h) } else { ins };
            if next > ins && y + 1 < r.h {
                let span = next - ins;
                self.hline(a, r.y + y, span + 1, c);
                self.hline(b - span, r.y + y, span + 1, c);
            }
            prev = ins;
        }
    }
    /// Скруглённая плашка с контуром.
    fn panel(&mut self, r: Rect, rad: i32, line: u32, top: u32, bot: u32) {
        self.rrect(r, rad, top, bot);
        self.rline(r, rad, line);
    }
    fn disc(&mut self, cx: i32, cy: i32, rad: i32, c: u32) {
        let d = 2 * rad + 1;
        self.rrect(Rect::new(cx - rad, cy - rad, d, d), rad, c, c);
    }
    fn sprite(&mut self, s: &Sprite, x: i32, y: i32) {
        for yy in 0..s.h {
            for xx in 0..s.w {
                let p = s.px[(yy * s.w + xx) as usize];
                if p >> 24 != 0 {
                    self.put(x + xx, y + yy, p);
                }
            }
        }
    }
    /// Значок из строк: символ → цвет (' ' и '.' — пусто).
    fn icon(&mut self, rows: &[&str], x: i32, y: i32, pal: &[(char, u32)]) {
        for (yy, row) in rows.iter().enumerate() {
            for (xx, ch) in row.chars().enumerate() {
                if let Some((_, c)) = pal.iter().find(|(k, _)| *k == ch) {
                    let u = self.unit;
                    self.fill(Rect::new(x + xx as i32 * u, y + yy as i32 * u, u, u), *c);
                }
            }
        }
    }
}

fn mix(a: u32, b: u32, y: i32, h: i32) -> u32 {
    let t = if h <= 1 { 0 } else { (y * 256 / (h - 1)) as u32 };
    let ch = |s: u32| (((a >> s) & 255) * (256 - t) + ((b >> s) & 255) * t) >> 8;
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

// ───────────────────────────── Пиксельные значки ─────────────────────────────

/// Круглый значок статуса 11×11: зелёный с галочкой, жёлтый «!», красный «×», синий «i».
fn status_icon(p: &mut Px, st: &str, x: i32, y: i32) {
    let (fill, dark) = match st {
        "ok" => (0x34B14A, 0x1D7A31),
        "warn" => (0xE6A116, 0x9C6A00),
        "bad" => (0xE0483E, 0x9E2620),
        _ => (0x3D7BD9, 0x22508F),
    };
    const RING: [&str; 11] = [
        "...ddddd...",
        "..dfffffd..",
        ".dfffffffd.",
        "dfffffffffd",
        "dfffffffffd",
        "dfffffffffd",
        "dfffffffffd",
        "dfffffffffd",
        ".dfffffffd.",
        "..dfffffd..",
        "...ddddd...",
    ];
    p.icon(&RING, x, y, &[('d', dark), ('f', fill)]);
    // блик
    let u = p.unit;
    p.fill(Rect::new(x + 3 * u, y + 2 * u, u, u), mix(fill, 0xFFFFFF, 1, 2));
    let glyph: &[&str] = match st {
        "ok" => &["........", "......w.", ".....ww.", ".w..ww..", ".wwww...", "..ww....", "........"],
        "warn" => &["...w....", "...w....", "...w....", "...w....", "........", "...w....", "........"],
        "bad" => &["........", ".w...w..", "..w.w...", "...w....", "..w.w...", ".w...w..", "........"],
        _ => &["...w....", "........", "..ww....", "...w....", "...w....", "..www...", "........"],
    };
    p.icon(glyph, x + 2 * u, y + 2 * u, &[('w', 0xFFFFFF)]);
}

/// Значки-типы 9×9 (как в концептах): сеть, щит, мозг, чип, диск; c — цвет.
fn type_icon(p: &mut Px, kind: usize, x: i32, y: i32, c: u32) {
    let rows: &[&str] = match kind {
        0 => &[".ooooo...", "o.....o..", "..ooo....", ".o...o...", "...o.....", "..ooo....", "...o.....", ".........", "........."], // wifi
        1 => &["..ooooo..", ".ooooooo.", ".oo.o.oo.", ".oo...oo.", ".ooo.ooo.", "..ooooo..", "...ooo...", "....o....", "........."], // щит
        2 => &["..oo.oo..", ".o..o..o.", "o.o.o.o.o", "o..ooo..o", "o.o.o.o.o", ".o..o..o.", "..oo.oo..", ".........", "........."], // мозг
        3 => &[".o.o.o.o.", "ooooooooo", ".o.....o.", "oo.ooo.oo", ".o.ooo.o.", "oo.....oo", "ooooooooo", ".o.o.o.o.", "........."], // чип
        _ => &["ooooooooo", "o.......o", "o.......o", "ooooooooo", "o.....o.o", "ooooooooo", ".........", ".........", "........."], // диск
    };
    p.icon(rows, x, y, &[('o', c)]);
}

/// Квадратный крестик «закрыть» (красная кнопка XP) 11×11.
fn xp_close(p: &mut Px, x: i32, y: i32) {
    let u = p.unit;
    p.panel(Rect::new(x, y, 11 * u, 11 * u), 2 * u, 0xFFFFFF, 0xF07A5C, 0xC8391C);
    p.icon(&["w...w", ".w.w.", "..w..", ".w.w.", "w...w"], x + 3 * u, y + 3 * u, &[('w', 0xFFFFFF)]);
}

/// Белый крест-«аптечка» 7×7.
fn cross(p: &mut Px, x: i32, y: i32, c: u32) {
    p.icon(&["..ccc..", "..ccc..", "ccccccc", "ccccccc", "ccccccc", "..ccc..", "..ccc.."], x, y, &[('c', c)]);
}

// ───────────────────────────── Текст ─────────────────────────────

/// Как писать пиксельным шрифтом: толщина (сколько раз повторить на точку правее), интервал между буквами,
/// добавка к пробелу, длина тире «—» (None — как в шрифте; иначе короткое «–», растянутое на столько точек).
struct Ts {
    weight: i32,
    ls: i32,
    sp: i32,
    dash: Option<i32>,
}

impl Ts {
    fn adv(&self, f: &mut Fonts, ch: char, px: u16) -> i32 {
        let mut b = [0u8; 4];
        let w = match (ch, self.dash) {
            ('—', Some(d)) => f.width("–", px, false) + d,
            _ => f.width(ch.encode_utf8(&mut b), px, false),
        };
        w + self.ls + if ch == ' ' { self.sp } else { 0 }
    }
    // перевод (i18n.rs) — здесь, до разбиения на буквы: по одной букве фразу не узнать
    fn width(&self, f: &mut Fonts, s: &str, px: u16) -> i32 {
        let s = crate::i18n::tr(s);
        s.chars().map(|ch| self.adv(f, ch, px)).sum::<i32>() - if s.is_empty() { 0 } else { self.ls } + self.weight
    }
    fn fit(&self, f: &mut Fonts, s: &str, w: i32, px: u16) -> String {
        let s = &*crate::i18n::tr(s);
        if self.width(f, s, px) <= w {
            return s.to_string();
        }
        let mut t: Vec<char> = s.chars().collect();
        while !t.is_empty() {
            t.pop();
            let c: String = t.iter().collect::<String>().trim_end().to_string() + "…";
            if self.width(f, &c, px) <= w {
                return c;
            }
        }
        String::new()
    }
    #[allow(clippy::too_many_arguments)]
    fn draw(&self, cv: &mut Canvas, f: &mut Fonts, x: i32, base: i32, s: &str, c: u32, px: u16) {
        let s = crate::i18n::tr(s);
        let mut cx = x;
        let mut b = [0u8; 4];
        for ch in s.chars() {
            for k in 0..=self.weight {
                match (ch, self.dash) {
                    ('—', Some(d)) => {
                        f.draw(cv, cx + k, base, "–", c, px, false);
                        f.draw(cv, cx + k + d, base, "–", c, px, false);
                    }
                    _ => {
                        f.draw(cv, cx + k, base, ch.encode_utf8(&mut b), c, px, false);
                    }
                }
            }
            cx += self.adv(f, ch, px);
        }
    }
}

/// Пиксельный текст: FusionPixel 12 без сглаживания (рисуем через Canvas в цвет, маску не трогаем).
fn text(p: &mut Px, f: &mut Fonts, x: i32, y: i32, h: i32, s: &str, c: u32, bold: bool) -> i32 {
    let u = p.unit;
    let px = (12 * u) as u16; // пиксельный шрифт чёткий на кратных 12
    let base = f.center_base(y, h, px);
    let (w, hh) = (p.w, p.h);
    let mut cv = Canvas::new(&mut p.rgb, w as usize, w, hh);
    f.draw(&mut cv, x, base, s, c, px, false);
    if bold {
        f.draw(&mut cv, x + (u / 2).max(1), base, s, c, px, false);
    }
    f.width(s, px, false) + bold as i32 * (u / 2).max(1)
}

fn fit(f: &mut Fonts, s: &str, w: i32, bold: bool) -> String {
    f.fit(s, w - bold as i32, 12, false)
}

// ───────────────────────────── Виды ─────────────────────────────

pub struct Pieces<'a> {
    /// кот или сцена для вида (ok / warn / bad уже выбран)
    pub cat: Option<&'a Sprite>,
    pub mug: &'a Sprite,
}

/// Нарисовать вид на пиксельном холсте; Hits — в пикселях холста.
pub fn draw(style: Style, size: Size, d: &TplData, pc: &Pieces, f: &mut Fonts, show_close: bool) -> (Px, Hits) {
    match style {
        Style::Window => window(size, d, pc, f, None),
        Style::Monitor => monitor(size, d, pc, f, show_close),
        Style::Glass => glass(size, d, pc, f, show_close),
    }
}

/// Строк в среднем виджете: Сеть, VPN, ИИ, ЦП, Диск.
const ROWS: usize = 5;

/// Заглушка, если спрайта нет: тёмный прямоугольник нужного размера.
fn cat_or_blank(p: &mut Px, cat: Option<&Sprite>, r: Rect, bg: u32) {
    match cat {
        Some(s) => p.sprite(s, r.x + (r.w - s.w) / 2, r.bottom() - s.h),
        None => p.fill(r, bg),
    }
}

fn cat_size(cat: Option<&Sprite>, w: i32, h: i32) -> (i32, i32) {
    cat.map_or((w, h), |s| (s.w, s.h))
}

// ── 1. Окно (Windows XP) ──

fn window(size: Size, d: &TplData, pc: &Pieces, f: &mut Fonts, fill: Option<(i32, i32)>) -> (Px, Hits) {
    let (navy, beige, beige2, line) = (0x0F2A4F, 0xEFE6D2, 0xF8F2E4, 0xBFB49A);
    let (cw, ch) = cat_size(pc.cat, 72, 58);
    // в виджете Мака холст плотнее: поля меньше
    let tight = fill.is_some();
    let (pic_y, gap, pill_h, bot) = if tight { (19, 2, 16, 1) } else { (21, 4, 18, 5) };
    let natural = match size {
        Size::Small => ((cw + 12).max(if tight { 0 } else { 120 }), pic_y + ch + 1 + gap + pill_h + bot),
        Size::Medium => (6 + cw + 2 + 5 + 116 + 6, pic_y + (ch + 1).max(ROWS as i32 * 17 - 2) + bot),
    };
    let (w, h) = match fill {
        Some((fw, fh)) => (fw.max(natural.0), fh.max(natural.1)),
        None => natural,
    };
    // область картинки: по размеру кота, а если холст больше — растянута (фон — цвет фона самой картинки)
    let pic_h = match size {
        Size::Small => h - pic_y - 1 - gap - pill_h - bot,
        Size::Medium => h - pic_y - 1 - bot,
    };
    let pic_w = match (size, fill) {
        (Size::Small, Some(_)) => w - 12,
        _ => cw,
    };
    let row_w = w - (6 + pic_w + 2 + 5) - if tight { 9 } else { 6 };
    let mut p = Px::new(w, h);
    // рамка окна: синяя, скруглённая сверху
    // в виджете Мака углы плашки скругляет система (радиус ≈ 13 пикселей холста): рамка скругляется так же,
    // заголовок начинается ниже и правее, крестика нет (в системном виджете он ничего не закрывает)
    let (rad, ty, tx) = if tight { (13, 3, 10) } else { (4, 2, 0) };
    p.panel(Rect::new(0, 0, w, h), rad, 0x0B3A9E, 0x2E6BE0, 0x2459C6);
    p.panel(Rect::new(2, 17, w - 4, h - 19), if tight { 11 } else { 2 }, 0x1B4FB5, beige, beige);
    // заголовок: градиент, крест, «Аптечка», красная кнопка
    for y in ty..16 {
        let c = mix(0x5B9BFF, 0x1E5BD8, y - ty, 16 - ty);
        let ins = if tight { Px::inset(rad, y, h) } else { 0 };
        p.hline(3 + ins, y, w - 6 - 2 * ins, c);
    }
    let ins0 = if tight { Px::inset(rad, ty, h) } else { 0 };
    p.hline(4 + ins0, ty, w - 8 - 2 * ins0, 0x8FBCFF);
    cross(&mut p, 5 + tx, 5 + ty - 2, 0xFFFFFF);
    text(&mut p, f, 14 + tx, ty - 1, 14, "Аптечка", 0xFFFFFF, true);
    let close = Rect::new(w - 15, 3, 11, 11);
    if !tight {
        xp_close(&mut p, w - 15, 3);
    }
    // картинка в рамке
    let pic = Rect::new(if size == Size::Small { (w - pic_w) / 2 } else { 6 }, pic_y, pic_w, pic_h);
    p.fill(Rect::new(pic.x - 1, pic.y - 1, pic.w + 2, pic.h + 2), 0x123A7A);
    let bg = pc.cat.and_then(|s| s.px.get((2 * s.w + 2) as usize).copied()).unwrap_or(0x6F93C9) & 0xFF_FFFF;
    p.fill(pic, bg);
    cat_or_blank(&mut p, pc.cat, pic, bg);

    let list = match size {
        Size::Small => {
            // у Мака нижние углы скруглены системой — плашка уже, чтобы её углы не срезались
            let m = if tight { 7 } else { 5 };
            let b = Rect::new(m, pic.bottom() + 1 + gap, w - 2 * m, pill_h);
            p.panel(b, 3, line, beige2, beige2);
            let (ix, tx_) = if tight { (2, 15) } else { (4, 19) };
            status_icon(&mut p, &d.total.1, b.x + ix, b.y + (b.h - 11) / 2);
            // не влезает «Внимание: OBS» — без «Внимание:»/«Беда:» (о беде и так говорит цвет значка)
            let avail = b.w - tx_ - 2;
            let full = &d.total.0;
            let short = full.split_once(": ").map_or(full.as_str(), |(_, rest)| rest);
            let s = if f.width(full, 12, false) + !tight as i32 > avail { short } else { full.as_str() };
            let t = fit(f, s, avail, !tight);
            let col = if d.total.1 == "bad" { 0xB0261C } else { navy };
            text(&mut p, f, b.x + tx_, b.y, b.h, &t, col, !tight);
            b
        }
        Size::Medium => {
            let x0 = pic.right() + 1 + 5;
            // строки делят высоту поровну (в виджете Мака холст выше — строки выше)
            let rh = ((h - pic_y - bot - 2 * (ROWS as i32 - 1)) / ROWS as i32).max(14);
            for i in 0..ROWS {
                let b = Rect::new(x0, pic_y - 1 + i as i32 * (rh + 2), row_w, rh);
                p.panel(b, 2, line, beige2, beige2);
                let (txt, st) = &d.rows[i];
                if i < 3 {
                    status_icon(&mut p, st, b.x + 3, b.y + (rh - 11) / 2);
                } else {
                    type_icon(&mut p, i, b.x + 4, b.y + (rh - 9) / 2, if st == "bad" { 0xB0261C } else { 0x2F5FB8 });
                }
                let t = fit(f, txt, b.w - 20, true);
                let col = if st == "bad" { 0xB0261C } else { navy };
                text(&mut p, f, b.x + 17, b.y, rh, &t, col, true);
            }
            Rect::new(x0, 20, row_w, h - 24)
        }
    };
    (p, Hits { close, cat: pic, list })
}

// ── 3. Монитор ──

fn monitor(size: Size, d: &TplData, pc: &Pieces, f: &mut Fonts, show_close: bool) -> (Px, Hits) {
    let ink = 0x0B3A66;
    let (cw, ch) = cat_size(pc.cat, 80, 40);
    let mon_w = if size == Size::Small { 112 } else { 210 };
    let scr_h = if size == Size::Small { 56 } else { 96 };
    // кот выглядывает: его нижние 6 пикселей (лапы) лежат на корпусе
    let paws = 6;
    let w = mon_w.max(cw);
    let top = ch - paws;
    let mon = Rect::new((w - mon_w) / 2, top, mon_w, 8 + scr_h + 16);
    let h = mon.bottom();
    let mut p = Px::new(w, h);
    // корпус: тёмный пластик, светлая кромка
    p.panel(mon, 7, 0x15191E, 0x4A535E, 0x2A3038);
    p.hline(mon.x + 6, mon.y + 1, mon.w - 12, 0x6B7682);
    let scr = Rect::new(mon.x + 7, mon.y + 7, mon.w - 14, scr_h);
    p.panel(Rect::new(scr.x - 2, scr.y - 2, scr.w + 4, scr.h + 4), 5, 0x101317, 0x101317, 0x101317);
    p.rrect(scr, 4, 0x9EE0FA, 0x5BBDEB);
    // сканлайны
    for y in (scr.y + 1..scr.bottom() - 1).step_by(2) {
        let ins = Px::inset(4, y - scr.y, scr.h);
        for x in scr.x + ins..scr.right() - ins {
            let i = (y * p.w + x) as usize;
            p.rgb[i] = mix(p.rgb[i], 0x3F9ACB, 1, 6);
        }
    }
    // шапка: «✚ АПТЕЧКА» и часы (в маленьком часы — строкой выше, как в концепте)
    let cw_ = f.width(&d.clock, 12, false);
    let hy = if size == Size::Small { scr.y + 12 } else { scr.y + 2 };
    if size == Size::Small {
        text(&mut p, f, scr.right() - 5 - cw_, scr.y + 1, 11, &d.clock, ink, false);
    } else {
        text(&mut p, f, scr.right() - 5 - cw_, hy, 11, &d.clock, ink, false);
    }
    cross(&mut p, scr.x + 5, hy + 2, ink);
    text(&mut p, f, scr.x + 15, hy, 11, "АПТЕЧКА", ink, true);
    p.hline(scr.x + 4, hy + 12, scr.w - 8, 0x2F7DB0);
    let body_y = hy + 15;

    let list = match size {
        Size::Small => {
            let t = fit(f, &d.total.0, scr.w - 8, false);
            let tw = f.width(&t, 12, false);
            let col = if d.total.1 == "bad" { 0x9A1830 } else { ink };
            text(&mut p, f, scr.x + (scr.w - tw) / 2, body_y, scr.bottom() - body_y - 2, &t, col, false);
            scr
        }
        Size::Medium => {
            for i in 0..ROWS {
                let y = body_y + i as i32 * 15;
                let (txt, st) = &d.rows[i];
                let col = if st == "bad" { 0x9A1830 } else { ink };
                type_icon(&mut p, i, scr.x + 6, y + 3, col);
                let t = fit(f, txt, scr.w - 70, false);
                text(&mut p, f, scr.x + 19, y, 15, &t, col, false);
                // справа — «прибор» строки
                let gx = scr.right() - 34;
                match i {
                    0 => {
                        for b in 0..4 {
                            let bh = 2 + b * 2;
                            let lit = st == "ok" || b == 0;
                            p.fill(Rect::new(gx + 14 + b * 4, y + 11 - bh, 3, bh), if lit { ink } else { 0x7FC4E6 });
                        }
                    }
                    1 => {
                        let on = st == "ok";
                        let r = Rect::new(gx + 14, y + 4, 15, 8);
                        p.rrect(r, 4, if on { 0x1F5FA8 } else { 0x7FA9C6 }, if on { 0x1F5FA8 } else { 0x7FA9C6 });
                        p.disc(if on { r.right() - 5 } else { r.x + 4 }, r.y + 4, 2, 0xFFFFFF);
                    }
                    2 => p.fill(Rect::new(gx + 14, y + 7, 15, 3), if st == "bad" { 0x9A1830 } else { 0x2F7DB0 }),
                    _ => {
                        let v = if i == 3 { d.cpu } else { d.disk_used }.unwrap_or(0.0).clamp(0.0, 100.0);
                        let r = Rect::new(gx, y + 5, 29, 5);
                        p.fill(r, 0x8ACDEB);
                        p.fill(Rect::new(r.x, r.y, ((r.w as f64) * v / 100.0).round() as i32, r.h), ink);
                    }
                }
            }
            scr
        }
    };
    // решётка и лампочки
    let vy = mon.bottom() - 9;
    for i in 0..4 {
        p.fill(Rect::new(mon.x + 8 + i * 4, vy, 2, 4), 0x15191E);
    }
    p.disc(mon.right() - 26, vy + 2, 2, 0x15191E);
    p.disc(mon.right() - 19, vy + 2, 2, 0x15191E);
    p.fill(Rect::new(mon.right() - 13, vy + 1, 5, 2), 0x7CFC70);
    // кот поверх корпуса (лапы на кромке)
    let cat_r = Rect::new((w - cw) / 2, 0, cw, ch);
    if let Some(s) = pc.cat {
        p.sprite(s, cat_r.x, 0);
    }
    let close = Rect::new(mon.right() - 13, mon.y + 2, 9, 9);
    if show_close {
        mini_close(&mut p, close);
    }
    (p, Hits { close, cat: cat_r, list })
}

fn mini_close(p: &mut Px, r: Rect) {
    let u = p.unit;
    p.disc(r.x + 4 * u, r.y + 4 * u, 4 * u, 0x3A4252);
    p.icon(&["w...w", ".w.w.", "..w..", ".w.w.", "w...w"], r.x + 2 * u, r.y + 2 * u, &[('w', 0xFFFFFF)]);
}

// ── 6. Тамагочи ──

// ── 2. Стекло ──

fn glass(size: Size, d: &TplData, pc: &Pieces, f: &mut Fonts, show_close: bool) -> (Px, Hits) {
    let (text_c, sub) = (0x192947, 0x525D73);
    let mug = pc.mug;
    let (w, h) = match size {
        Size::Small => ((mug.w + 14).max(134), mug.h + 4 + 26 + 6),
        Size::Medium => (6 + mug.w + 4 + 118 + 8, mug.h + 10),
    };
    let mut p = Px::new(w, h);
    p.panel(Rect::new(0, 0, w, h), 12, 0xAFC0DC, 0xF1F5FC, 0xD7E2F2);
    p.rline(Rect::new(1, 1, w - 2, h - 2), 11, 0xFFFFFF);
    let pill = |p: &mut Px, r: Rect, rad: i32| {
        p.panel(r, rad, 0xD5DFEE, 0xFDFEFF, 0xF1F5FB);
    };
    let (cat_r, list);
    match size {
        Size::Small => {
            cat_r = Rect::new((w - mug.w) / 2, 3, mug.w, mug.h);
            p.sprite(mug, cat_r.x, cat_r.y);
            let b = Rect::new(5, h - 31, w - 10, 26);
            pill(&mut p, b, 8);
            // значок приложения: синий квадрат с белым крестом
            p.panel(Rect::new(b.x + 4, b.y + 5, 16, 16), 4, 0x2563EB, 0x5B9BFF, 0x2F6FE8);
            cross(&mut p, b.x + 8, b.y + 9, 0xFFFFFF);
            text(&mut p, f, b.x + 24, b.y + 1, 13, "Аптечка", text_c, true);
            let t = fit(f, &d.total.0, b.w - 40, false);
            text(&mut p, f, b.x + 24, b.y + 12, 12, &t, sub, false);
            let dot = match d.total.1.as_str() {
                "bad" => 0xE5534B,
                "warn" => 0xF08A1C,
                "ok" => 0x3CC24F,
                _ => 0x8E9AAB,
            };
            p.disc(b.right() - 9, b.y + 13, 3, dot);
            list = b;
        }
        Size::Medium => {
            cat_r = Rect::new(6, h - 5 - mug.h, mug.w, mug.h);
            p.sprite(mug, cat_r.x, cat_r.y);
            let x0 = cat_r.right() + 4;
            let rh = 16;
            let gap = (h - 10 - rh * ROWS as i32) / (ROWS as i32 - 1);
            for i in 0..ROWS {
                let b = Rect::new(x0, 5 + i as i32 * (rh + gap), w - 8 - x0, rh);
                pill(&mut p, b, 8);
                let (txt, st) = &d.rows[i];
                let c = match st.as_str() {
                    "bad" => 0xE5534B,
                    "warn" => 0xF08A1C,
                    "ok" if i < 3 => 0x3CC24F,
                    _ => 0x8E9AAB,
                };
                p.disc(b.x + 8, b.y + 8, 6, c);
                type_icon(&mut p, i, b.x + 4, b.y + 4, 0xFFFFFF);
                let t = fit(f, txt, b.w - 22, false);
                text(&mut p, f, b.x + 18, b.y + 1, rh - 1, &t, text_c, false);
            }
            list = Rect::new(x0, 0, w - x0, h);
        }
    }
    let close = Rect::new(w - 13, 4, 9, 9);
    if show_close {
        mini_close(&mut p, close);
    }
    (p, Hits { close, cat: cat_r, list })
}

// ───────────────────────────── Увеличение ─────────────────────────────

/// Холст ×s без сглаживания в буфер окна (ARGB с предумноженной альфой: снаружи формы — 0).
pub fn blow_up(p: &Px, s: i32, buf: &mut [u32], bw: i32) {
    for y in 0..p.h {
        for x in 0..p.w {
            let i = (y * p.w + x) as usize;
            // ARGB с предумноженной альфой (мягкий край: цвет × непрозрачность)
            let v = if !p.on[i] {
                0
            } else if p.a[i] == 255 {
                O | p.rgb[i]
            } else {
                let a = p.a[i] as u32;
                let pm = |sh: u32| ((p.rgb[i] >> sh) & 255) * a / 255;
                a << 24 | pm(16) << 16 | pm(8) << 8 | pm(0)
            };
            for yy in 0..s {
                let row = ((y * s + yy) * bw + x * s) as usize;
                if let Some(d) = buf.get_mut(row..row + s as usize) {
                    d.fill(v);
                }
            }
        }
    }
}

/// Масштаб для окна: высота виджета около 240 точек × k (целое увеличение, не меньше 2).
pub fn scale_for(p_h: i32, k: i32) -> i32 {
    ((240 * k) / p_h.max(1)).max(1)
}

/// Прямоугольник холста → окно.
pub fn up(r: Rect, s: i32) -> Rect {
    Rect::new(r.x * s, r.y * s, r.w * s, r.h * s)
}

// ───────────────────────────── Целый виджет из спрайта ─────────────────────────────

/// Виджет целиком — пиксельный спрайт (art/layers/виджет/целиком/<вид>_<размер>_<состояние>.png, концепт ChatGPT,
/// приведённый к сетке, текст стёрт), поверх — живые данные по разметке (<вид>_<размер>.json, в пикселях сетки).
/// Картинка — на сетке вдвое мельче (каждый её пиксель 2×2), текст и значки — в этих мелких пикселях:
/// так же, как в концепте (надписи там мельче «пикселей» кота), и шрифт 12 влезает в поля.
/// Рисуется сразу в пикселях экрана: клетка картинки — `cell`×`cell`, текст и значки — крупностью `u`
/// (шрифт 12·u, чёткий). Цвета приглушены, экраны (монитор, ЖК тамагочи) — ещё и притушены, как настоящие.
/// look: screen_dim — притушить экран (не OLED); lift — сколько добавить светлоты всему (0..256);
/// t — время в секундах (анимация ЭЛТ-экрана, если в описании есть "crt").
pub struct Look<'a> {
    pub screen_dim: bool,
    pub lift: u32,
    pub t: f64,
    /// значки состояния из концепта: «<вид>_<размер>_f<номер поля>@<клетка>» (art/layers/виджет/слои)
    pub pieces: &'a std::collections::HashMap<String, Sprite>,
    pub name: String,
    /// во сколько крупнее пиксельные надписи, чем при экране ×2 (там клетка 3 точки на шрифт ×2; на экране ×1
    /// клетка 2 точки на шрифт ×1 — надпись относительно картинки мельче, растягиваем её на 4/3)
    pub text_scale: f64,
}

/// layer — нижний слой в полной детализации (клетка cell×cell точек), если есть: тогда клетки base только для формы.
/// sans — гладкий шрифт для полей с "font": "sans"/"sans-bold" (у «Стекла»).
#[allow(clippy::too_many_arguments)]
pub fn whole(
    base: &Sprite,
    layer: Option<&Sprite>,
    spec: &serde_json::Value,
    d: &TplData,
    f: &mut Fonts,
    sans: &mut Fonts,
    show_close: bool,
    cell: i32,
    u: i32,
    look: &Look,
) -> (Px, Hits) {
    let screen_dim = look.screen_dim;
    let cs = cell;
    // «пиксель» концепта не квадратный (шаг по высоте больше — до 10%): высота строки клеток — cs·ry точек
    // (строки по cs и cs+1 точек), иначе всё сплюснуто по высоте. Та же формула — tools/widget_layers.py, rowy
    let ry = spec["step"].as_array().and_then(|a| Some(a.get(1)?.as_f64()? / a.first()?.as_f64()?)).filter(|v| *v > 0.5 && *v < 2.0).unwrap_or(1.0);
    let rowy = |j: i32| (j as f64 * cs as f64 * ry + 0.5).floor() as i32;
    let mut p = Px::new(base.w * cs, rowy(base.h));
    p.unit = u;
    // экран (притушить сильнее): все поля данных, у тамагочи — и сцена с котом на ЖК
    let rr = |v: &serde_json::Value| {
        Rect::new(
            v[0].as_i64().unwrap_or(0) as i32,
            v[1].as_i64().unwrap_or(0) as i32,
            v[2].as_i64().unwrap_or(0) as i32,
            v[3].as_i64().unwrap_or(0) as i32,
        )
    };
    let mut screen = Rect::new(0, 0, 0, 0);
    if screen_dim {
        for fl in spec["fields"].as_array().into_iter().flatten() {
            let r = rr(&fl["rect"]);
            screen = if screen.is_empty() { r } else { screen.union(r) };
        }
        if spec["fields"].as_array().is_some_and(|a| a.iter().any(|f| f["kind"] == "row")) && base.h > base.w / 2 && spec["cat"].is_array() {
            // ЖК тамагочи: сцена с котом — тоже экран (монитору кот — над экраном, не трогаем)
        }
        for key in ["lcd", "crt"] {
            if spec.get(key).is_some() {
                screen = screen.union(rr(&spec[key]));
            }
        }
    }
    // ЭЛТ-экран монитора: фон экрана ровный (полосы концепта убраны), чуть темнее к краям, как выпуклое стекло
    let crt = spec.get("crt").map(rr);
    let crt_bg = crt.and_then(|c| screen_bg(base, c));
    // слой не того размера (собран старой разметкой) — не молчать: иначе вид тихо рисуется запасным способом
    if let Some(l) = layer.filter(|l| l.w != base.w * cs || l.h != rowy(base.h)) {
        static WARNED: std::sync::Once = std::sync::Once::new();
        let (lw, lh, ww, wh) = (l.w, l.h, base.w * cs, rowy(base.h));
        WARNED.call_once(|| eprintln!("[виджет] слой {lw}×{lh} не совпал с разметкой {ww}×{wh} — пересобери tools/widget_layers.py и программу"));
    }
    let layer = layer.filter(|l| l.w == base.w * cs && l.h == rowy(base.h));
    if let Some(l) = layer {
        // слой как есть — цвета концепта (без приглушения), края — по клеткам (альфа 0/255)
        for y in 0..l.h {
            for x in 0..l.w {
                let v = l.px[(y * l.w + x) as usize];
                if v >> 24 != 0 {
                    p.put(x, y, v);
                    p.a[(y * p.w + x) as usize] = (v >> 24) as u8;
                }
            }
        }
    }
    for y in 0..base.h {
        for x in 0..base.w {
            if layer.is_some() {
                break;
            }
            let mut v = base.px[(y * base.w + x) as usize];
            if v >> 24 == 0 {
                continue;
            }
            if let (Some(c), Some(bg)) = (crt, crt_bg) {
                if c.contains(x, y) && is_screen_bg(v) {
                    let dx = (x - c.x) as f64 / c.w as f64 * 2.0 - 1.0;
                    let dy = (y - c.y) as f64 / c.h as f64 * 2.0 - 1.0;
                    // экран концепта ровный по яркости (сверка: V ≈ 0,995 по всему экрану) — без затемнения к краям
                    let _ = (dx, dy);
                    // синий экрана — на 1,2% выше (сверка: 251 против 254 в концепте)
                    v = 0xFF00_0000 | (bg & 0xFF_FF00) | (((bg & 255) as f64 * 1.012).round().min(255.0) as u32);
                }
            }
            // клетки с цветами концепта («graded» в разметке, tools/widget_layers.py) не приглушаем
            let v = if spec["graded"].as_bool() == Some(true) { v & 0xFF_FFFF } else { mute(v, screen.contains(x, y)) };
            p.fill(Rect::new(x * cs, rowy(y), cs, rowy(y + 1) - rowy(y)), lift(v, look.lift));
        }
    }
    let r = |v: &serde_json::Value| -> Rect {
        let n = |i: usize| v[i].as_i64().unwrap_or(0) as i32;
        let (y0, y1) = (rowy(n(1)), rowy(n(1) + n(3)));
        Rect::new(n(0) * cs, y0, n(2) * cs, y1 - y0)
    };
    let hex =
        |v: &serde_json::Value, def: u32| -> u32 { v.as_str().and_then(|s| u32::from_str_radix(s.trim_start_matches('#'), 16).ok()).unwrap_or(def) };
    let st_col = |st: &str| match st {
        "bad" => 0xE0483E,
        "warn" => 0xE6A116,
        "ok" => 0x34B14A,
        _ => 0x8E9AAB,
    };
    for (fi, fl) in spec["fields"].as_array().into_iter().flatten().enumerate() {
        let rect = r(&fl["rect"]);
        let idx = fl["index"].as_u64().or(fl["of"].as_u64()).map(|x| (x as usize).min(ROWS - 1));
        let st: String = match (fl["of"].as_str(), idx) {
            (Some("total"), _) | (None, None) => d.total.1.clone(),
            (_, Some(i)) => d.rows[i].1.clone(),
            _ => d.total.1.clone(),
        };
        let col = hex(&fl["color"], 0x1F2937);
        let bad_col = hex(&fl["bad_color"], 0xB0261C);
        let bold = fl["bold"].as_bool().unwrap_or(false);
        let smooth = matches!(fl["font"].as_str(), Some("sans" | "sans-bold"));
        let sans_px = (fl["font_px"].as_f64().unwrap_or(rect.h as f64 / cs as f64 * 0.6) * cs as f64).round().max(6.0) as u16;
        let sans_bold = fl["font"].as_str() == Some("sans-bold");
        // подгонка под концепт (замеры по концепту): сдвиг dx/dy — в клетках; толщина, интервал между буквами,
        // добавка к пробелу и длина тире — в точках экрана ×2 (на экране ×1 — вдвое меньше)
        let at2 = |v: f64| (v * u as f64 / 2.0 * look.text_scale).round() as i32;
        let ts = Ts {
            weight: at2(fl["weight"].as_f64().unwrap_or(if bold { 1.0 } else { 0.0 })),
            ls: at2(fl["ls"].as_f64().unwrap_or(0.0)),
            sp: at2(fl["sp"].as_f64().unwrap_or(0.0)),
            dash: fl["dash"].as_f64().map(at2),
        };
        let ox = (fl["dx"].as_f64().unwrap_or(0.0) * cs as f64).round() as i32;
        let oy = (fl["dy"].as_f64().unwrap_or(0.0) * cs as f64 * ry).round() as i32;
        // место под текст: от сдвинутого начала до конца поля (+2 клетки: в концептах надпись бывает длиннее)
        let avail = rect.w - ox.max(0) + 2 * cs + (fl["over"].as_f64().unwrap_or(0.0) * cs as f64).round() as i32;
        let put_text = |p: &mut Px, f: &mut Fonts, sans: &mut Fonts, s: &str, c: u32| {
            let (w, hh) = (p.w, p.h);
            if smooth {
                let n = s.chars().count() as i32;
                let t = sans.fit(s, avail - ts.ls * (n - 1).max(0), sans_px, sans_bold);
                let tw = sans.width(&t, sans_px, sans_bold) + ts.ls * (t.chars().count() as i32 - 1).max(0);
                let x = match fl["align"].as_str() {
                    Some("center") => rect.x + (rect.w - tw) / 2,
                    Some("right") => rect.right() - tw,
                    _ => rect.x,
                } + ox;
                let base = sans.center_base(rect.y, rect.h, sans_px) + oy;
                let mut cv = Canvas::new(&mut p.rgb, w as usize, w, hh);
                if ts.ls == 0 {
                    sans.draw(&mut cv, x, base, &t, c, sans_px, sans_bold);
                } else {
                    let mut cx = x;
                    let mut b = [0u8; 4];
                    for ch in t.chars() {
                        let g = ch.encode_utf8(&mut b);
                        sans.draw(&mut cv, cx, base, g, c, sans_px, sans_bold);
                        cx += sans.width(g, sans_px, sans_bold) + ts.ls;
                    }
                }
                return;
            }
            let px = (12 * u) as u16;
            let (sx, sy) = (fl["sx"].as_f64().unwrap_or(1.0) * look.text_scale, fl["sy"].as_f64().unwrap_or(1.0) * look.text_scale);
            let t = ts.fit(f, s, (avail as f64 / sx) as i32, px);
            let tw = (ts.width(f, &t, px) as f64 * sx).round() as i32;
            let x = match fl["align"].as_str() {
                Some("center") => rect.x + (rect.w - tw) / 2,
                Some("right") => rect.right() - tw,
                _ => rect.x,
            } + ox;
            let base = f.center_base(rect.y, rect.h, px) + oy;
            if sx == 1.0 && sy == 1.0 {
                let mut cv = Canvas::new(&mut p.rgb, w as usize, w, hh);
                ts.draw(&mut cv, f, x, base, &t, c, px);
                return;
            }
            // растянутая надпись: нарисовать отдельно и перенести, растянув от базовой линии
            let (tw0, th) = (ts.width(f, &t, px) + 2, px as i32 * 2);
            let tb = px as i32 * 3 / 2;
            let mut tmp = vec![0u32; (tw0 * th) as usize];
            {
                let mut cv = Canvas::new(&mut tmp, tw0 as usize, tw0, th);
                ts.draw(&mut cv, f, 0, tb, &t, 0xFF_FFFF, px);
            }
            let (dw, top, bot) =
                ((tw0 as f64 * sx).ceil() as i32, base - (tb as f64 * sy).ceil() as i32, base + ((th - tb) as f64 * sy).ceil() as i32);
            for dy in top..bot {
                let sy_ = (tb as f64 - (base - dy) as f64 / sy).floor() as i32;
                if sy_ < 0 || sy_ >= th {
                    continue;
                }
                for dx in 0..dw {
                    let sx_ = (dx as f64 / sx).floor() as i32;
                    if sx_ < tw0 && tmp[(sy_ * tw0 + sx_) as usize] & 0xFF_FFFF != 0 {
                        let i = (dy * w + x + dx) as usize;
                        if dy >= 0 && dy < hh && x + dx >= 0 && x + dx < w {
                            p.rgb[i] = c & 0xFF_FFFF;
                        }
                    }
                }
            }
        };
        let too_wide = |f: &mut Fonts, sans: &mut Fonts, s: &str| {
            let n = s.chars().count() as i32;
            let sx = fl["sx"].as_f64().unwrap_or(1.0) * look.text_scale;
            if smooth {
                sans.width(s, sans_px, sans_bold) + ts.ls * (n - 1).max(0) > avail
            } else {
                (ts.width(f, s, (12 * u) as u16) as f64 * sx) as i32 > avail
            }
        };
        match fl["kind"].as_str().unwrap_or("") {
            "label" => put_text(&mut p, f, sans, fl["text"].as_str().unwrap_or(""), col),
            // название, нарисованное в самой картинке («Аптечка»): на другом языке закрыть его полосой
            // того же фона (столбец src, где надписи нет — у заголовка фон меняется только по высоте) и написать перевод
            "retitle" if crate::i18n::lang() != crate::i18n::Lang::Ru => {
                let sx = ((fl["src"].as_i64().unwrap_or(0) as i32) * cs + cs / 2).clamp(0, p.w - 1);
                for y in rect.y.max(0)..rect.bottom().min(p.h) {
                    let c = p.rgb[(y * p.w + sx) as usize];
                    for x in rect.x.max(0)..rect.right().min(p.w) {
                        p.rgb[(y * p.w + x) as usize] = c;
                    }
                }
                put_text(&mut p, f, sans, fl["text"].as_str().unwrap_or(""), col)
            }
            "clock" => put_text(&mut p, f, sans, &d.clock, col),
            "total" => {
                // не влезает — без «Внимание:»/«Беда:» (о беде говорит цвет)
                let full = &d.total.0;
                let short = full.split_once(": ").map_or(full.as_str(), |(_, r)| r);
                let s = if too_wide(f, sans, full) { short } else { full.as_str() };
                put_text(&mut p, f, sans, s, if d.total.1 == "bad" { bad_col } else { col })
            }
            "row" => {
                let i = idx.unwrap_or(0);
                // длинное «ИИ — Claude думает» в узком поле — «Claude думает»
                let full = &d.rows[i].0;
                let short = full.split_once(" — ").map_or(full.as_str(), |(_, r)| r);
                let s = if too_wide(f, sans, full) && i == 2 { short } else { full.as_str() };
                let warn_col = fl["warn_color"].as_str().map(|_| hex(&fl["warn_color"], col)).unwrap_or(col);
                put_text(
                    &mut p,
                    f,
                    sans,
                    s,
                    match d.rows[i].1.as_str() {
                        "bad" => bad_col,
                        "warn" => warn_col,
                        _ => col,
                    },
                );
            }
            "status" if look.pieces.contains_key(&format!("{}_f{fi}@{cs}", look.name)) => {
                // значок как в концепте; под «внимание»/«беда» — перекрашен, галочка заменена на «!» / «×»
                let pc = &look.pieces[&format!("{}_f{fi}@{cs}", look.name)];
                let c = hex(&fl["colors"][st.as_str()], st_col(&st));
                let shape = fl["shape"].as_str().unwrap_or("dot");
                piece(&mut p, pc, rect.x + (rect.w - pc.w) / 2, rect.y + (rect.h - pc.h) / 2, c, &st, shape == "check", cs, shape == "dot");
            }
            "status" => {
                let c = hex(&fl["colors"][st.as_str()], st_col(&st));
                let (cx, cy) = (rect.x + rect.w / 2, rect.y + rect.h / 2);
                match fl["shape"].as_str().unwrap_or("dot") {
                    "check" => status_icon(&mut p, &st, cx - 5 * u - u / 2, cy - 5 * u - u / 2),
                    "square" => p.fill(rect, c),
                    "heart" => {
                        p.icon(&[".hh.hh.", "hhhhhhh", "hhhhhhh", ".hhhhh.", "..hhh..", "...h..."], cx - 3 * u - u / 2, cy - 3 * u, &[('h', c)]);
                    }
                    _ => {
                        let rad = (rect.w.min(rect.h) - 1) / 2;
                        p.disc(cx, cy, rad, c);
                        if let Some(g) = fl["glyph"].as_u64() {
                            type_icon(&mut p, g as usize, cx - 4 * u - u / 2, cy - 4 * u - u / 2, 0xFFFFFF);
                        }
                    }
                }
            }
            "bar" => {
                // как в концепте: колба — тёмный скруглённый контур, внутри светлая обводка, потом заливка
                let v = if idx == Some(3) { d.cpu } else { d.disk_used }.unwrap_or(0.0).clamp(0.0, 100.0);
                let ink = hex(&fl["fill"], 0x0B3A66);
                let back = hex(&fl["back"], 0x8ACDEB);
                let t = u.max(1);
                let rad = (rect.h / 2).min(3 * t);
                p.rrect(rect, rad, ink, ink);
                let inner = Rect::new(rect.x + t, rect.y + t, rect.w - 2 * t, rect.h - 2 * t);
                p.rrect(inner, (rad - t).max(0), back, back);
                let well = Rect::new(inner.x + t, inner.y + t, inner.w - 2 * t, inner.h - 2 * t);
                let fw = ((well.w as f64) * v / 100.0).round() as i32;
                if well.h > 0 {
                    p.fill(Rect::new(well.x, well.y, fw.max(if v > 0.0 { t } else { 0 }).min(well.w), well.h), ink);
                }
            }
            "signal" => {
                let on = d.rows[0].1 == "ok";
                let n = 4;
                let bw = (rect.w / (2 * n - 1)).max(1);
                for b in 0..n {
                    let bh = (rect.h * (b + 1) / n).max(1);
                    let lit = on || b == 0;
                    p.fill(Rect::new(rect.x + b * 2 * bw, rect.bottom() - bh, bw, bh), if lit { col } else { hex(&fl["off"], 0x7FC4E6) });
                }
            }
            "toggle" => {
                let on = d.rows[1].1 == "ok";
                let bg = if on { hex(&fl["on"], 0x1F5FA8) } else { hex(&fl["off"], 0x7FA9C6) };
                let rad = rect.h / 2;
                p.rrect(rect, rad, bg, bg);
                p.disc(if on { rect.right() - rad - 1 } else { rect.x + rad }, rect.y + rad, (rad - 1).max(1), 0xFFFFFF);
            }
            "dots" => {
                // «…»: ИИ думает — три точки горят, иначе одна
                let thinking = d.rows[2].0.contains("думает");
                for i in 0..3 {
                    if thinking || i == 0 {
                        p.fill(Rect::new(rect.x + i * (rect.w / 3), rect.y + rect.h / 2 - u, 2 * u, 2 * u), col);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(c) = crt {
        crt_glow(&mut p, Rect::new(c.x * cs, rowy(c.y), c.w * cs, rowy(c.y + c.h) - rowy(c.y)), cs, look.t);
    }
    let close = r(&spec["close"]);
    if show_close {
        mini_close(&mut p, Rect::new(close.x + (close.w - 9 * u) / 2, close.y + (close.h - 9 * u) / 2, 9 * u, 9 * u));
    }
    (p, Hits { close, cat: r(&spec["cat"]), list: r(&spec["list"]) })
}

/// Цвет → (оттенок 0..360, насыщенность 0..1, яркость 0..1) и обратно.
fn hsv(v: u32) -> (f64, f64, f64) {
    let (r, g, b) = (((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let d = mx - mn;
    let h = if d == 0.0 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((h + 360.0) % 360.0, if mx == 0.0 { 0.0 } else { d / mx }, mx)
}

fn from_hsv(h: f64, s: f64, v: f64) -> u32 {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let ch = |t: f64| (((t + m) * 255.0).round() as i64).clamp(0, 255) as u32;
    ch(r) << 16 | ch(g) << 8 | ch(b)
}

fn mix_rgb(a: u32, b: u32, t: f64) -> u32 {
    let ch = |s: u32| (((a >> s) & 255) as f64 * (1.0 - t) + ((b >> s) & 255) as f64 * t).round() as u32;
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

fn luma(v: u32) -> f64 {
    let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
    0.3 * r as f64 + 0.59 * g as f64 + 0.11 * b as f64
}

fn dist(a: u32, b: u32) -> i32 {
    let d = |s: u32| (((a >> s) & 255) as i32 - ((b >> s) & 255) as i32).abs();
    d(16) + d(8) + d(0)
}

/// Значок состояния из концепта в (x, y). Фон значка — цвет его краёв; сам значок (не фон и не белое) —
/// перекрашивается в цвет состояния c с той же светлотой (если цвет концепта другой). У галочки (check)
/// под «внимание»/«беда» белая галочка закрашивается и рисуется «!» или «×» клетками.
#[allow(clippy::too_many_arguments)]
fn piece(p: &mut Px, pc: &Sprite, x: i32, y: i32, c: u32, st: &str, check: bool, cs: i32, hue_shift: bool) {
    let at = |i: i32, j: i32| pc.px[(j * pc.w + i) as usize];
    // фон — средний цвет углов (значок круглый — углы вне его)
    let corners = [at(0, 0), at(pc.w - 1, 0), at(0, pc.h - 1), at(pc.w - 1, pc.h - 1)];
    let avg = |sh: u32| corners.iter().map(|v| (v >> sh) & 255).sum::<u32>() / 4;
    let bg = avg(16) << 16 | avg(8) << 8 | avg(0);
    let white = |v: u32| ((v >> 16) & 255).min((v >> 8) & 255).min(v & 255) > 205;
    let ink = |v: u32| v >> 24 >= 128 && dist(v, bg) > 60 && !white(v);
    // основной цвет значка — средний по «чернилам» средней светлоты
    let (mut n, mut sum) = (0.0, [0.0f64; 3]);
    for j in 0..pc.h {
        for i in 0..pc.w {
            let v = at(i, j);
            if ink(v) {
                n += 1.0;
                sum[0] += ((v >> 16) & 255) as f64;
                sum[1] += ((v >> 8) & 255) as f64;
                sum[2] += (v & 255) as f64;
            }
        }
    }
    let main = if n > 0.0 { ((sum[0] / n) as u32) << 16 | ((sum[1] / n) as u32) << 8 | (sum[2] / n) as u32 } else { c };
    let recolor = if st == "ok" { dist(main, c) > 150 } else { dist(main, c) > 45 };
    // гладкий цветной кружок «Стекла» (градиент, блик, свечение вокруг): сдвинуть оттенок у всего, что
    // близко по оттенку к кружку, — тогда и блики, и свечение становятся цветом состояния, без пятен
    let (mh, ms, _) = hsv(main);
    let (th, tsat, tv) = hsv(c);
    if hue_shift && recolor && ms > 0.25 && tsat > 0.2 {
        let (_, _, mv) = hsv(main);
        for j in 0..pc.h {
            for i in 0..pc.w {
                let v = at(i, j);
                if v >> 24 < 128 {
                    continue;
                }
                let (h, sat, val) = hsv(v);
                let dh = ((h - mh + 540.0) % 360.0 - 180.0).abs();
                let o = if sat > 0.08 && dh < 45.0 {
                    // яркость — к цвету состояния настолько, насколько точка «цветная» (кайма, смешанная с фоном,
                    // остаётся светлой, без тёмного ободка)
                    let t = (sat / ms).min(1.0);
                    from_hsv(th, (sat * tsat / ms).min(1.0), (val * (1.0 + t * (tv / mv.max(0.01) - 1.0))).min(1.0))
                } else {
                    v & 0xFF_FFFF
                };
                p.put(x + i, y + j, o);
            }
        }
        return;
    }
    let swap = check && matches!(st, "warn" | "bad");
    for j in 0..pc.h {
        for i in 0..pc.w {
            let v = at(i, j);
            if v >> 24 < 128 {
                continue;
            }
            let mut o = v;
            // (перекраска — и каймы сглаживания: всё, что заметно отличается от фона, кроме белого значка)
            let a0 = (dist(v, bg) as f64 / dist(main, bg).max(1) as f64).min(1.0);
            if recolor && !white(v) && dist(v, bg) > 15 && dist(v, mix_rgb(bg, main, a0)) < 60 {
                // тёмный значок (сердечко тамагочи) — ровно цветом состояния; цветной — с его светлотой
                let col = if n == 0.0 { c } else { scale_rgb(c, (luma(v) / luma(main).max(1.0)).clamp(0.55, 1.1)) };
                // насколько точка — значок, а не фон (кайма сглаживания): смешать с фоном в той же доле
                let a = (dist(v, bg) as f64 / dist(main, bg).max(1) as f64).min(1.0);
                o = mix_rgb(bg, col, a);
            }
            let (dx, dy) = (2 * i - pc.w + 1, 2 * j - pc.h + 1);
            // (значок вырезан с запасом в клетку: круг — на 2 клетки уже)
            let inner = (pc.w.min(pc.h) - 5 * cs).max(1);
            // (внутри круга — ровная заливка: от галочки не остаётся ни следа, ни тени)
            if swap && dx * dx + dy * dy < inner * inner {
                o = c;
            }
            p.put(x + i, y + j, o);
        }
    }
    if swap {
        // «!» или «×» белым по клеткам, по центру значка
        let g: &[&str] = if st == "warn" {
            &["..##..", "..##..", "..##..", "..##..", "......", "..##.."]
        } else {
            &["##..##", ".####.", "..##..", ".####.", "##..##", "......"]
        };
        let gw = 6 * cs;
        let (ox, oy) = (x + (pc.w - gw) / 2, y + (pc.h - gw) / 2 + if st == "bad" { cs / 2 } else { 0 });
        for (j, row) in g.iter().enumerate() {
            for (i, ch) in row.bytes().enumerate() {
                if ch == b'#' {
                    p.fill(Rect::new(ox + i as i32 * cs, oy + j as i32 * cs, cs, cs), 0xFFFFFF);
                }
            }
        }
    }
}

/// Светлый фон экрана (а не значок, текст или рамка).
fn is_screen_bg(v: u32) -> bool {
    let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
    b > 160 && g > 130 && r < 210 && b > r + 20
}

/// Средний цвет фона экрана (по светлым точкам) — им залить экран ровно.
fn screen_bg(base: &Sprite, c: Rect) -> Option<u32> {
    let (mut n, mut s) = (0u64, [0u64; 3]);
    for y in c.y.max(0)..c.bottom().min(base.h) {
        for x in c.x.max(0)..c.right().min(base.w) {
            let v = base.px[(y * base.w + x) as usize];
            if v >> 24 != 0 && is_screen_bg(v) {
                n += 1;
                s[0] += ((v >> 16) & 255) as u64;
                s[1] += ((v >> 8) & 255) as u64;
                s[2] += (v & 255) as u64;
            }
        }
    }
    (n > 0).then(|| ((s[0] / n) as u32) << 16 | ((s[1] / n) as u32) << 8 | (s[2] / n) as u32)
}

fn scale_rgb(v: u32, k: f64) -> u32 {
    let ch = |c: u32| ((c as f64 * k).round() as i64).clamp(0, 255) as u32;
    ch((v >> 16) & 255) << 16 | ch((v >> 8) & 255) << 8 | ch(v & 255)
}

/// Светлее на lift/256 (к белому).
fn lift(v: u32, lift: u32) -> u32 {
    if lift == 0 {
        return v;
    }
    let ch = |c: u32| c + (((255 - c) * lift) >> 8);
    (v & 0xFF00_0000) | ch((v >> 16) & 255) << 16 | ch((v >> 8) & 255) << 8 | ch(v & 255)
}

/// ЭЛТ: тонкие строки развёртки (по пикселям экрана, не по клеткам картинки) и медленная светлая
/// полоса, сползающая сверху вниз (период 7 с).
fn crt_glow(p: &mut Px, r: Rect, cs: i32, t: f64) {
    let band = (r.h as f64 * 0.22).max(4.0);
    let pos = (t / 7.0).fract() * (r.h as f64 + 2.0 * band) - band;
    for y in r.y.max(0)..r.bottom().min(p.h) {
        let dy = (y - r.y) as f64 - pos;
        let glow = if dy.abs() < band { 0.07 * (1.0 + (dy / band * std::f64::consts::PI).cos()) / 2.0 } else { 0.0 };
        // строки развёртки: тёмная −4%, остальные +2% — в среднем яркость экрана как в концепте (было −5% и 0)
        let scan = if cs >= 2 && (y - r.y) % cs == cs - 1 { -0.04 } else { 0.02 };
        let k = 1.0 + glow + scan;
        if k == 1.0 {
            continue;
        }
        for x in r.x.max(0)..r.right().min(p.w) {
            let i = (y * p.w + x) as usize;
            if p.on[i] {
                let c = p.rgb[i];
                p.rgb[i] = if glow > 0.0 { lift(scale_rgb(c, 1.0 + scan), (glow * 256.0) as u32) } else { scale_rgb(c, k) };
            }
        }
    }
}

/// Приглушить цвет, как в концептах: чуть меньше насыщенности и яркости; экраны (screen) — сильнее,
/// и самые яркие места (подсветка ЖК/ЭЛТ) притушены — не OLED.
fn mute(v: u32, screen: bool) -> u32 {
    let (r, g, b) = ((v >> 16) & 255, (v >> 8) & 255, v & 255);
    let l = (r * 77 + g * 151 + b * 28) >> 8;
    let (desat, dark) = if screen { (60u32, 232u32) } else { (40u32, 246u32) };
    let ch = |c: u32| {
        let c = (c * (256 - desat) + l * desat) >> 8; // к серому
        let mut c = (c * dark) >> 8; // чуть темнее
        if screen && l > 170 {
            c = (c * 240) >> 8; // яркая подсветка экрана — тусклее
        }
        c.min(255)
    };
    0xFF00_0000 | ch(r) << 16 | ch(g) << 8 | ch(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_styles_draw_without_sprites() {
        let d = TplData {
            rows: Default::default(),
            total: ("Всё в порядке".into(), "ok".into()),
            clock: "12:34".into(),
            cpu: Some(12.0),
            disk_used: Some(40.0),
            state: 0,
        };
        let mug = Sprite { w: 20, h: 20, px: vec![0xFF00_0000; 400] };
        let mut f = Fonts::pixel();
        for st in crate::widget_look::STYLES {
            for sz in [Size::Small, Size::Medium] {
                let (p, hits) = draw(st, sz, &d, &Pieces { cat: None, mug: &mug }, &mut f, true);
                assert!(p.w > 20 && p.h > 20, "{st:?} {sz:?}");
                assert!(hits.close.right() <= p.w && hits.close.bottom() <= p.h);
            }
        }
    }
}
