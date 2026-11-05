//! Оживление виджетов «Окно» и «Монитор»: облачко с сердцем покачивается, искорки мерцают, кот на мониторе
//! дышит, моргает и даёт потянуть себя за ушко. Кадры считаются по времени чистыми функциями (их проверяют
//! тесты), а на картинку ложатся готовой «накладкой» поверх неподвижного холста: целые пиксели, без сглаживания,
//! только в нескольких прямоугольниках. Места (в клетках сетки, как поля) — в "anim" разметки вида
//! (art/layers/виджет/целиком/<вид>_<размер>.json):
//!   "bubble": {"rect", "bg"}   — облачко: прямоугольник вокруг него с запасом сверху; bg — столбец чистого фона слева;
//!   "sparks": [{"rect"}]       — искорки: прямоугольники с чистым левым столбцом (фон берётся оттуда);
//!   "breath": {"rect", "hold", "fx"} — тело кота: вся область поднимается на 1 точку, ниже «hold» — плавно к нулю
//!                                (шов дыхания), по бокам — на «fx» клеток к нулю;
//!   "blink":  [{"rect"}]       — глаза: радужка закрашивается кожей снизу (веко остаётся линией);
//!   "ears":   [{"tip", "r", "dir", "hit"}] — ухо: от кончика «tip» в радиусе «r» картинка тянется на «dir»
//!                                клеток (плавно к нулю), «hit» — куда нажимать.

use crate::gfx::Rect;
use crate::widget_px::Px;
use serde_json::Value;

/// Частота отсчётов: картинка перерисовывается, только если кадр изменился.
pub const FPS: f64 = 10.0;
/// Сколько искорок бывает у вида.
pub const MAX_SPARK: usize = 4;
/// Ступеней свечения искорки (0 — погасла, SPARK_FULL — как нарисована).
pub const SPARK_FULL: u8 = 3;
/// Ступеней оттяжки уха (0 — не тронуто).
pub const EAR_STEPS: u8 = 4;
/// Ухо тянется до предела за столько секунд, а потом возвращается за столько.
const EAR_IN: f64 = 0.18;
const EAR_OUT: f64 = 0.30;
/// Дольше этого ухо не держим, даже если кнопку не отпустили.
pub const EAR_HOLD_MAX: f64 = 2.5;
const BUBBLE_PERIOD: f64 = 3.0;
const SPARK_PERIOD: [f64; MAX_SPARK] = [2.6, 1.9, 3.1, 2.3];
const SPARK_DELAY: [f64; MAX_SPARK] = [0.8, 1.6, 2.4, 0.4];
const BLINK_SLOT: f64 = 4.1;
const BLINK_LEN: f64 = 0.2;

/// Кадр оживления: всё в целых шагах, чтобы одинаковые кадры не перерисовывать.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Anim {
    /// на сколько точек облачко приподнято
    pub bubble: i32,
    pub spark: [u8; MAX_SPARK],
    /// на сколько точек поднято тело (дыхание)
    pub breath: i32,
    pub blink: bool,
    /// оттяжка левого и правого уха, 0..=EAR_STEPS
    pub ear: [u8; 2],
    /// смущение (тянут за ухо): мордочка из нарисованного кадра «shy» — зажмурился, румянец
    pub shy: bool,
}

impl Default for Anim {
    /// Покой: всё как нарисовано (картинка без оживления).
    fn default() -> Anim {
        Anim { bubble: 0, spark: [SPARK_FULL; MAX_SPARK], breath: 0, blink: false, ear: [0; 2], shy: false }
    }
}

/// Что умеет вид (по его разметке): лишние части кадра обнуляются, чтобы они не вызывали перерисовку.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Caps {
    pub bubble: bool,
    pub sparks: usize,
    pub breath: bool,
    pub blink: bool,
    pub ears: usize,
}

impl Caps {
    pub fn of(spec: &Value) -> Caps {
        let an = &spec["anim"];
        let n = |k: &str| an[k].as_array().map_or(0, |a| a.len());
        Caps {
            bubble: an["bubble"].is_object(),
            sparks: n("sparks").min(MAX_SPARK),
            breath: an["breath"].is_object(),
            blink: n("blink") > 0,
            ears: n("ears").min(2),
        }
    }
    pub fn any(&self) -> bool {
        self.bubble || self.sparks > 0 || self.breath || self.blink || self.ears > 0
    }
}

