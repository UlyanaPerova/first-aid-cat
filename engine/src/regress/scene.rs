//! Регрессионные тесты анимации сценки плашки (`scene_anim.rs`, данные — `scene_anim_px.rs`, `scene_px.rs`).
//! Дочерний модуль `scene_anim::regress`.
//!
//! Эталоны: (1) расписания — наивный оракул с константами, выписанными вручную из спецификации (паузы моргания посчитаны
//! Python-ом: splitmix64 от номера; хвост и огурчик — таблицы основного окна `life.rs`); (2) кадры — контрольные суммы,
//! вычисленные Python-скриптом `tools/regress_scene_expected.py` из утверждённых PNG и из текста `scene_px.rs`, разжатого
//! на Python (файл `regress/scene_expected.rs`).
#![allow(clippy::manual_is_multiple_of)] // is_multiple_of стабилен с Rust 1.90, а проект собирается с 1.89

use super::*;
use crate::regress_common::*;
use crate::scene_px::{BG, DONE, H, SLEEP, WRITE};

#[path = "scene_expected.rs"]
mod expected;

// ───────────── расписания ─────────────

/// Паузы между морганиями, мс: 2500 + splitmix64(i) % 3501, i = 0..16 (Python, независимо от Rust-кода).
const PAUSES: [u64; 16] = [4515, 4922, 3593, 4157, 4288, 4586, 5312, 4252, 4731, 3364, 5931, 4866, 3521, 2880, 4275, 2900];
/// Круг морганий: сумма пауз + 16 × (3 × 70 мс) = 71453 (Python).
const BLINK_LOOP: u64 = 71_453;
/// Хвост: (длительность мс, поза) — таблица основного окна (life.rs, TAIL, секунды × 1000).
const TAIL_SPEC: [(u64, u8); 10] = [(3200, 0), (280, 1), (500, 2), (280, 1), (3600, 0), (200, 1), (220, 2), (1000, 3), (260, 2), (240, 1)];

/// Паузы перед дёргом ушка, мс: 5000 + splitmix64(i + 1000) % 7001 для i = 0..15 — посчитаны на Python.
const EAR_PAUSES: [u64; 16] = [9844, 11192, 6614, 8012, 7251, 5929, 9692, 5335, 7561, 6411, 8700, 7780, 10669, 6321, 9045, 6799];
/// Круг ушка: сумма пауз + 16 × (70 + 90 + 80 мс) = 130995 (Python).
const EAR_LOOP: u64 = 130995;

/// Наивный оракул: что включено в миг `t` (мс) — без общего кода с `scene_anim`.
fn oracle(state: u8, t: u64) -> Frame {
    // кардиограмма: 20 кадров по 120 мс
    let ecg = ((t / 120) % 20) as u8;
    // огурчик: 5500-мс круг: [0;1300) покой, [1300;2800) вверх, [2800;3900) покой, [3900;5500) вниз
    let o = t % 5500;
    let pickle = if o < 1300 {
        0
    } else if o < 2800 {
        1
    } else if o < 3900 {
        0
    } else {
        2
    };
    // «z» — только у спящего, три кадра по 900 мс
    let z = if state == 0 { ((t / 900) % 3) as u8 } else { 0 };
    // хвост: длительность каждого шага = d × k / 1000 (целочисленно), k — 1000 / 600 / 1000
    let k = [1000u64, 600, 1000][usize::from(state).min(2)];
    let steps: Vec<(u64, u8)> = TAIL_SPEC.iter().map(|(d, p)| (d * k / 1000, *p)).collect();
    let loop_ms: u64 = steps.iter().map(|s| s.0).sum();
    let mut o = t % loop_ms;
    let mut tail = 0;
    for (d, p) in &steps {
        if o < *d {
            tail = *p;
            break;
        }
        o -= d;
    }
    // моргание: пауза_i, затем веки: 70 мс полуприкрыты (1), 70 закрыты (2), 70 полуприкрыты (1)
    let mut blink = 0;
    if state == 1 || state == 2 {
        let mut o = t % BLINK_LOOP;
        'l: for p in PAUSES {
            if o < p {
                break 'l;
            }
            o -= p;
            if o < 210 {
                blink = [1, 2, 1][(o / 70) as usize];
                break 'l;
            }
            o -= 210;
        }
    }
    // ушко: пауза_i, затем наклон 70 мс (1), сложено 90 мс (2), наклон 80 мс (1) — одинаково во всех состояниях
    let mut ear = 0;
    let mut o = t % EAR_LOOP;
    for p in EAR_PAUSES {
        if o < p {
            break;
        }
        o -= p;
        if o < 240 {
            ear = if (70..160).contains(&o) { 2 } else { 1 };
            break;
        }
        o -= 240;
    }
    Frame { blink, ecg: Some(ecg), pickle, z, tail, ear, shy: false }
}

