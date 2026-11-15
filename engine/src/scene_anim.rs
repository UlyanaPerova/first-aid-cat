// Анимация сценки плашки: что в какой миг включено (моргание, кардиограмма, огурчик, «z») и сборка картинки.
// Данные кадров — scene_anim_px.rs, фон и коты — scene_px.rs. Время — целые миллисекунды от открытия плашки.

use crate::scene_anim_px::{Patch, BLINK_DONE, BLINK_WRITE, ECG, PICKLE, Z};
use crate::scene_px::{PAL, W};

/// Что включено в один миг. `Frame::default()` — «без анимации»: сценка как в статике.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    /// веки: 0 открыты, 1 полуприкрыты, 2 закрыты
    pub blink: u8,
    /// кадр кардиограммы; None — анимации нет
    pub ecg: Option<u8>,
    /// огурчик: 0 покой, 1 вверх, 2 вниз
    pub pickle: u8,
    /// «z» у спящего: 0 — как в слое, 1 и 2 — кадры
    pub z: u8,
}

const ECG_MS: u64 = 120;
const BLINK_STEP_MS: u64 = 70;
/// веки: полуприкрыты, закрыты, полуприкрыты — по шагу каждое
const BLINK_MS: u64 = 3 * BLINK_STEP_MS;
/// огурчик: (конец отрезка в мс от начала цикла, поза)
const PICKLE_TL: [(u64, u8); 4] = [(1300, 0), (2800, 1), (3900, 0), (5500, 2)];
const Z_MS: u64 = 900;
/// пауз между морганиями хватает на цикл из стольких морганий, дальше они повторяются
const BLINKS_IN_LOOP: u64 = 16;

/// Случайное, но всегда одно и то же число по номеру (splitmix64) — моргание не выглядит механическим.
fn hash(i: u64) -> u64 {
    let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 30)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Пауза перед морганием номер i: от 2,5 до 6 с.
fn pause_ms(i: u64) -> u64 {
    2500 + hash(i) % 3501
}

/// Мигаем ли только у «пишет» и «закончил» (у спящего глаза закрыты в рисунке).
fn blinks(state: u8) -> bool {
    state == 1 || state == 2
}

/// Весь круг морганий в мс.
fn blink_loop_ms() -> u64 {
    (0..BLINKS_IN_LOOP).map(|i| pause_ms(i) + BLINK_MS).sum()
}

/// Моргание в миг `t` (мс): шаг веков и когда он кончится (мс, считая от начала круга; вызывающий добавляет витки).
fn blink_at(t: u64) -> (u8, u64) {
    let mut o = t % blink_loop_ms();
    let mut start = 0;
    for i in 0..BLINKS_IN_LOOP {
        let p = pause_ms(i);
        if o < p {
            return (0, start + p);
        }
        o -= p;
        start += p;
        if o < BLINK_MS {
            let s = o / BLINK_STEP_MS;
            return (if s == 1 { 2 } else { 1 }, start + (s + 1) * BLINK_STEP_MS);
        }
        o -= BLINK_MS;
        start += BLINK_MS;
    }
    (0, start)
}

fn pickle_at(t: u64) -> (u8, u64) {
    let o = t % PICKLE_TL[3].0;
    let (end, pose) = PICKLE_TL.iter().copied().find(|(e, _)| o < *e).unwrap_or(PICKLE_TL[3]);
    (pose, end)
}

/// Что включено в миг `t` мс для состояния `state` (0 спит, 1 пишет, 2 закончил).
pub fn frame(state: u8, t: u64) -> Frame {
    Frame {
        blink: if blinks(state) { blink_at(t).0 } else { 0 },
        ecg: Some((t / ECG_MS % ECG.len() as u64) as u8),
        pickle: pickle_at(t).0,
        z: if state == 0 { (t / Z_MS % 3) as u8 } else { 0 },
    }
}