/// Насколько приподнято облачко в момент t: плавно вверх и обратно, амплитуда — в точках экрана.
pub fn bubble_px(t: f64, amp: i32) -> i32 {
    let k = (1.0 - (std::f64::consts::TAU * t / BUBBLE_PERIOD).cos()) / 2.0;
    (k * amp as f64).round() as i32
}

/// Амплитуда покачивания: полторы клетки картинки.
pub fn bubble_amp(cs: i32) -> i32 {
    (cs * 3 / 2).max(2)
}

/// Свечение i-й искорки: пока не пришла её очередь — как нарисована; потом то горит, то плавно гаснет и
/// разгорается (у каждой свой период — не мигают разом).
pub fn spark_level(t: f64, i: usize) -> u8 {
    let i = i.min(MAX_SPARK - 1);
    let u = t - SPARK_DELAY[i];
    if u < 0.0 {
        return SPARK_FULL;
    }
    let b = (0.5 + 0.9 * (std::f64::consts::TAU * u / SPARK_PERIOD[i]).cos()).clamp(0.0, 1.0);
    (b * SPARK_FULL as f64).round() as u8
}

/// Дыхание — тот же ритм, что у кота в окне (life.rs): 1 точка вверх на 1,5 с, вниз на 1,9 с.
pub fn breath_px(t: f64) -> i32 {
    crate::life::Life::breath(t, false)
}

fn mix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Моргание: раз в несколько секунд (в «окне» по 4,1 с, внутри окна — случайно, но всегда одинаково),
/// изредка двойное. Глаза закрыты BLINK_LEN секунд.
pub fn blinking(t: f64) -> bool {
    if t < 0.0 {
        return false;
    }
    let n = (t / BLINK_SLOT).floor() as u64;
    let h = mix64(n);
    let start = n as f64 * BLINK_SLOT + 0.8 + (h % 1000) as f64 / 1000.0 * 2.6;
    let d = t - start;
    let double = (h >> 20).is_multiple_of(7);
    (0.0..BLINK_LEN).contains(&d) || (double && (0.45..0.45 + BLINK_LEN).contains(&d))
}

/// Оттяжка уха от 0 до 1: нажато `held` секунд (тянется за EAR_IN), после отпускания — вернулось за EAR_OUT.
/// `since` — сколько прошло с отпускания; `held` тогда — сколько ухо держали.
pub fn ear_env(held: f64, since: Option<f64>) -> f64 {
    let up = (held / EAR_IN).clamp(0.0, 1.0);
    match since {
        None => up,
        Some(s) => up * (1.0 - s / EAR_OUT).clamp(0.0, 1.0),
    }
}

pub fn ear_step(env: f64) -> u8 {
    (env * EAR_STEPS as f64).round() as u8
}

impl Anim {
    /// Покойная жизнь вида в момент t (уши не трогаем): только то, что вид умеет.
    pub fn idle(t: f64, cs: i32, caps: Caps) -> Anim {
        let mut a = Anim::default();
        if caps.bubble {
            a.bubble = bubble_px(t, bubble_amp(cs));
        }
        for i in 0..caps.sparks {
            a.spark[i] = spark_level(t, i);
        }
        if caps.breath {
            a.breath = breath_px(t);
        }
        if caps.blink {
            a.blink = blinking(t);
        }
        a
    }
}

// ───────────────────────────── Пиксели ─────────────────────────────

/// Сетка вида: клетка cs точек, строки по высоте — как в widget_px::whole (шаг по высоте больше).
struct Geo {
    cs: i32,
    ry: f64,
}

impl Geo {
    fn new(spec: &Value, cs: i32) -> Geo {
        let ry =
            spec["step"].as_array().and_then(|a| Some(a.get(1)?.as_f64()? / a.first()?.as_f64()?)).filter(|v| *v > 0.5 && *v < 2.0).unwrap_or(1.0);
        Geo { cs, ry }
    }
    fn rowy(&self, j: i32) -> i32 {
        (j as f64 * self.cs as f64 * self.ry + 0.5).floor() as i32
    }
    fn rect(&self, v: &Value) -> Rect {
        let n = |i: usize| v[i].as_i64().unwrap_or(0) as i32;
        let (y0, y1) = (self.rowy(n(1)), self.rowy(n(1) + n(3)));
        Rect::new(n(0) * self.cs, y0, n(2) * self.cs, y1 - y0)
    }
    fn num(&self, v: &Value) -> f64 {
        v.as_f64().unwrap_or(0.0)
    }
}