#[test]
fn schedule_constants_match_independent_values() {
    // длины кругов — по выкладке в комментарии к таблице
    assert_eq!(blink_loop_ms(), BLINK_LOOP);
    for (i, p) in PAUSES.iter().enumerate() {
        assert_eq!(pause_ms(i as u64), *p, "пауза {i}");
    }
    assert_eq!((tail_loop_ms(2), tail_loop_ms(1), tail_loop_ms(0)), (9780, 5868, 9780));
    // пауза укладывается в «от 2,5 до 6 с», включительно
    assert!(PAUSES.iter().all(|p| (2500..=6000).contains(p)));
}

/// Каждую миллисекунду на двух кругах: поля кадра совпадают с оракулом (для всех трёх состояний и состояния «вне диапазона»).
#[test]
fn frame_matches_oracle_every_millisecond() {
    for state in 0..=2u8 {
        for t in 0..(2 * BLINK_LOOP + 5) {
            let (f, o) = (frame(state, t), oracle(state, t));
            assert_eq!(f, o, "state {state} t {t}");
        }
    }
    // состояние за пределами: ведёт себя как «закончил» для хвоста, без морганий и «z»
    for t in (0..60_000).step_by(7) {
        let f = frame(7, t);
        assert_eq!((f.blink, f.z), (0, 0));
        assert_eq!(f.tail, oracle(2, t).tail);
    }
}

/// `next_change` против перебора: список моментов смены кадра (перебор каждой мс) совпадает с обходом по `next_change`.
#[test]
fn next_change_equals_bruteforce_change_points() {
    let end = 2 * BLINK_LOOP;
    for state in 0..=2u8 {
        let mut brute = Vec::new();
        let mut prev = oracle(state, 0);
        for t in 1..end {
            let f = oracle(state, t);
            if f != prev {
                brute.push(t);
                prev = f;
            }
        }
        let mut walk = Vec::new();
        let mut t = 0;
        loop {
            t = next_change(state, t);
            if t >= end {
                break;
            }
            walk.push(t);
        }
        assert_eq!(walk, brute, "state {state}: моменты смены кадра расходятся");
    }
}

/// Метаморфическое: сдвиг на период каждой составляющей не меняет её поле (случайные большие моменты, до 10^12 мс ≈ 31 год).
#[test]
fn frame_periodicity_at_large_times() {
    for_seeds("scene_period", |r| {
        let (state, t) = (r.below(3) as u8, r.below(1_000_000_000_000));
        let f = frame(state, t);
        assert_eq!(frame(state, t + 2400).ecg, f.ecg);
        assert_eq!(frame(state, t + 5500).pickle, f.pickle);
        assert_eq!(frame(state, t + 2700).z, f.z);
        assert_eq!(frame(state, t + tail_loop_ms(state)).tail, f.tail);
        assert_eq!(frame(state, t + BLINK_LOOP).blink, f.blink);
        // следующая смена — строго позже и не дальше 120 мс (кардиограмма меняется чаще всего)
        let n = next_change(state, t);
        assert!(n > t && n - t <= 120, "{state} {t} {n}");
    });
}

/// Границы времени: огромное t не паникует; следующая смена всегда впереди (до 2^62 мс, дальше — вне суток работы плашки).
#[test]
fn schedule_survives_huge_times() {
    for state in 0..3u8 {
        for t in [0u64, 1, u32::MAX as u64, 1 << 40, 1 << 50, 1 << 62] {
            let _ = frame(state, t);
            assert!(next_change(state, t) > t);
        }
    }
}

// ───────────── кадры против эталона из PNG ─────────────

/// Независимый разжиматель (наивный): пары «символ значения, символ длины−1»; всё, что меньше '0' — переводы строк.
fn naive_unrle(s: &str) -> Vec<u8> {
    let b: Vec<u8> = s.bytes().filter(|c| *c >= 48).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        for _ in 0..(b[i + 1] - 48 + 1) {
            out.push(b[i] - 48);
        }
        i += 2;
    }
    out
}

