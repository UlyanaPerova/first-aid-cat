//! «Жизнь» картинки: дыхание и покадровые движения (моргание, уши, рука с луком, хвост,
//! огурец в банке, листья). Тайминги — как в прежней версии Аптечки.
//!
//! Дыхание: на вдохе голова, шея, плечи и грудь поднимаются на клетку, низ (планшет, руки
//! на нём, ноги) стоит. Между ними — шов дыхания (art/layers/дыхание.json, считает
//! tools/breath_seam.py): строка шва повторяется, свитер чуть растягивается.
//! Тени и блик при дыхании не двигаются.
//!
//! Кадры — «позы» слоёв (art/layers/позы, переносит tools/port_frames.py): в нужный момент
//! слой рисуется в позе кадра вместо обычного вида.

use std::collections::HashMap;

use crate::scene::{Draw, Scene};

/// Простой генератор случайных чисел (xorshift), чтобы не тянуть зависимость.
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn uniform(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
}

/// Дыхание: (подъём, длительность). Голова, шея и плечи — вместе: если плечи опережают
/// голову хоть на кадр, воротник наезжает на подбородок и шея будто сжимается.
const BREATH: [(i32, f64); 2] = [(0, 1.9), (1, 1.5)];
/// Частое дыхание, когда кот беспокоится.
const BREATH_FAST: [(i32, f64); 2] = [(0, 0.75), (1, 0.6)];
/// Капля пота при беспокойстве: сползает по виску и повторяется.
const SWEAT: &[(&str, f64)] = &[("пот_1", 0.5), ("пот_2", 0.45), ("пот_3", 0.7), ("", 0.35)];

// Покадровые движения: (кадр или "" — обычный вид, длительность). Повторяются по кругу.
// Крайние позы держим дольше — движение «замедляется» у разворота.
const HAND: &[(&str, f64)] = &[
    ("", 0.8),
    ("hand_11", 0.2),
    ("hand_10", 0.35),
    ("hand_11", 0.2),
    ("", 0.3),
    ("hand_20", 0.2),
    ("hand_30", 0.35),
    ("hand_20", 0.2),
    ("", 0.25),
    ("hand_11", 0.2),
    ("", 0.2),
    ("hand_21", 0.22),
    ("", 1.4),
];
const TAIL: &[(&str, f64)] = &[
    ("", 3.2),
    ("tail_P1", 0.28),
    ("tail_P2", 0.5),
    ("tail_P1", 0.28),
    ("", 3.6),
    ("tail_P1", 0.2),
    ("tail_P2", 0.22),
    ("tail_P3", 1.0),
    ("tail_P2", 0.26),
    ("tail_P1", 0.24),
];
const PICKLE: &[(&str, f64)] = &[("", 1.3), ("pickle_U", 1.5), ("", 1.1), ("pickle_D", 1.6)];
const LEAF_M: &[(&str, f64)] = &[("", 2.2), ("leafM_R", 0.55), ("", 0.45), ("leafM_L", 0.7), ("", 1.6), ("leafM_L", 0.5), ("", 0.9)];
const LEAF_B: &[(&str, f64)] = &[("", 1.4), ("leafB_L", 0.6), ("", 0.5), ("leafB_R", 0.6), ("", 2.6), ("leafB_R", 0.5), ("", 0.7)];

/// Подёргивания хвоста при смущении (от тычка, без повтора).
const SHY_TAIL: &[(&str, f64)] = &[
    ("", 0.25),
    ("tail_P1", 0.07),
    ("tail_P2", 0.09),
    ("tail_P1", 0.07),
    ("", 0.12),
    ("tail_P1", 0.06),
    ("tail_P2", 0.08),
    ("tail_P1", 0.06),
    ("", 1.3),
    ("tail_P1", 0.07),
    ("tail_P2", 0.09),
    ("tail_P1", 0.07),
];