/// Копия куска холста (чтобы читать «как было», пока пишем «как стало»).
struct Snap {
    r: Rect,
    rgb: Vec<u32>,
    on: Vec<bool>,
    a: Vec<u8>,
}

impl Snap {
    fn take(p: &Px, r: Rect) -> Snap {
        let r = r.intersect(Rect::new(0, 0, p.w, p.h));
        let (mut rgb, mut on, mut a) = (Vec::new(), Vec::new(), Vec::new());
        for y in r.y..r.bottom() {
            let i = (y * p.w + r.x) as usize;
            rgb.extend_from_slice(&p.rgb[i..i + r.w as usize]);
            on.extend_from_slice(&p.on[i..i + r.w as usize]);
            a.extend_from_slice(&p.a[i..i + r.w as usize]);
        }
        Snap { r, rgb, on, a }
    }
    /// Точка (x, y); за краем куска — ближайшая у края.
    fn at(&self, x: i32, y: i32) -> (u32, bool, u8) {
        let x = x.clamp(self.r.x, self.r.right() - 1);
        let y = y.clamp(self.r.y, self.r.bottom() - 1);
        let i = ((y - self.r.y) * self.r.w + (x - self.r.x)) as usize;
        (self.rgb[i], self.on[i], self.a[i])
    }
}

fn put(p: &mut Px, x: i32, y: i32, v: (u32, bool, u8)) {
    let i = (y * p.w + x) as usize;
    p.rgb[i] = v.0;
    p.on[i] = v.1;
    p.a[i] = v.2;
}

fn inside(p: &Px, r: Rect) -> Rect {
    r.intersect(Rect::new(0, 0, p.w, p.h))
}

/// Сдвиг картинки внутри прямоугольника: новая точка (x, y) — старая точка (x + dx, y + dy), где (dx, dy) даёт f.
/// Читаем из копии прямоугольника с полями margin.
fn warp(p: &mut Px, rect: Rect, margin: i32, f: impl Fn(i32, i32) -> (i32, i32)) {
    let rect = inside(p, rect);
    if rect.is_empty() {
        return;
    }
    let snap = Snap::take(p, Rect::new(rect.x - margin, rect.y - margin, rect.w + 2 * margin, rect.h + 2 * margin));
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            let (dx, dy) = f(x, y);
            if (dx, dy) != (0, 0) {
                put(p, x, y, snap.at(x + dx, y + dy));
            }
        }
    }
}

fn mix_c(a: u32, b: u32, k: u32, of: u32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh) & 255, (b >> sh) & 255);
        (x * (of - k) + y * k + of / 2) / of
    };
    ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// Сумма разниц по каналам.
fn dist(a: u32, b: u32) -> u32 {
    (0..3).map(|i| ((a >> (8 * i)) & 255).abs_diff((b >> (8 * i)) & 255)).sum()
}