fn still(state: u8) -> Vec<u32> {
    let mut px: Vec<u32> = naive_unrle(BG).iter().map(|&i| PAL[usize::from(i)]).collect();
    let (x0, y0, w, _h, data) = *[&SLEEP, &WRITE, &DONE][usize::from(state)];
    for (i, v) in naive_unrle(data).into_iter().enumerate() {
        if v > 0 {
            px[((y0 + i as i32 / w) * W + x0 + i as i32 % w) as usize] = PAL[usize::from(v) - 1];
        }
    }
    px
}

fn frame_hash(px: &[u32]) -> u64 {
    fnv64(px.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8]))
}

fn case_frame(name: &str) -> (u8, Frame) {
    let p: Vec<&str> = name.split('_').collect();
    let n = |i: usize| p[i].parse::<u8>().unwrap();
    match p[0] {
        "still" => (n(1), Frame::default()),
        "ecg" => (n(1), Frame { ecg: Some(n(2)), ..Frame::default() }),
        "pickle" => (n(1), Frame { pickle: n(2), ..Frame::default() }),
        "tail" => (n(1), Frame { tail: n(2), ..Frame::default() }),
        "blink" => (n(1), Frame { blink: n(2), ..Frame::default() }),
        "z" => (n(1), Frame { z: n(2), ..Frame::default() }),
        "combo" => match p[1] {
            "a" => (1, Frame { tail: 2, ecg: Some(5), pickle: 1, blink: 2, z: 0, ear: 0, shy: false }),
            "b" => (0, Frame { tail: 3, ecg: Some(11), pickle: 2, blink: 0, z: 2, ear: 0, shy: false }),
            "c" => (2, Frame { tail: 1, ecg: Some(0), pickle: 1, blink: 1, z: 0, ear: 0, shy: false }),
            "d" => (1, Frame { tail: 3, ecg: Some(19), pickle: 2, blink: 1, z: 0, ear: 0, shy: false }),
            x => panic!("неизвестный состав {x}"),
        },
        x => panic!("неизвестный кадр {x}"),
    }
}

/// Каждый кадр анимации (88 штук: три статики, 60 кардиограммы, огурчик, моргание, «z», хвост, 4 составных) даёт ту же
/// контрольную сумму, что независимо собранная из утверждённых PNG картинка.
#[test]
fn composed_frames_match_approved_png_checksums() {
    let stills = [still(0), still(1), still(2)];
    let mut bad = Vec::new();
    for (name, want) in expected::CASES {
        let (st, f) = case_frame(name);
        let got = frame_hash(&compose(st, &stills[usize::from(st)], &f));
        if got != want {
            bad.push(format!("{name}: {got:016x} вместо {want:016x}"));
        }
    }
    assert!(bad.is_empty(), "кадры не совпали с эталоном:\n{}", bad.join("\n"));
    assert_eq!(expected::CASES.len(), 88);
}

// ───────────── свойства compose ─────────────