/// Когда (мс) кадр сменится в следующий раз — всегда позже `t`.
pub fn next_change(state: u8, t: u64) -> u64 {
    let mut n = (t / ECG_MS + 1) * ECG_MS;
    let lap = |len: u64, end_in_lap: u64| t / len * len + end_in_lap;
    n = n.min(lap(PICKLE_TL[3].0, pickle_at(t).1));
    if blinks(state) {
        n = n.min(lap(blink_loop_ms(), blink_at(t).1));
    }
    if state == 0 {
        n = n.min((t / Z_MS + 1) * Z_MS);
    }
    n
}

/// Заплатка в виде списка точек (x, y, цвет 0xRRGGBB).
pub struct Cells(Vec<(i32, i32, u32)>);

fn decode(p: &Patch) -> Cells {
    let (x0, y0, w, h, s) = *p;
    let mut v = Vec::new();
    let mut i = 0;
    for pair in s.bytes().filter(|c| *c >= b'0').collect::<Vec<u8>>().as_chunks::<2>().0 {
        for _ in 0..=(pair[1] - b'0') {
            let c = pair[0] - b'0';
            if c > 0 && i < w * h {
                if let Some(rgb) = PAL.get(usize::from(c) - 1) {
                    v.push((x0 + i % w, y0 + i / w, *rgb));
                }
            }
            i += 1;
        }
    }
    Cells(v)
}

struct Decoded {
    ecg: Vec<Cells>,
    pickle: Vec<Cells>,
    blink_write: Vec<Cells>,
    blink_done: Vec<Cells>,
    z: Vec<Cells>,
}

fn decoded() -> &'static Decoded {
    static D: std::sync::OnceLock<Decoded> = std::sync::OnceLock::new();
    D.get_or_init(|| Decoded {
        ecg: ECG.iter().map(decode).collect(),
        pickle: PICKLE.iter().map(decode).collect(),
        blink_write: BLINK_WRITE.iter().map(decode).collect(),
        blink_done: BLINK_DONE.iter().map(decode).collect(),
        z: Z.iter().map(decode).collect(),
    })
}

/// Заплатки кадра `f` для состояния `state`, по порядку наложения.
fn overlays(state: u8, f: &Frame) -> Vec<&'static Cells> {
    let d = decoded();
    let mut v = Vec::new();
    if let Some(e) = f.ecg {
        v.extend(d.ecg.get(usize::from(e)));
    }
    if f.pickle > 0 {
        v.extend(d.pickle.get(usize::from(f.pickle) - 1));
    }
    if f.z > 0 && state == 0 {
        v.extend(d.z.get(usize::from(f.z) - 1));
    }
    if f.blink > 0 {
        let b = if state == 1 { &d.blink_write } else { &d.blink_done };
        if blinks(state) {
            v.extend(b.get(usize::from(f.blink) - 1));
        }
    }
    v
}