/// Облачко (всё, что связано с точкой seed не фоном: контур, сердце, хвостик; пряди волос рядом остаются на месте)
/// поднимается на d точек; на его месте — фон из столбца bg, взятый по строкам.
fn bubble(p: &mut Px, g: &Geo, spec: &Value, d: i32) {
    let rect = inside(p, g.rect(&spec["rect"]));
    if rect.is_empty() || d <= 0 {
        return;
    }
    let bx = ((g.num(&spec["bg"]) as i32) * g.cs + g.cs / 2).clamp(0, p.w - 1);
    let col = Snap::take(p, Rect::new(bx, rect.y, 1, rect.h));
    let snap = Snap::take(p, rect);
    let (sx, sy) =
        (((g.num(&spec["seed"][0]) as i32) * g.cs + g.cs / 2).clamp(rect.x, rect.right() - 1), g.rowy(g.num(&spec["seed"][1]) as i32) + g.cs / 2);
    // облачко — область, связанная с seed, не похожая на фон своей строки
    let is_bubble = |x: i32, y: i32| snap.at(x, y).1 && dist(snap.at(x, y).0, col.at(bx, y).0) > 24;
    let mut mask = vec![false; (rect.w * rect.h) as usize];
    let idx = |x: i32, y: i32| ((y - rect.y) * rect.w + (x - rect.x)) as usize;
    if rect.contains(sx, sy) && is_bubble(sx, sy) {
        let mut stack = vec![(sx, sy)];
        mask[idx(sx, sy)] = true;
        while let Some((x, y)) = stack.pop() {
            for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                if rect.contains(nx, ny) && !mask[idx(nx, ny)] && is_bubble(nx, ny) {
                    mask[idx(nx, ny)] = true;
                    stack.push((nx, ny));
                }
            }
        }
    } else {
        // seed не попал в облачко — двигаем прямоугольник целиком
        mask.fill(true);
    }
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if mask[idx(x, y)] {
                put(p, x, y, col.at(bx, y));
            }
        }
    }
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if mask[idx(x, y)] && y - d >= rect.y {
                put(p, x, y - d, snap.at(x, y));
            }
        }
    }
}

/// Искорка: цвет точек прямоугольника — смесь «фон слева ↔ как нарисовано» (ступень lvl из SPARK_FULL).
fn spark(p: &mut Px, g: &Geo, spec: &Value, lvl: u8) {
    let rect = inside(p, g.rect(&spec["rect"]));
    if rect.is_empty() || lvl >= SPARK_FULL {
        return;
    }
    let col = Snap::take(p, Rect::new(rect.x, rect.y, 1, rect.h));
    for y in rect.y..rect.bottom() {
        let bg = col.at(rect.x, y).0;
        for x in rect.x..rect.right() {
            let i = (y * p.w + x) as usize;
            if p.on[i] {
                p.rgb[i] = mix_c(bg, p.rgb[i], lvl as u32, SPARK_FULL as u32);
            }
        }
    }
}

/// Глаз закрыт: всё в прямоугольнике (радужка, блик) — цвета кожи точкой ниже, над ним остаётся линия века.
fn blink(p: &mut Px, g: &Geo, spec: &Value) {
    let rect = inside(p, g.rect(&spec["rect"]));
    if rect.is_empty() || rect.bottom() >= p.h {
        return;
    }
    let below = Snap::take(p, Rect::new(rect.x, rect.bottom(), rect.w, 1));
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            let v = below.at(x, rect.bottom());
            if p.on[(y * p.w + x) as usize] {
                p.rgb[(y * p.w + x) as usize] = v.0;
            }
        }
    }
}

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ухо: от кончика в радиусе r точки тянутся на dir·lvl/EAR_STEPS (чем дальше от кончика, тем меньше).
fn ear(p: &mut Px, g: &Geo, spec: &Value, lvl: u8) {
    if lvl == 0 {
        return;
    }
    let (cs, k) = (g.cs as f64, lvl as f64 / EAR_STEPS as f64);
    let tx = g.num(&spec["tip"][0]) * cs + cs / 2.0;
    let ty = g.rowy(g.num(&spec["tip"][1]) as i32) as f64 + cs / 2.0;
    let r = g.num(&spec["r"]) * cs;
    let (ux, uy) = (g.num(&spec["dir"][0]) * cs * k, g.num(&spec["dir"][1]) * cs * k);
    let rect = Rect::new((tx - r).floor() as i32, (ty - r).floor() as i32, (2.0 * r).ceil() as i32 + 1, (2.0 * r).ceil() as i32 + 1);
    let margin = ux.abs().max(uy.abs()).ceil() as i32 + 2;
    warp(p, rect, margin, |x, y| {
        let d = ((x as f64 - tx).powi(2) + (y as f64 - ty).powi(2)).sqrt();
        let w = 1.0 - smooth(d / r);
        if w <= 0.0 {
            return (0, 0);
        }
        ((-ux * w).round() as i32, (-uy * w).round() as i32)
    });
}