#[test]
fn compose_changes_only_inside_patch_boxes_and_is_idempotent() {
    for_seeds("compose", |r| {
        let state = r.below(3) as u8;
        let s = still(state);
        let f = Frame {
            blink: if state > 0 { r.below(3) as u8 } else { 0 },
            ecg: if r.chance(70) { Some(r.below(20) as u8) } else { None },
            pickle: r.below(3) as u8,
            z: if state == 0 { r.below(3) as u8 } else { 0 },
            tail: r.below(4) as u8,
            ear: r.below(3) as u8,
            shy: false,
        };
        let out = compose(state, &s, &f);
        assert_eq!(out.len(), s.len());
        // ни одна точка вне объединения рамок включённых заплаток не изменилась
        let mut boxes: Vec<(i32, i32, i32, i32)> = Vec::new();
        if f.tail > 0 {
            let b = TAIL[usize::from(state)][usize::from(f.tail) - 1];
            boxes.push((b.0, b.1, b.2, b.3));
        }
        if f.ear > 0 {
            let b = crate::scene_anim_px::EAR[usize::from(state)][usize::from(f.ear) - 1];
            boxes.push((b.0, b.1, b.2, b.3));
        }
        if let Some(e) = f.ecg {
            let b = ECG[usize::from(e)];
            boxes.push((b.0, b.1, b.2, b.3));
        }
        if f.pickle > 0 {
            let b = PICKLE[usize::from(f.pickle) - 1];
            boxes.push((b.0, b.1, b.2, b.3));
        }
        if f.z > 0 && state == 0 {
            let b = Z[usize::from(f.z) - 1];
            boxes.push((b.0, b.1, b.2, b.3));
        }
        if f.blink > 0 && state > 0 {
            let b = if state == 1 { BLINK_WRITE[usize::from(f.blink) - 1] } else { BLINK_DONE[usize::from(f.blink) - 1] };
            boxes.push((b.0, b.1, b.2, b.3));
        }
        for (i, (a, b)) in s.iter().zip(&out).enumerate() {
            if a != b {
                let (x, y) = (i as i32 % W, i as i32 / W);
                assert!(
                    boxes.iter().any(|&(bx, by, bw, bh)| x >= bx && x < bx + bw && y >= by && y < by + bh),
                    "точка ({x},{y}) изменена вне заплаток"
                );
            }
        }
        // наложение тех же заплаток поверх результата ничего не меняет (идемпотентность)
        assert_eq!(compose(state, &out, &f), out);
        // кадр без анимации — сама картинка
        assert_eq!(compose(state, &s, &Frame::default()), s);
    });
}

/// Короткая или пустая картинка-основа и «чужие» значения кадра: без паники, без расползания по памяти.
#[test]
fn compose_tolerates_bad_inputs() {
    let f = Frame { blink: 2, ecg: Some(255), pickle: 2, z: 2, tail: 3, ear: 2, shy: true };
    for state in [0u8, 1, 2, 9] {
        let _ = compose(state, &[], &f);
        let short = vec![0u32; 1000];
        let out = compose(state, &short, &f);
        assert_eq!(out.len(), 1000);
    }
    // номера вне диапазона просто пропускаются
    let s = still(1);
    assert_eq!(compose(1, &s, &Frame { blink: 9, ecg: Some(200), pickle: 9, z: 9, tail: 9, ear: 9, shy: false }), s);
}

// ───────────── разжимание и целостность данных ─────────────

/// Случайные строки RLE (включая нечётную длину, переводы строк, значения за палитрой и «длиннее рамки»): заплатка разжимается
/// так же, как наивным оракулом (клетки внутри рамки, цвет по палитре, лишнее отбрасывается).
#[test]
fn decode_matches_naive_oracle_on_random_rle() {
    for_seeds("decode", |r| {
        let (w, h) = (r.range(1, 12) as i32, r.range(1, 12) as i32);
        let (x0, y0) = (r.range(0, 50) as i32, r.range(0, 50) as i32);
        let mut s = String::from("\n");
        let pairs = r.range(0, 60);
        for _ in 0..pairs {
            // значение 0..=78 (за палитрой — 74.. — отбрасывается), длина 1..=71
            let v = r.range(0, 78) as u8;
            let n = r.range(1, 71) as u8;
            s.push((48 + v) as char);
            s.push((48 + n - 1) as char);
            if r.chance(10) {
                s.push('\n');
            }
        }
        if r.chance(20) {
            s.push('0'); // нечётный хвост
        }
        let leaked: &'static str = Box::leak(s.clone().into_boxed_str());
        let patch: Patch = (x0, y0, w, h, leaked);
        let got = decode(&patch).0;
        // оракул
        let flat = naive_unrle(&s);
        let mut want = Vec::new();
        for (i, v) in flat.iter().enumerate() {
            if *v > 0 && (i as i32) < w * h {
                if let Some(rgb) = PAL.get(usize::from(*v) - 1) {
                    want.push((x0 + i as i32 % w, y0 + i as i32 / w, *rgb));
                }
            }
        }
        assert_eq!(got, want, "{s:?}");
    });
}