/// Сценка W×H (0xRRGGBB): статическая картинка состояния и поверх неё заплатки кадра.
pub fn compose(state: u8, still: &[u32], f: &Frame) -> Vec<u32> {
    let mut px = still.to_vec();
    for c in overlays(state, f) {
        for &(x, y, rgb) in &c.0 {
            if let Some(p) = px.get_mut((y * W + x) as usize) {
                *p = rgb;
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_px::H;

    #[test]
    fn blink_sequence_and_pause() {
        // шаги веков подряд: пауза, полу, закрыты, полу, пауза
        let mut seen = Vec::new();
        let mut t = 0;
        let mut last = 99;
        while t < 20_000 {
            let b = frame(1, t).blink;
            if b != last {
                seen.push((b, t));
                last = b;
            }
            t = next_change(1, t);
        }
        let seq: Vec<u8> = seen.iter().map(|s| s.0).collect();
        assert_eq!(&seq[..6], &[0, 1, 2, 1, 0, 1]);
        // шаг — 70 мс, пауза — от 2,5 до 6 с
        assert_eq!(seen[2].1 - seen[1].1, 70);
        assert_eq!(seen[3].1 - seen[2].1, 70);
        assert_eq!(seen[4].1 - seen[3].1, 70);
        for i in 0..BLINKS_IN_LOOP {
            assert!((2500..=6000).contains(&pause_ms(i)));
        }
        // паузы разные
        assert!((1..BLINKS_IN_LOOP).any(|i| pause_ms(i) != pause_ms(0)));
    }

    #[test]
    fn blink_only_awake() {
        for t in (0..60_000).step_by(10) {
            assert_eq!(frame(0, t).blink, 0);
        }
        assert!((0..60_000).step_by(10).any(|t| frame(1, t).blink == 2));
        assert!((0..60_000).step_by(10).any(|t| frame(2, t).blink == 2));
    }

    #[test]
    fn z_only_asleep() {
        for t in (0..20_000).step_by(10) {
            assert_eq!(frame(1, t).z, 0);
            assert_eq!(frame(2, t).z, 0);
        }
        let s: Vec<u8> = [0, 900, 1800, 2700].iter().map(|t| frame(0, *t).z).collect();
        assert_eq!(s, [0, 1, 2, 0]);
    }

    #[test]
    fn loops() {
        assert_eq!(frame(0, 0).ecg, Some(0));
        assert_eq!(frame(0, 119).ecg, Some(0));
        assert_eq!(frame(0, 120).ecg, Some(1));
        assert_eq!(frame(0, 2400).ecg, Some(0));
        let p = |t| frame(0, t).pickle;
        assert_eq!([p(0), p(1299), p(1300), p(2799), p(2800), p(3899), p(3900), p(5499), p(5500)], [0, 0, 1, 1, 0, 0, 2, 2, 0]);
        // период кардиограммы и огурчика
        for t in [0, 500, 1234, 4000] {
            assert_eq!(frame(1, t).pickle, frame(1, t + 5500).pickle);
            assert_eq!(frame(1, t).ecg, frame(1, t + 2400).ecg);
        }
        // круг морганий
        let l = blink_loop_ms();
        for t in [0, 777, 5000, 12_345] {
            assert_eq!(frame(1, t).blink, frame(1, t + l).blink);
        }
    }

    #[test]
    fn next_change_is_a_change() {
        for state in 0..3 {
            for t in (0..30_000).step_by(37) {
                let n = next_change(state, t);
                assert!(n > t && n - t <= ECG_MS, "{state} {t} {n}");
                // до n кадр тот же, в n — уже другой
                assert_eq!(frame(state, n - 1), frame(state, t));
                assert_ne!(frame(state, n), frame(state, t));
            }
        }
    }

    #[test]
    fn patches_inside_box() {
        for p in ECG.iter().chain(&PICKLE).chain(&BLINK_WRITE).chain(&BLINK_DONE).chain(&Z) {
            let (x, y, w, h, _) = *p;
            assert!(x >= 0 && y >= 0 && x + w <= W && y + h <= H);
            let c = decode(p);
            assert!(!c.0.is_empty());
            assert!(c.0.iter().all(|&(cx, cy, _)| cx >= x && cy >= y && cx < x + w && cy < y + h));
        }
    }

    #[test]
    fn compose_without_overlays_is_still() {
        let still: Vec<u32> = (0..(W * H) as u32).collect();
        assert_eq!(compose(1, &still, &Frame::default()), still);
        // с огурчиком — меняются только точки в его рамке
        let f = Frame { pickle: 1, ..Frame::default() };
        let out = compose(0, &still, &f);
        let (bx, by, bw, bh, _) = PICKLE[0];
        for (i, (a, b)) in still.iter().zip(&out).enumerate() {
            let (x, y) = (i as i32 % W, i as i32 / W);
            if a != b {
                assert!(x >= bx && x < bx + bw && y >= by && y < by + bh);
            }
        }
        assert_ne!(out, still);
    }
}