/// Дыхание: область поднимается на amt точек; ниже «hold» (клетка) вес падает до нуля к низу прямоугольника,
/// по бокам — к нулю на «fx» клеток.
fn breath(p: &mut Px, g: &Geo, spec: &Value, amt: i32) {
    if amt == 0 {
        return;
    }
    let rect = g.rect(&spec["rect"]);
    let hold = g.rowy(g.num(&spec["hold"]) as i32);
    let fx = (g.num(&spec["fx"]) * g.cs as f64).max(1.0);
    let (x0, x1, yb) = (rect.x, rect.right() - 1, rect.bottom());
    warp(p, rect, amt + 1, |x, y| {
        let wy = if y <= hold { 1.0 } else { ((yb - y) as f64 / (yb - hold).max(1) as f64).clamp(0.0, 1.0) };
        let wx = (((x - x0).min(x1 - x)) as f64 / fx).clamp(0.0, 1.0);
        (0, (amt as f64 * wy * wx).round() as i32)
    });
}

/// Наложить кадр `a` на неподвижный холст вида. Покой (Anim::default) холст не меняет.
pub fn apply(p: &mut Px, spec: &Value, cs: i32, a: &Anim) {
    if *a == Anim::default() {
        return;
    }
    let g = Geo::new(spec, cs);
    let an = &spec["anim"];
    if let Some(b) = an.get("bubble").filter(|b| b.is_object()) {
        bubble(p, &g, b, a.bubble);
    }
    for (i, s) in an["sparks"].as_array().into_iter().flatten().take(MAX_SPARK).enumerate() {
        spark(p, &g, s, a.spark[i]);
    }
    for (i, e) in an["ears"].as_array().into_iter().flatten().take(2).enumerate() {
        ear(p, &g, e, a.ear[i]);
    }
    // смущение: нарисованная мордочка (ChatGPT по образцу «румянца» приложения, подогнана под клетки вида);
    // она закрывает глаза сама — моргание поверх не нужно
    let shy = a.shy && an.get("shy").is_some_and(|s| shy_face(p, &g, s));
    if a.blink && !shy {
        for e in an["blink"].as_array().into_iter().flatten() {
            blink(p, &g, e);
        }
    }
    if let Some(b) = an.get("breath").filter(|b| b.is_object()) {
        breath(p, &g, b, a.breath);
    }
}

/// Кадр «shy»: картинка размером с rect (в клетках), прозрачное — не трогать. Клетка — сплошной прямоугольник
/// cs × (высота строки), как клетки самого вида. false — нет картинки (тогда обычное моргание).
fn shy_face(p: &mut Px, g: &Geo, spec: &Value) -> bool {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, Option<std::sync::Arc<crate::widget_px::Sprite>>>>> = OnceLock::new();
    let file = spec["file"].as_str().unwrap_or("");
    let sp = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(file.to_string())
        .or_insert_with(|| crate::widget_px::Sprite::load(file).map(std::sync::Arc::new))
        .clone();
    let Some(sp) = sp else { return false };
    let n = |i: usize| spec["rect"][i].as_i64().unwrap_or(0) as i32;
    let (cx0, cy0) = (n(0), n(1));
    for y in 0..sp.h {
        for x in 0..sp.w {
            let v = sp.px[(y * sp.w + x) as usize];
            if v >> 24 == 0 {
                continue;
            }
            let (y0, y1) = (g.rowy(cy0 + y), g.rowy(cy0 + y + 1));
            p.fill(Rect::new((cx0 + x) * g.cs, y0, g.cs, y1 - y0), v & 0xFF_FFFF);
        }
    }
    true
}

