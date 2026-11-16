// Анимация сценки плашки: что в какой миг включено (моргание, кардиограмма, огурчик, «z», хвост, ушки, письмо ручкой) и сборка картинки.
// Данные кадров — scene_anim_px.rs, фон и коты — scene_px.rs. Время — целые миллисекунды от открытия плашки.

use crate::scene_anim_px::{Patch, BLINK_DONE, BLINK_WRITE, EAR, EAR_AT, EAR_R, EAR_R_AT, ECG, PEN, PEN_TL, PEN_UNDER, PICKLE, SHY, TAIL, Z};
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
    /// хвост: 0 покой, 1–3 — всё дальше вправо
    pub tail: u8,
    /// левое (на экране) ушко: 0 покой, 1 наклон, 2 сложено
    pub ear: u8,
    /// правое ушко — так же
    pub ear_r: u8,
    /// кисть с ручкой у «пишет»: 0 покой, 1–6 — положения
    pub pen: u8,
    /// смущён (после нажатия на ушко): румянец, у бодрствующего — зажмуренные глаза
    pub shy: bool,
}

const ECG_MS: u64 = 120;
const BLINK_STEP_MS: u64 = 70;
/// веки: полуприкрыты, закрыты, полуприкрыты — по шагу каждое
const BLINK_MS: u64 = 3 * BLINK_STEP_MS;
/// огурчик: (конец отрезка в мс от начала цикла, поза)
const PICKLE_TL: [(u64, u8); 4] = [(1300, 0), (2800, 1), (3900, 0), (5500, 2)];
const Z_MS: u64 = 900;
/// хвост, как в основном окне (life.rs, TAIL): (длительность в мс, поза) — два взмаха, второй шире и с задержкой
const TAIL_TL: [(u64, u8); 10] = [(3200, 0), (280, 1), (500, 2), (280, 1), (3600, 0), (200, 1), (220, 2), (1000, 3), (260, 2), (240, 1)];
/// скорость хвоста по состояниям (спит, пишет, закончил), тысячные доли длительности: спящий и закончивший машут
/// как кот в основном окне, пишущий — чаще (спящий раньше махал вдвое реже, владелица попросила чаще)
const TAIL_SPEED: [u64; 3] = [1000, 600, 1000];
/// пауз между морганиями хватает на цикл из стольких морганий, дальше они повторяются
const BLINKS_IN_LOOP: u64 = 16;

/// ушко дёргается само: наклон, сложено, наклон — (длительность в мс, поза)
const EAR_TL: [(u64, u8); 3] = [(70, 1), (90, 2), (80, 1)];
const EAR_MS: u64 = 240;
/// от нажатия ушко держится сложенным дольше: наклон, сложено почти полсекунды, наклон
const POKE_TL: [(u64, u8); 3] = [(70, 1), (490, 2), (80, 1)];
/// смущение от нажатия: появляется, когда ушко сложилось, и держится ещё полторы секунды после его возврата
const SHY_FROM: u64 = 70;
const SHY_TO: u64 = 640 + 1500;
const EARS_IN_LOOP: u64 = 16;

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

/// Пауза перед дёргом ушка номер i: от 5 до 12 с (как в основном окне); свои числа, не те, что у моргания.
fn ear_pause_ms(i: u64) -> u64 {
    5000 + hash(i + 1000) % 7001
}

fn ear_loop_ms() -> u64 {
    (0..EARS_IN_LOOP).map(|i| ear_pause_ms(i) + EAR_MS).sum()
}

/// Поза по расписанию шагов с начала движения: поза и конец шага; None — движение кончилось.
fn step_of(tl: &[(u64, u8)], o: u64) -> Option<(u8, u64)> {
    let mut end = 0;
    for &(d, pose) in tl {
        end += d;
        if o < end {
            return Some((pose, end));
        }
    }
    None
}

/// Нажатие на ушко: миг (мс от открытия плашки) и правое ли это ушко.
pub type Poke = Option<(u64, bool)>;

/// Ушко само по себе в миг `t`: поза, когда она кончится (мс от начала круга) и правое ли ушко дёргается в этот
/// раз (случайно, как в основном окне).
fn ear_at(t: u64) -> (u8, u64, bool) {
    let mut o = t % ear_loop_ms();
    let mut start = 0;
    for i in 0..EARS_IN_LOOP {
        let p = ear_pause_ms(i);
        if o < p {
            return (0, start + p, false);
        }
        o -= p;
        start += p;
        if let Some((pose, end)) = step_of(&EAR_TL, o) {
            return (pose, start + end, hash(i + 2000) & 1 == 1);
        }
        o -= EAR_MS;
        start += EAR_MS;
    }
    (0, start, false)
}