/// Целостность вшитых данных: фон — ровно W×H клеток; каждый слой кота и каждая заплатка — ровно ширина×высота; индексы
/// палитры допустимы; рамка заплатки плотная (на каждой её стороне есть цветная клетка); ничего не выходит за рамку сцены.
#[test]
fn embedded_scene_data_is_consistent() {
    assert_eq!(naive_unrle(BG).len(), (W * H) as usize, "фон не W×H");
    assert!(naive_unrle(BG).iter().all(|v| usize::from(*v) < PAL.len()), "индекс фона вне палитры");
    for (name, l) in [("SLEEP", &SLEEP), ("WRITE", &WRITE), ("DONE", &DONE)] {
        let (x, y, w, h, d) = *l;
        let v = naive_unrle(d);
        assert_eq!(v.len(), (w * h) as usize, "{name}: клеток {} вместо {}", v.len(), w * h);
        assert!(v.iter().all(|c| usize::from(*c) <= PAL.len()), "{name}: индекс вне палитры");
        assert!(x >= 0 && y >= 0 && x + w <= W && y + h <= H, "{name}: слой вне сцены");
    }
    let mut all: Vec<(String, Patch)> = Vec::new();
    for (i, p) in ECG.iter().enumerate() {
        all.push((format!("ECG[{i}]"), *p));
    }
    for (i, p) in PICKLE.iter().enumerate() {
        all.push((format!("PICKLE[{i}]"), *p));
    }
    for (i, p) in BLINK_WRITE.iter().enumerate() {
        all.push((format!("BLINK_WRITE[{i}]"), *p));
    }
    for (i, p) in BLINK_DONE.iter().enumerate() {
        all.push((format!("BLINK_DONE[{i}]"), *p));
    }
    for (i, p) in Z.iter().enumerate() {
        all.push((format!("Z[{i}]"), *p));
    }
    for (s, row) in TAIL.iter().enumerate() {
        for (i, p) in row.iter().enumerate() {
            all.push((format!("TAIL[{s}][{i}]"), *p));
        }
    }
    assert_eq!(all.len(), 20 + 2 + 2 + 2 + 2 + 9);
    for (name, (x, y, w, h, s)) in all {
        let v = naive_unrle(s);
        assert_eq!(v.len(), (w * h) as usize, "{name}: клеток {} вместо {}", v.len(), w * h);
        assert!(s.bytes().filter(|c| *c >= 48).count() % 2 == 0, "{name}: нечётное число знаков RLE");
        assert!(v.iter().all(|c| usize::from(*c) <= PAL.len()), "{name}: индекс палитры вне диапазона");
        assert!(x >= 0 && y >= 0 && w > 0 && h > 0 && x + w <= W && y + h <= H, "{name}: рамка вне сцены");
        let at = |cx: i32, cy: i32| v[(cy * w + cx) as usize] > 0;
        assert!((0..w).any(|cx| at(cx, 0)) && (0..w).any(|cx| at(cx, h - 1)), "{name}: верхняя или нижняя строка рамки пуста");
        assert!((0..h).any(|cy| at(0, cy)) && (0..h).any(|cy| at(w - 1, cy)), "{name}: левый или правый столбец рамки пуст");
    }
}

/// Длительное испытание: кадры плашки идут «как в работе» (смена кадра по `next_change`), на каждом проверяется
/// совпадение с оракулом и ограниченность времени сборки; RSS/потоки/дескрипторы пишутся в CSV.
#[test]
#[ignore = "длительное испытание: см. REGRESSION_TESTS.md"]
fn soak_scene_frames() {
    let secs = soak_secs(30);
    csv_line("t_s,frames,rss_kb,threads,fds,worst_compose_us,ok");
    let stills = [still(0), still(1), still(2)];
    let t0 = std::time::Instant::now();
    let (mut t, mut frames, mut worst) = (0u64, 0u64, 0u128);
    let mut next_report = 0;
    let mut state = 0u8;
    while t0.elapsed().as_secs() < secs {
        let f = frame(state, t);
        assert_eq!(f, oracle(state, t), "state {state} t {t}");
        let s = std::time::Instant::now();
        let px = compose(state, &stills[usize::from(state)], &f);
        worst = worst.max(s.elapsed().as_micros());
        assert_eq!(px.len(), (W * H) as usize);
        frames += 1;
        t = next_change(state, t);
        if frames % 50_000 == 0 {
            state = (state + 1) % 3; // смена состояния агента, как при перерисовке плашки
        }
        if t0.elapsed().as_secs() >= next_report {
            let (rss, th, fd) = proc_stats();
            csv_line(&format!("{},{frames},{rss},{th},{fd},{worst},1", t0.elapsed().as_secs()));
            next_report += 5;
        }
    }
}