/// Хвост при возмущении: быстрые нервные взмахи сериями.
const ANGRY_TAIL: &[(&str, f64)] = &[
    ("", 0.1),
    ("tail_P1", 0.05),
    ("tail_P2", 0.05),
    ("tail_P3", 0.12),
    ("tail_P2", 0.05),
    ("tail_P1", 0.05),
    ("", 0.18),
    ("tail_P1", 0.05),
    ("tail_P2", 0.05),
    ("tail_P3", 0.12),
    ("tail_P2", 0.05),
    ("tail_P1", 0.05),
    ("", 0.6),
    ("tail_P1", 0.05),
    ("tail_P2", 0.05),
    ("tail_P3", 0.12),
    ("tail_P2", 0.05),
    ("tail_P1", 0.05),
];

/// Последовательность один раз (после конца — обычный вид).
fn timeline_once(t: f64, seq: &[(&'static str, f64)]) -> &'static str {
    let mut k = t;
    for &(st, d) in seq {
        if k < d {
            return st;
        }
        k -= d;
    }
    ""
}

fn timeline(t: f64, seq: &[(&'static str, f64)]) -> &'static str {
    let total: f64 = seq.iter().map(|p| p.1).sum();
    let mut k = t.rem_euclid(total);
    for &(st, d) in seq {
        if k < d {
            return st;
        }
        k -= d;
    }
    seq[seq.len() - 1].0
}

pub struct Life {
    rng: Rng,
    lifted: Vec<usize>, // поднимаются по шву при вдохе
    pub seam: Vec<i32>,
    /// кадр → [(слой, номер позы)]
    frames: HashMap<String, Vec<(usize, usize)>>,
    next_blink: f64,
    blink_t0: f64,
    ear_next: f64,
    ear_t0: f64,
    ear_left: bool,
    ear_double: bool,
    pub writing: bool,
    /// качающиеся от тычка слои: (слой, время тычка)
    pokes: Vec<(usize, f64)>,
    /// смущение: (начало, левое ли ухо ткнули, до какого времени прятаться)
    shy: Option<(f64, bool, f64)>,
    /// возмущение: (начало, до какого времени сердится)
    angry: Option<(f64, f64)>,
    /// беспокойство от тычка: до какого времени
    worry_until: f64,
    /// беспокойство, пока есть проблема (его будет включать диагностика)
    pub trouble: bool,
    names: Vec<String>,
    /// клетки ножек (чулки и открытая коленка) внутри слоя «тело» — тычок в них смущает
    legs: Vec<bool>,
    w: usize,
}

/// Ножки в слое «тело»: тёмные чулки и светлый кусочек коленки внизу, у робота.
/// Свитер и планшет — не ножки (тычок туда — без реакции).
fn legs_mask(sc: &Scene) -> Vec<bool> {
    let mut m = vec![false; sc.w * sc.h];
    let Some(body) = sc.layers.iter().find(|l| l.name == "тело") else { return m };
    for y in 178..sc.h {
        for x in 110..183.min(sc.w) {
            let p = body.at(x, y);
            if p >> 24 == 0 {
                continue;
            }
            let (r, g, b) = ((p >> 16) & 255, (p >> 8) & 255, p & 255);
            let lum = (r * 3 + g * 6 + b) / 10;
            let stocking = lum < 60 && x >= 116; // чёрный чулок (без рамки планшета слева)
            let knee = y >= 205 && x >= 140 && lum >= 60; // светлое внизу: коленка/ножка
            m[y * sc.w + x] = stocking || knee;
        }
    }
    m
}

impl Life {
    pub fn new(sc: &Scene, dir: &str, seed: u64) -> Life {
        let meta: serde_json::Value =
            serde_json::from_str(&crate::art::read_string(dir, "дыхание.json").expect("дыхание.json (tools/breath_seam.py)"))
                .expect("дыхание.json: не JSON");
        let seam: Vec<i32> = meta["seam"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap() as i32).collect();
        assert_eq!(seam.len(), sc.w, "шов дыхания не под ширину картинки");
        let lifted = meta["layers"].as_array().unwrap().iter().map(|n| sc.index(n.as_str().unwrap())).collect();
        let mut frames: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (i, l) in sc.layers.iter().enumerate() {
            for (k, (name, _, _)) in l.poses.iter().enumerate() {
                frames.entry(name.clone()).or_default().push((i, k));
            }
        }
        let mut rng = Rng::new(seed);
        let next_blink = rng.uniform(1.0, 3.0);
        Life {
            rng,
            lifted,
            seam,
            frames,
            next_blink,
            blink_t0: -10.0,
            ear_next: 4.0,
            ear_t0: -10.0,
            ear_left: true,
            ear_double: false,
            writing: true,
            pokes: Vec::new(),
            shy: None,
            angry: None,
            worry_until: -1.0,
            trouble: false,
            names: sc.layers.iter().map(|l| l.name.clone()).collect(),
            legs: legs_mask(sc),
            w: sc.w,
        }
    }

    /// Моргание: раз в 3–7 с, изредка двойное.
    fn eyes(&mut self, t: f64) -> &'static str {
        if t > self.next_blink {
            self.blink_t0 = t;
            self.next_blink = t + self.rng.uniform(3.0, 7.0);
            if self.rng.f() < 0.08 {
                self.next_blink = t + 0.5;
            }
        }
        let b = t - self.blink_t0;
        if b < 0.0 || b >= 0.30 {
            ""
        } else if b < 0.08 || b >= 0.20 {
            "eyes_half"
        } else {
            "eyes_closed"
        }
    }

    /// До какого мига идёт короткое движение (моргание 0,3 с, дёрг ушка): пока оно идёт, окно рисует в полном темпе,
    /// иначе при редких кадрах (окно без фокуса — 4 кадра в секунду) из трёх фаз век попадала одна, и кот «моргал
    /// не до конца»: видны были только полуприкрытые глаза.
    pub fn quick_until(&self) -> f64 {
        let ear = self.ear_t0 + if self.ear_double { 0.50 } else { 0.18 };
        (self.blink_t0 + 0.30).max(ear)
    }

    /// Дёрг ушка: случайно левое или правое, раз в 5–12 с, иногда двойной.
    /// Кадры: half 50 мс → flick 70 мс → half 60 мс. С морганием не совпадает.
    fn ear(&mut self, t: f64) -> String {
        if t > self.ear_next {
            if (t - self.blink_t0).abs() < 0.4 || (self.next_blink - t).abs() < 0.4 {
                self.ear_next = t + 0.5;
            } else {
                self.ear_t0 = t;
                self.ear_left = self.rng.f() < 0.5;
                self.ear_double = self.rng.f() < 0.13;
                self.ear_next = t + self.rng.uniform(5.0, 12.0);
            }
        }
        let mut k = t - self.ear_t0;
        if self.ear_double && k >= 0.32 {
            k -= 0.32;
        }
        let st = if (0.0..0.05).contains(&k) || (0.12..0.18).contains(&k) {
            "half"
        } else if (0.05..0.12).contains(&k) {
            "flick"
        } else {
            return String::new();
        };
        format!("ear_{}_{st}", if self.ear_left { "L" } else { "R" })
    }

    pub(crate) fn breath(t: f64, fast: bool) -> i32 {
        let seq = if fast { BREATH_FAST } else { BREATH };
        let total: f64 = seq.iter().map(|p| p.1).sum();
        let mut k = t.rem_euclid(total);
        for (b, d) in seq {
            if k < d {
                return b;
            }
            k -= d;
        }
        0
    }

    fn pose(&self, draws: &mut [Draw], frame: &str) {
        if let Some(list) = self.frames.get(frame) {
            for &(i, k) in list {
                draws[i].pose = Some(k);
            }
        }
    }

    /// Беспокоится ли сейчас: после тычка в хвост или пока есть проблема (trouble).
    pub fn worried(&self, t: f64) -> bool {
        self.trouble || t < self.worry_until
    }

    /// Точка (x, y) — ножки (чулок или коленка).
    pub fn is_legs(&self, x: usize, y: usize) -> bool {
        self.legs.get(y * self.w + x).copied().unwrap_or(false)
    }

    /// Тычок мышкой в слой. Возвращает, на что отреагировали (для лога).
    pub fn poke(&mut self, layer: usize, x: usize, y: usize, t: f64) -> Option<&str> {
        let name = self.names[layer].as_str();
        // ножки (чулок, коленка) — тоже смущение. По месту на картинке, а не по слою сверху: позы кота
        // (моргание, румянец, возмущение…) нарисованы целой фигурой в слое «волосы» и закрывают ноги —
        // тычок в чулок посреди моргания читался как тычок в голову («у меня причёска!»)
        let legs = matches!(name, "тело" | "волосы" | "лицо") && self.legs.get(y * self.w + x).copied().unwrap_or(false);
        // пока прячется, вся голова с руками — одна поза в слое «волосы»: тычок в неё = в ухо
        let hiding = self.shy.is_some() && self.angry.is_none() && matches!(name, "волосы" | "лицо");
        if name == "ухо_левое" || name == "ухо_правое" || hiding || legs {
            match &mut self.shy {
                // уже смущён и прячется — ткнули ещё: прячется дольше
                Some((t0, _, until)) if t - *t0 > 0.5 && t < *until => {
                    *until = (*until).max(t + 2.0);
                    return Some("смущение: дольше");
                }
                Some((t0, _, until)) if t < *until + 1.6 && t - *t0 <= 0.5 => return Some("уже смущается"),
                _ => {
                    self.shy = Some((t, name != "ухо_правое", t + 3.2));
                    return Some("смущение");
                }
            }
        }
        // тычок в хвост — беспокойство (ещё тычок — дольше)
        if name == "хвост" {
            self.worry_until = self.worry_until.max(t + 3.5);
            return Some("беспокойство");
        }
        // тычок в голову (не в ухо) — возмущение; ещё тычок, пока сердится, — сердится дольше
        if matches!(name, "волосы" | "лицо") {
            match &mut self.angry {
                Some((_, until)) if t < *until => {
                    *until = (*until).max(t + 2.0);
                    return Some("возмущение: дольше");
                }
                _ => {
                    self.angry = Some((t, t + 2.8));
                    return Some("возмущение");
                }
            }
        }
        if name.starts_with("растение_") {
            self.pokes.retain(|p| p.0 != layer);
            self.pokes.push((layer, t));
            return Some("растение качается");
        }
        None
    }

    /// Шаг времени: заполнить, как рисовать слои в этом кадре.
    pub fn step(&mut self, t: f64, _dt: f64, draws: &mut [Draw]) {
        for d in draws.iter_mut() {
            *d = Draw::default();
        }
        let worried = self.worried(t);
        let b = Self::breath(t, worried);
        for &i in &self.lifted {
            draws[i].lift = b;
        }
        // смущён или сердится: не пишет, обычные моргания, дёрги ушей и хвост ждут
        let shy = self.shy.is_some() || self.angry.is_some() || worried;
        let eyes = self.eyes(t);
        let ear = self.ear(t);
        if !shy {
            self.pose(draws, eyes);
            self.pose(draws, &ear);
        }
        if self.writing && !shy {
            self.pose(draws, timeline(t, HAND));
        }
        if !shy {
            self.pose(draws, timeline(t, TAIL));
        }
        self.pose(draws, timeline(t, PICKLE));
        self.pose(draws, timeline(t, LEAF_M));
        self.pose(draws, timeline(t + 2.3, LEAF_B));
        // смущение: дёрг уха → румянец (глаза зажмурены, ушки прижаты), хвостик дёргается → снова пишет
        if let Some((t0, left, until)) = self.shy {
            let k = t - t0;
            let side = if left { "L" } else { "R" };
            if k < 0.05 || (0.12..0.18).contains(&k) {
                self.pose(draws, &format!("ear_{side}_half"));
            } else if k < 0.12 {
                self.pose(draws, &format!("ear_{side}_flick"));
            }
            // ход смущения (k — от тычка, until — до когда смущён): дёрг уха → румянец → обычный
            let r = t - until;
            // румянец с зажмуренными глазами и прижатыми ушками, пока смущён
            let face = if k >= 0.18 && r < 0.0 { "румянец" } else { "" };
            if k >= 0.18 && r < 0.9 {
                // ушки прижаты — нарисованы в самих позах смущения
            }
            // хвостик дёргается: два коротких подёргивания сразу и ещё одно, пока прячется
            self.pose(draws, timeline_once(k, SHY_TAIL));
            self.pose(draws, face);
            if r >= 0.8 {
                self.shy = None; // успокоился — снова пишет
            }
        }

        // беспокойство: брови домиком, капля пота по виску, хвост замер, дыхание чаще.
        // Смущение и возмущение (реакции на тычок) важнее — поверх
        if worried && self.shy.is_none() && self.angry.is_none() {
            self.pose(draws, "беспокойство");
            self.pose(draws, timeline(t, SWEAT));
        }

        // возмущение: нахмурился, ушки развёрнуты назад, хвост хлещет → снова пишет
        if let Some((t0, until)) = self.angry {
            let k = t - t0;
            if t < until {
                self.pose(draws, "возмущение");
                // значок злости 💢 пульсирует: побольше — поменьше
                if k >= 0.15 {
                    self.pose(draws, if ((k - 0.15) / 0.22) as i64 % 2 == 0 { "злость_2" } else { "злость_1" });
                }
            }
            self.pose(draws, timeline_once(k, ANGRY_TAIL));
            if t >= until + 0.8 {
                self.angry = None;
            }
        }

        // покачивание после тычка: верх отклоняется на 2 клетки, затухает за ~1,5 с
        self.pokes.retain(|&(_, t0)| t - t0 < 1.8);
        for &(i, t0) in &self.pokes {
            let k = t - t0;
            draws[i].sway = (2.2 * (k * std::f64::consts::TAU * 1.6).sin() * (-k / 0.55).exp()) as f32;
        }
    }
}

/// Шаг до следующего кадра с учётом короткого движения: пока оно идёт (`t < until`) — не реже полного темпа.
pub fn quick_step(step: f64, alive: bool, t: f64, until: f64, fps: f64) -> f64 {
    if alive && t < until {
        step.min(1.0 / fps)
    } else {
        step
    }
}

#[cfg(test)]
mod quick_tests {
    use super::quick_step;

    /// Какие фазы век увидит человек, если окно без фокуса (кадр раз в 0,25 с при 20 кадрах/с), а моргание
    /// начинается на первом кадре после `start`. Фазы — по расписанию моргания: полу до 0,08 с, закрыты до 0,20 с,
    /// полу до 0,30 с (выписано здесь заново, не через Life::eyes).
    fn seen(start: f64, quick: bool) -> Vec<u8> {
        let (fps, slow) = (20.0, 0.25);
        let (mut t, mut t0, mut out) = (0.0f64, f64::MAX, Vec::new());
        while t < start + 2.0 {
            if t0 == f64::MAX && t > start {
                t0 = t;
            }
            let b = t - t0;
            let phase = if !(0.0..0.30).contains(&b) {
                0
            } else if !(0.08..0.20).contains(&b) {
                1
            } else {
                2
            };
            if out.last() != Some(&phase) {
                out.push(phase);
            }
            let until = if t0 == f64::MAX { f64::MIN } else { t0 + 0.30 };
            t += quick_step(slow, quick, t, until, fps);
        }
        out
    }

    #[test]
    fn blink_shows_all_phases_at_slow_frame_rate() {
        // с полным темпом на время моргания: открыты, полу, закрыты, полу, открыты — при любом миге начала
        for i in 0..50 {
            assert_eq!(seen(3.0 + f64::from(i) * 0.013, true), [0, 1, 2, 1, 0], "{i}");
        }
        // без него (как было): кадр раз в 0,25 с попадает только в первую фазу — «моргает не до конца»
        assert_eq!(seen(3.0, false), [0, 1, 0]);
        // движение кончилось или окно скрыто — шаг прежний
        assert_eq!(quick_step(0.25, true, 5.0, 4.9, 20.0), 0.25);
        assert_eq!(quick_step(1.0, false, 5.0, 9.0, 20.0), 1.0);
        assert_eq!(quick_step(0.02, true, 5.0, 9.0, 20.0), 0.02);
    }
}