/// Письмо ручкой в миг `t`: положение кисти и когда оно кончится (мс от начала круга).
fn pen_at(t: u64) -> (u8, u64) {
    let o = t % pen_loop_ms();
    step_of(&PEN_TL, o).unwrap_or((0, pen_loop_ms()))
}

fn pen_loop_ms() -> u64 {
    PEN_TL.iter().map(|s| s.0).sum()
}

/// Попало ли нажатие в ушко (`x`, `y` — клетка сценки): Some(false) — левое, Some(true) — правое.
pub fn ear_hit(state: u8, x: i32, y: i32) -> Option<bool> {
    let i = usize::from(state).min(2);
    let inside = |(x0, y0, x1, y1): (i32, i32, i32, i32)| x >= x0 && x < x1 && y >= y0 && y < y1;
    if inside(EAR_AT[i]) {
        Some(false)
    } else {
        inside(EAR_R_AT[i]).then_some(true)
    }
}

/// Кадр с учётом нажатия на ушко в миг `poke` (мс от открытия плашки): пока идёт движение от нажатия, оно важнее
/// собственного дёрга.
pub fn frame_poked(state: u8, t: u64, poke: Poke) -> Frame {
    let mut f = frame(state, t);
    if let Some((p, right)) = poke {
        if let Some((pose, _)) = t.checked_sub(p).and_then(|o| step_of(&POKE_TL, o)) {
            // тронутое ушко складывается, другое в это время спокойно
            (f.ear, f.ear_r) = if right { (0, pose) } else { (pose, 0) };
        }
        f.shy = t >= p + SHY_FROM && t < p + SHY_TO;
        // пока кот смущён, он не моргает: лицо смущения нарисовано с открытыми глазами и перекрывает часть клеток век —
        // моргание под ним выходило обрывками (полглаза закрыто, полглаза открыто)
        if f.shy {
            f.blink = 0;
        }
    }
    f
}

/// Когда кадр сменится с учётом нажатия на ушко — всегда позже `t`.
pub fn next_change_poked(state: u8, t: u64, poke: Poke) -> u64 {
    let n = next_change(state, t);
    match poke.map(|p| p.0) {
        Some(p) if t < p => n.min(p),
        Some(p) if t < p + SHY_TO => step_of(&POKE_TL, t - p).map_or(n.min(p + SHY_TO), |(_, end)| n.min(p + end)),
        Some(_) => n,
        None => n,
    }
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

/// Хвост в миг `t`: поза и когда она кончится (мс от начала круга).
fn tail_at(state: u8, t: u64) -> (u8, u64) {
    let k = TAIL_SPEED[usize::from(state).min(2)];
    let o = t % tail_loop_ms(state);
    let mut end = 0;
    for (d, pose) in TAIL_TL {
        end += d * k / 1000;
        if o < end {
            return (pose, end);
        }
    }
    (0, end)
}

fn tail_loop_ms(state: u8) -> u64 {
    TAIL_TL.iter().map(|(d, _)| d * TAIL_SPEED[usize::from(state).min(2)] / 1000).sum()
}

/// Что включено в миг `t` мс для состояния `state` (0 спит, 1 пишет, 2 закончил).
pub fn frame(state: u8, t: u64) -> Frame {
    Frame {
        blink: if blinks(state) { blink_at(t).0 } else { 0 },
        ecg: Some((t / ECG_MS % ECG.len() as u64) as u8),
        pickle: pickle_at(t).0,
        z: if state == 0 { (t / Z_MS % 3) as u8 } else { 0 },
        tail: tail_at(state, t).0,
        ear: if ear_at(t).2 { 0 } else { ear_at(t).0 },
        ear_r: if ear_at(t).2 { ear_at(t).0 } else { 0 },
        shy: false,
        pen: if state == 1 { pen_at(t).0 } else { 0 },
    }
}

/// Когда (мс) кадр сменится в следующий раз — всегда позже `t`.
pub fn next_change(state: u8, t: u64) -> u64 {
    let mut n = (t / ECG_MS + 1) * ECG_MS;
    let lap = |len: u64, end_in_lap: u64| t / len * len + end_in_lap;
    n = n.min(lap(PICKLE_TL[3].0, pickle_at(t).1));
    n = n.min(lap(tail_loop_ms(state), tail_at(state, t).1));
    n = n.min(lap(ear_loop_ms(), ear_at(t).1));
    if state == 1 {
        n = n.min(lap(pen_loop_ms(), pen_at(t).1));
    }
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
    tail: Vec<Vec<Cells>>,
    ear: Vec<Vec<Cells>>,
    ear_r: Vec<Vec<Cells>>,
    pen: Vec<Cells>,
    pen_under: Vec<Cells>,
    shy: Vec<Option<Cells>>,
}

fn decoded() -> &'static Decoded {
    static D: std::sync::OnceLock<Decoded> = std::sync::OnceLock::new();
    D.get_or_init(|| Decoded {
        ecg: ECG.iter().map(decode).collect(),
        pickle: PICKLE.iter().map(decode).collect(),
        blink_write: BLINK_WRITE.iter().map(decode).collect(),
        blink_done: BLINK_DONE.iter().map(decode).collect(),
        z: Z.iter().map(decode).collect(),
        tail: TAIL.iter().map(|s| s.iter().map(decode).collect()).collect(),
        ear: EAR.iter().map(|s| s.iter().map(decode).collect()).collect(),
        ear_r: EAR_R.iter().map(|s| s.iter().map(decode).collect()).collect(),
        pen: PEN.iter().map(decode).collect(),
        pen_under: PEN_UNDER.iter().map(decode).collect(),
        shy: SHY.iter().map(|p| p.as_ref().map(decode)).collect(),
    })
}