/// Куда нажимать, чтобы потянуть i-е ухо (точки холста).
pub fn ear_hits(spec: &Value, cs: i32) -> Vec<Rect> {
    let g = Geo::new(spec, cs);
    spec["anim"]["ears"].as_array().into_iter().flatten().take(2).map(|e| g.rect(&e["hit"])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_goes_up_and_back() {
        let amp = bubble_amp(3);
        assert_eq!(bubble_px(0.0, amp), 0);
        assert_eq!(bubble_px(BUBBLE_PERIOD / 2.0, amp), amp);
        assert_eq!(bubble_px(BUBBLE_PERIOD, amp), 0);
        // плавно: за один отсчёт не больше точки-двух, вверх и вниз по целым шагам
        let mut prev = 0;
        for i in 0..=(BUBBLE_PERIOD * FPS) as i32 {
            let v = bubble_px(i as f64 / FPS, amp);
            assert!((v - prev).abs() <= 2 && (0..=amp).contains(&v), "{i}: {v}");
            prev = v;
        }
        assert!((2..=6).contains(&amp));
    }

    #[test]
    fn sparks_rest_then_twinkle_apart() {
        for i in 0..MAX_SPARK {
            assert_eq!(spark_level(0.0, i), SPARK_FULL);
        }
        // за 10 секунд каждая гаснет и разгорается, и не все сразу
        for i in 0..MAX_SPARK {
            let l: Vec<u8> = (0..100).map(|k| spark_level(k as f64 / 10.0, i)).collect();
            assert!(l.contains(&0) && l.contains(&SPARK_FULL), "{i}");
        }
        let together = (0..100).filter(|k| (0..MAX_SPARK).all(|i| spark_level(*k as f64 / 10.0, i) == 0)).count();
        assert!(together < 20, "гаснут вместе: {together}");
    }

    #[test]
    fn breath_is_one_point_like_the_app() {
        assert_eq!(breath_px(0.0), 0);
        assert_eq!(breath_px(1.95), 1);
        assert_eq!(breath_px(3.4), 0);
    }

    #[test]
    fn blink_is_rare_and_short() {
        assert!(!blinking(0.0));
        let n = (0..6000).filter(|k| blinking(*k as f64 / 100.0)).count() as f64 / 100.0;
        // за минуту — 15 раз по 0,2 с (с двойными чуть больше): 3–6 с закрытых глаз
        assert!((2.0..8.0).contains(&n), "{n}");
        // между морганиями проходит не меньше секунды
        let mut last_open = 0.0;
        let mut gaps = vec![];
        let mut was = false;
        for k in 0..6000 {
            let t = k as f64 / 100.0;
            let b = blinking(t);
            if b && !was {
                gaps.push(t - last_open);
            }
            if !b {
                last_open = t;
            }
            was = b;
        }
        assert!(gaps.len() >= 8);
    }

    #[test]
    fn ear_comes_and_goes() {
        assert_eq!(ear_step(ear_env(0.0, None)), 0);
        assert_eq!(ear_step(ear_env(EAR_IN, None)), EAR_STEPS);
        assert_eq!(ear_step(ear_env(1.0, None)), EAR_STEPS);
        assert_eq!(ear_step(ear_env(1.0, Some(0.0))), EAR_STEPS);
        assert_eq!(ear_step(ear_env(1.0, Some(EAR_OUT))), 0);
        // отпустили быстро — вернулось с того, докуда дотянули
        assert!(ear_step(ear_env(0.09, Some(0.0))) < EAR_STEPS);
    }

    #[test]
    fn idle_at_zero_is_rest_and_caps_mask() {
        let caps = Caps { bubble: true, sparks: 2, breath: true, blink: true, ears: 2 };
        assert_eq!(Anim::idle(0.0, 3, caps), Anim::default());
        // чего вид не умеет — в кадре не меняется
        let none = Caps::default();
        for k in 0..200 {
            assert_eq!(Anim::idle(k as f64 / 10.0, 3, none), Anim::default());
        }
    }

    fn canvas(w: i32, h: i32) -> Px {
        let mut p = Px::blank(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                p.rgb[i] = ((x * 7 + y * 13) as u32 & 255) << 8 | 0x40;
                p.on[i] = true;
                p.a[i] = 255;
            }
        }
        p
    }

    fn spec() -> Value {
        serde_json::json!({
            "step": [10.0, 10.0],
            "anim": {
                "bubble": {"rect": [2, 2, 10, 12], "bg": 1},
                "sparks": [{"rect": [14, 2, 3, 3]}],
                "breath": {"rect": [2, 0, 14, 12], "hold": 6, "fx": 2},
                "blink": [{"rect": [5, 5, 3, 2]}],
                "ears": [{"tip": [8, 3], "r": 4, "dir": [-1.0, -2.0], "hit": [6, 1, 4, 5]}]
            }
        })
    }

    #[test]
    fn rest_frame_is_pixel_identical() {
        let p0 = canvas(60, 50);
        let mut p = p0.clone();
        apply(&mut p, &spec(), 3, &Anim::default());
        assert!(p.same(&p0));
        // нет разметки "anim" — любой кадр ничего не меняет
        let mut q = p0.clone();
        let bare = serde_json::json!({"step": [10.0, 10.0]});
        apply(&mut q, &bare, 3, &Anim { bubble: 4, spark: [0; MAX_SPARK], breath: 1, blink: true, ear: [4, 4], shy: false });
        assert!(q.same(&p0));
    }

    #[test]
    fn frames_change_only_their_rectangles() {
        let p0 = canvas(60, 50);
        let g = Geo::new(&spec(), 3);
        let moved = |a: Anim| {
            let mut p = p0.clone();
            apply(&mut p, &spec(), 3, &a);
            let mut r = Rect::new(0, 0, 0, 0);
            let mut n = 0;
            for y in 0..p.h {
                for x in 0..p.w {
                    if p.rgb[(y * p.w + x) as usize] != p0.rgb[(y * p0.w + x) as usize] {
                        r = if n == 0 { Rect::new(x, y, 1, 1) } else { r.union(Rect::new(x, y, 1, 1)) };
                        n += 1;
                    }
                }
            }
            (n, r)
        };
        let (n, r) = moved(Anim { bubble: 4, ..Anim::default() });
        assert!(n > 0 && g.rect(&spec()["anim"]["bubble"]["rect"]).intersect(r) == r);
        let (n, r) = moved(Anim { spark: [0, SPARK_FULL, SPARK_FULL, SPARK_FULL], ..Anim::default() });
        assert!(n > 0 && g.rect(&spec()["anim"]["sparks"][0]["rect"]).intersect(r) == r);
        let (n, r) = moved(Anim { blink: true, ..Anim::default() });
        assert!(n > 0 && g.rect(&spec()["anim"]["blink"][0]["rect"]).intersect(r) == r);
        let (n, _) = moved(Anim { breath: 1, ..Anim::default() });
        assert!(n > 0);
        let (n, r) = moved(Anim { ear: [EAR_STEPS, 0], ..Anim::default() });
        assert!(n > 0 && r.w <= 2 * 4 * 3 + 2, "{r:?}");
    }

    #[test]
    fn bubble_moves_whole_pixels_up() {
        // плоский фон и «облачко» 6×6 с центром в клетке (5, 6)
        let mut p0 = Px::blank(60, 50);
        for i in 0..(60 * 50) as usize {
            p0.rgb[i] = 0x406080;
            p0.on[i] = true;
            p0.a[i] = 255;
        }
        for y in 24..30 {
            for x in 12..18 {
                p0.rgb[(y * 60 + x) as usize] = 0xFFFFFF;
            }
        }
        let mut sp = spec();
        sp["anim"]["bubble"]["seed"] = serde_json::json!([5, 8]);
        let mut p = p0.clone();
        apply(&mut p, &sp, 3, &Anim { bubble: 4, ..Anim::default() });
        // верх облачка поднялся ровно на 4 точки, низ освободился фоном
        assert_eq!(p.rgb[(20 * 60 + 14) as usize], 0xFFFFFF);
        assert_eq!(p.rgb[(19 * 60 + 14) as usize], 0x406080);
        assert_eq!(p.rgb[(29 * 60 + 14) as usize], 0x406080);
        assert_eq!(p.rgb[(25 * 60 + 14) as usize], 0xFFFFFF);
    }
}

#[cfg(test)]
mod shy_tests {
    /// Мордочка «смущение» вшита для обоих мониторов и по размеру совпадает со своим местом в разметке.
    #[test]
    fn shy_faces_are_embedded_and_fit() {
        for name in ["monitor_small", "monitor_medium"] {
            let raw = crate::art::read("", &format!("виджет/целиком/{name}.json")).expect("разметка");
            let spec: serde_json::Value = serde_json::from_slice(&raw).expect("json");
            let s = &spec["anim"]["shy"];
            let sp = crate::widget_px::Sprite::load(s["file"].as_str().unwrap()).expect("картинка мордочки");
            assert_eq!((sp.w as i64, sp.h as i64), (s["rect"][2].as_i64().unwrap(), s["rect"][3].as_i64().unwrap()), "{name}");
            assert!(sp.px.iter().any(|v| v >> 24 != 0), "{name}: пустая");
        }
    }
}