/// Заплатки кадра `f` для состояния `state`, по порядку наложения.
fn overlays(state: u8, f: &Frame) -> Vec<&'static Cells> {
    let d = decoded();
    let mut v = Vec::new();
    if f.tail > 0 {
        v.extend(d.tail.get(usize::from(state).min(2)).and_then(|s| s.get(usize::from(f.tail) - 1)));
    }
    if f.ear > 0 {
        v.extend(d.ear.get(usize::from(state).min(2)).and_then(|s| s.get(usize::from(f.ear) - 1)));
    }
    if f.ear_r > 0 {
        v.extend(d.ear_r.get(usize::from(state).min(2)).and_then(|s| s.get(usize::from(f.ear_r) - 1)));
    }
    if let Some(e) = f.ecg {
        v.extend(d.ecg.get(usize::from(e)));
    }
    if f.pickle > 0 {
        v.extend(d.pickle.get(usize::from(f.pickle) - 1));
    }
    if f.z > 0 && state == 0 {
        v.extend(d.z.get(usize::from(f.z) - 1));
    }
    // то, что открылось из-под сдвинутой кисти, — до моргания и смущения: веки и румянец ложатся поверх лица
    if f.pen > 0 && state == 1 {
        v.extend(d.pen_under.get(usize::from(f.pen) - 1));
    }
    if f.blink > 0 {
        let b = if state == 1 { &d.blink_write } else { &d.blink_done };
        if blinks(state) {
            v.extend(b.get(usize::from(f.blink) - 1));
        }
    }
    // смущение — поверх моргания: глаза остаются зажмуренными
    if f.shy {
        v.extend(d.shy.get(usize::from(state).min(2)).and_then(Option::as_ref));
    }
    // кисть с ручкой — поверх лица: она ближе к зрителю
    if f.pen > 0 && state == 1 {
        v.extend(d.pen.get(usize::from(f.pen) - 1));
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
    fn tail_sways_at_state_speed() {
        // «закончил» — как в основном окне: покой 3,2 с, затем позы 1, 2, 1, покой, 1, 2, 3, 2, 1
        let p = |s, t| frame(s, t).tail;
        assert_eq!([p(2, 0), p(2, 3199), p(2, 3200), p(2, 3480), p(2, 3980), p(2, 4260), p(2, 8500)], [0, 0, 1, 2, 1, 0, 3]);
        assert_eq!(tail_loop_ms(2), 9780);
        // пишущий машет чаще, спящий — как в основном окне; порядок поз тот же
        assert_eq!((tail_loop_ms(1), tail_loop_ms(0)), (5868, 9780));
        assert_eq!([p(1, 1919), p(1, 1920), p(0, 3199), p(0, 3200)], [0, 1, 0, 1]);
        for s in 0..3 {
            let mut seq = Vec::new();
            let mut t = 0;
            while t < tail_loop_ms(s) {
                if seq.last() != Some(&p(s, t)) {
                    seq.push(p(s, t));
                }
                t = next_change(s, t);
            }
            assert_eq!(seq, [0, 1, 2, 1, 0, 1, 2, 3, 2, 1]);
            assert_eq!(p(s, tail_loop_ms(s)), 0);
        }
    }

    #[test]
    fn tail_patch_stays_in_tail_area() {
        let still: Vec<u32> = vec![0x0100_0000; (W * H) as usize];
        for s in 0..3u8 {
            for pose in 1..=3u8 {
                let out = compose(s, &still, &Frame { tail: pose, ..Frame::default() });
                let (bx, by, bw, bh, _) = TAIL[usize::from(s)][usize::from(pose) - 1];
                let changed = still.iter().zip(&out).enumerate().filter(|(_, (a, b))| a != b);
                let mut n = 0;
                for (i, _) in changed {
                    let (x, y) = (i as i32 % W, i as i32 / W);
                    assert!(x >= bx && x < bx + bw && y >= by && y < by + bh);
                    n += 1;
                }
                assert!(n > 300, "{s} {pose} {n}");
            }
        }
    }

    #[test]
    fn ears_twitch_by_themselves_and_on_poke() {
        // само: пауза 5–12 с, затем наклон 70 мс, сложено 90 мс, наклон 80 мс; дёргается одно ушко — левое или правое
        for i in 0..EARS_IN_LOOP {
            assert!((5000..=12_000).contains(&ear_pause_ms(i)));
        }
        // первый дёрг — правым ушком, второй — левым (стороны посчитаны отдельно на Python: 1, 0, 0, 1, …)
        let p0 = ear_pause_ms(0);
        let e = |t| (frame(2, t).ear, frame(2, t).ear_r);
        assert_eq!([e(0), e(p0 - 1), e(p0), e(p0 + 70), e(p0 + 160), e(p0 + 240)], [(0, 0), (0, 0), (0, 1), (0, 2), (0, 1), (0, 0)]);
        let p1 = p0 + 240 + ear_pause_ms(1);
        assert_eq!([e(p1 - 1), e(p1), e(p1 + 70), e(p1 + 160), e(p1 + 240)], [(0, 0), (1, 0), (2, 0), (1, 0), (0, 0)]);
        assert_eq!(e(p0 + 100 + ear_loop_ms()), (0, 2));
        // одинаково во всех состояниях; за круг дёргаются оба ушка
        assert_eq!((frame(0, p0 + 100).ear_r, frame(1, p0 + 100).ear_r), (2, 2));
        // нажатие на левое в миг 1000: наклон до 1070, сложено до 1560, наклон до 1640, потом как без нажатия
        let k = |t| frame_poked(1, t, Some((1000, false))).ear;
        assert_eq!([k(999), k(1000), k(1069), k(1070), k(1559), k(1560), k(1639), k(1640)], [0, 1, 1, 2, 2, 1, 1, 0]);
        // нажатие на правое — складывается правое, левое спокойно
        let r = frame_poked(1, 1100, Some((1000, true)));
        assert_eq!((r.ear, r.ear_r), (0, 2));
        // смущение: с мига, когда ушко сложилось (1070), и ещё 1,5 с после его возврата (до 3140)
        let s = |t| frame_poked(2, t, Some((1000, true))).shy;
        assert_eq!([s(1069), s(1070), s(1640), s(3139), s(3140)], [false, true, true, true, false]);
        assert!(!frame(2, 1500).shy && !frame_poked(2, 1500, None).shy);
        // смущённый кот не моргает: ни в один миг смущения веки не опущены, а без нажатия в те же миги моргание есть
        let first_blink = (5000..25_000).find(|t| frame(1, *t).blink == 2).expect("моргание за 20 с");
        for dt in 0..2000 {
            let f = frame_poked(1, first_blink - 1000 + dt, Some((first_blink - 1100, false)));
            assert!(!(f.shy && f.blink != 0), "{dt}");
        }
        assert_eq!(frame_poked(1, first_blink, Some((first_blink - 1100, false))).blink, 0);
        assert_eq!(frame_poked(1, first_blink, Some((first_blink - 5000, false))).blink, 2);
        assert_eq!(next_change_poked(2, 3100, Some((1000, false))).min(3140), next_change(2, 3100).min(3140));
        assert_eq!(frame_poked(1, 3140, Some((1000, false))), frame(1, 3140));
        // смущённое лицо есть у всех трёх состояний
        let still: Vec<u32> = vec![0x0100_0000; (W * crate::scene_px::H) as usize];
        let shy = Frame { shy: true, ..Frame::default() };
        for state in 0..3 {
            assert_ne!(compose(state, &still, &shy), still);
        }
        assert_eq!(frame_poked(1, 500, None), frame(1, 500));
        // будильник: до нажатия — не позже его мига, во время движения — на границах шагов
        assert_eq!(next_change_poked(2, 990, Some((1000, false))), 1000);
        assert_eq!(next_change_poked(2, 1001, Some((1000, false))), 1070);
        assert_eq!(next_change_poked(2, 1500, Some((1000, true))), 1560);
        assert_eq!(next_change_poked(2, 2000, Some((1000, false))).min(3140), next_change(2, 2000).min(3140));
        // области ушек: границы из заготовок (x0, y0 входят, x1, y1 — нет); мимо — None
        assert_eq!([ear_hit(1, 182, 15), ear_hit(1, 209, 35), ear_hit(1, 210, 20), ear_hit(1, 181, 20)], [Some(false), Some(false), None, None]);
        assert_eq!([ear_hit(0, 184, 34), ear_hit(0, 183, 34), ear_hit(2, 190, 16), ear_hit(2, 190, 29)], [Some(false), None, Some(false), None]);
        assert_eq!([ear_hit(1, 261, 36), ear_hit(1, 290, 65), ear_hit(1, 291, 40), ear_hit(1, 270, 66)], [Some(true), Some(true), None, None]);
        assert_eq!([ear_hit(0, 254, 50), ear_hit(0, 253, 50), ear_hit(2, 287, 64), ear_hit(2, 288, 64)], [Some(true), None, Some(true), None]);
    }

    #[test]
    fn pen_writes_only_in_writing_state() {
        // кисть ходит по прямой вправо-влево: покой 1100 мс, влево на клетку (поза 4) 230, на две (поза 3) 700, обратно 230,
        // покой 250, вправо на клетку (поза 5) 1000 — и снова покой; круг 3510 мс
        assert_eq!(pen_loop_ms(), 1100 + 230 + 700 + 230 + 250 + 1000);
        let p = |t| frame(1, t).pen;
        assert_eq!([p(0), p(1099), p(1100), p(1330), p(2030), p(2260), p(2510), p(3509), p(3510)], [0, 0, 4, 3, 4, 0, 5, 5, 0]);
        for t in (0..10_000).step_by(7) {
            assert_eq!((frame(0, t).pen, frame(2, t).pen), (0, 0));
        }
        // кисть рисуется только у «пишет» и меняет точки только в своей рамке
        let still: Vec<u32> = vec![0x0100_0000; (W * crate::scene_px::H) as usize];
        let f = Frame { pen: 3, ..Frame::default() };
        assert_eq!(compose(0, &still, &f), still);
        assert_eq!(compose(2, &still, &f), still);
        let out = compose(1, &still, &f);
        let (ax, ay, aw, ah, _) = PEN[2];
        let (ux, uy, uw, uh, _) = PEN_UNDER[2];
        let (bx, by) = (ax.min(ux), ay.min(uy));
        let (bw, bh) = ((ax + aw).max(ux + uw) - bx, (ay + ah).max(uy + uh) - by);
        let changed: Vec<usize> = still.iter().zip(&out).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
        assert!(changed.len() > 100);
        assert!(changed.iter().all(|i| (*i as i32 % W) >= bx && (*i as i32 % W) < bx + bw && (*i as i32 / W) >= by && (*i as i32 / W) < by + bh));
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
        for p in ECG
            .iter()
            .chain(&PICKLE)
            .chain(&BLINK_WRITE)
            .chain(&BLINK_DONE)
            .chain(&Z)
            .chain(TAIL.iter().flatten())
            .chain(EAR.iter().flatten())
            .chain(EAR_R.iter().flatten())
            .chain(&PEN)
            .chain(&PEN_UNDER)
            .chain(SHY.iter().flatten())
        {
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

/// Независимые регрессионные тесты (REGRESSION_TESTS.md).
#[cfg(test)]
#[path = "regress/scene.rs"]
mod regress;
