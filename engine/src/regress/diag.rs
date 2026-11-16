//! Регрессионные тесты замера скорости и задержки сети (`diag.rs`): джиттер, медиана, перевод единиц, правило
//! «скорость устоялась», повтор при HTTP 429, отказы и обрывы сервера. Всё против ЛОКАЛЬНЫХ фальшивых серверов
//! (в интернет не ходят; исключение — один тест с пометкой ignore, он сравнивает с системным `ping`).
#![allow(clippy::manual_is_multiple_of)] // is_multiple_of стабилен с Rust 1.90, а проект собирается с 1.89

use super::tests::SPEED_TEST;
use super::*;
use crate::regress_common::*;
use std::io::Write;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

fn with_base<R>(base: String, f: impl FnOnce() -> R) -> R {
    let _g = SPEED_TEST.lock().unwrap_or_else(|e| e.into_inner());
    *TEST_BASE.lock().unwrap() = Some(base);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    *TEST_BASE.lock().unwrap() = None;
    match r {
        Ok(r) => r,
        Err(e) => std::panic::resume_unwind(e),
    }
}

// ───────────── джиттер ─────────────

/// Эталон, выложенный вручную. Ряд [100,102,99,3000,101]: расхождения соседних |100-102|=2, |102-99|=3,
/// |99-3000|=2901, |3000-101|=2899 → по возрастанию 2,3,2899,2901; их 4, берётся нижняя из двух средних
/// (индекс (4-1)/2=1) → 3.
#[test]
fn jitter_hand_computed() {
    assert_eq!(jitter(&[100.0, 102.0, 99.0, 3000.0, 101.0]), 3.0);
    // ровный ряд — 0
    assert_eq!(jitter(&[50.0; 8]), 0.0);
    // два замера — модуль разности
    assert_eq!(jitter(&[10.0, 25.5]), 15.5);
    assert_eq!(jitter(&[25.5, 10.0]), 15.5);
    // пилообразный: расхождения все 300 → 300
    assert_eq!(jitter(&[100.0, 400.0, 100.0, 400.0, 100.0]), 300.0);
    // пустой и одиночный
    assert_eq!(jitter(&[]), 0.0);
    assert_eq!(jitter(&[7.0]), 0.0);
    // нечётное число расхождений: [1,2,4,8] → 1,2,4 → медиана 2
    assert_eq!(jitter(&[1.0, 2.0, 4.0, 8.0]), 2.0);
    // 8 замеров по 20 мс шагом вверх → 7 расхождений по 20
    assert_eq!(jitter(&[20.0, 40.0, 60.0, 80.0, 100.0, 120.0, 140.0, 160.0]), 20.0);
}

/// Наивная независимая реализация: пузырьковая сортировка и выбор k-го.
fn naive_jitter(t: &[f64]) -> f64 {
    let mut d: Vec<f64> = Vec::new();
    for i in 1..t.len() {
        d.push(if t[i] > t[i - 1] { t[i] - t[i - 1] } else { t[i - 1] - t[i] });
    }
    if d.is_empty() {
        return 0.0;
    }
    for i in 0..d.len() {
        for j in 0..d.len() - 1 - i {
            if d[j] > d[j + 1] {
                d.swap(j, j + 1);
            }
        }
    }
    d[(d.len() - 1) / 2]
}

#[test]
fn jitter_properties() {
    for_seeds("jitter", |r| {
        let n = r.range(0, 30) as usize;
        let t: Vec<f64> = (0..n).map(|_| r.unit() * 1500.0).collect();
        let j = jitter(&t);
        assert!(j.is_finite() && j >= 0.0);
        assert_eq!(j, naive_jitter(&t), "{t:?}");
        if n >= 2 {
            let (mn, mx) = t.iter().fold((f64::MAX, f64::MIN), |a, v| (a.0.min(*v), a.1.max(*v)));
            assert!(j <= mx - mn + 1e-9, "джиттер больше размаха");
            // сдвиг всех замеров на константу не меняет (метаморфическое); допуск — на округление float
            let c = r.unit() * 500.0;
            let shifted: Vec<f64> = t.iter().map(|v| v + c).collect();
            assert!((jitter(&shifted) - j).abs() < 1e-9);
            // масштаб ×k даёт ×k
            let k = 1.0 + r.unit() * 9.0;
            let scaled: Vec<f64> = t.iter().map(|v| v * k).collect();
            assert!((jitter(&scaled) - j * k).abs() < 1e-7 * (1.0 + j * k));
            // порядок «задом наперёд» — те же расхождения → то же значение
            let mut rev = t.clone();
            rev.reverse();
            assert_eq!(jitter(&rev), j);
        }
        // один выброс в ряду из ≥5 замеров не поднимает джиттер выше самого большого прежнего расхождения
        if n >= 5 {
            let max_d = t.windows(2).map(|w| (w[0] - w[1]).abs()).fold(0.0, f64::max);
            let mut o = t.clone();
            let i = r.below(n as u64) as usize;
            o[i] = 1e9;
            assert!(jitter(&o) <= max_d + 1e-9, "один выброс сдвинул джиттер: {} > {max_d}", jitter(&o));
        }
    });
}

// ───────────── единицы и скорость ─────────────

/// Эталон по определению: Мбит/с = байты × 8 / секунды / 10^6 (десятичные мега, не MiB).
#[test]
fn mbps_units() {
    assert_eq!(mbps_of(12_500_000, 1.0), 100.0);
    assert_eq!(mbps_of(1_000_000, 1.0), 8.0);
    assert_eq!(mbps_of(125_000, 0.01), 100.0);
    assert_eq!(mbps_of(0, 5.0), 0.0);
    // 1 МиБ/с = 8,388608 Мбит/с, а не 8
    assert!((mbps_of(1 << 20, 1.0) - 8.388_608).abs() < 1e-9);
    // удвоение времени вдвое снижает; удвоение байт вдвое повышает (метаморфическое)
    for_seeds("mbps", |r| {
        let (b, s) = (1 + r.below(1 << 36), 0.01 + r.unit() * 30.0);
        let v = mbps_of(b, s);
        assert!(v.is_finite() && v > 0.0);
        assert!((mbps_of(b * 2, s) / v - 2.0).abs() < 1e-9);
        assert!((mbps_of(b, s * 2.0) / v - 0.5).abs() < 1e-9);
    });
}

/// Ряд (время, байты) с шагом 100 мс и постоянной скоростью `mbps` до `secs`.
fn steady_series(mbps: f64, secs: f64) -> Vec<(f64, u64)> {
    let mut v = vec![(0.0, 0u64)];
    let mut t = 0.1;
    while t <= secs + 1e-9 {
        v.push((t, (mbps * 1e6 / 8.0 * t) as u64));
        t += 0.1;
    }
    v
}

#[test]
fn steady_rate_hand_cases() {
    // 100 Мбит/с постоянно 6 с: байт 75 000 000; ожидание 100
    let s = steady_series(100.0, 6.0);
    assert!((steady_rate(&s) - 100.0).abs() < 0.5, "{}", steady_rate(&s));
    // пусто и одна точка — не паникует, значение конечное
    assert!(steady_rate(&[]).is_finite());
    assert!(steady_rate(&[(0.0, 0)]).is_finite());
    // разгон: первые 2 секунды вдвое медленнее — «установившаяся» ближе к 100, а не к среднему за всё время
    let mut v = vec![(0.0, 0u64)];
    let mut bytes = 0.0;
    let mut t = 0.0;
    while t < 8.0 {
        t += 0.1;
        let rate = if t < 2.0 { 50.0 } else { 100.0 };
        bytes += rate * 1e6 / 8.0 * 0.1;
        v.push((t, bytes as u64));
    }
    let avg = mbps_of(v.last().unwrap().1, 8.0);
    let st = steady_rate(&v);
    assert!(avg < 90.0 && st > avg, "avg {avg} steady {st}");
    assert!((st - 100.0).abs() < 8.0, "{st}");
}

#[test]
fn early_stop_rules() {
    // 100 Мбит/с ровно, 6 секунд: правило срабатывает и отдаёт скорость ≈100
    let r = early_stop_rate(&steady_series(100.0, 6.0)).expect("ровная линия должна считаться устоявшейся");
    assert!((r - 100.0).abs() < 1.0, "{r}");
    // меньше 3 секунд — рано
    assert!(early_stop_rate(&steady_series(100.0, 2.9)).is_none());
    // медленная линия (10 Мбит/с < 20) — гоним до конца срока
    assert!(early_stop_rate(&steady_series(10.0, 8.0)).is_none());
    // разгон: скорость растёт линейно (ускорение) — окна не совпадают в пределах ±5%
    let mut v = vec![(0.0, 0u64)];
    let mut bytes = 0.0;
    let mut t = 0.0;
    while t < 6.0 {
        t += 0.1;
        bytes += (20.0 + 30.0 * t) * 1e6 / 8.0 * 0.1;
        v.push((t, bytes as u64));
    }
    assert!(early_stop_rate(&v).is_none(), "рост скорости не должен считаться «устоялась»");
    // один провал в последней секунде: последние 1,0 с идут на 70% скорости
    let base = steady_series(100.0, 6.0);
    let mut w = base.clone();
    let cut = 5.0;
    let mut lost = 0.0;
    for p in w.iter_mut() {
        if p.0 > cut {
            lost += 0.3 * 100.0 * 1e6 / 8.0 * 0.1;
        }
        p.1 = (base.iter().find(|b| (b.0 - p.0).abs() < 1e-9).unwrap().1 as f64 - lost) as u64;
    }
    assert!(early_stop_rate(&w).is_none(), "провал на 30% в последней секунде — не «устоялась»");
}

/// Свойство: на случайных монотонных рядах ничего не паникует, значения конечны; если правило сработало — скорость ≥ 0.
/// Метаморфическое: все байты ×3 → скорость ×3, а решение «устоялась» остаётся тем же (пороги далеко от значений).
#[test]
fn early_stop_and_steady_properties() {
    for_seeds("early", |r| {
        let base = 30.0 + r.unit() * 800.0;
        let wobble = r.unit() * 0.12;
        let secs = 3.0 + r.unit() * 7.0;
        let mut v = vec![(0.0, 0u64)];
        let (mut t, mut bytes) = (0.0f64, 0.0f64);
        while t < secs {
            t += 0.1;
            let rate = base * (1.0 + wobble * (r.unit() * 2.0 - 1.0));
            bytes += rate * 1e6 / 8.0 * 0.1;
            v.push((t, bytes as u64));
        }
        let s = steady_rate(&v);
        assert!(s.is_finite() && s >= 0.0 && s < base * 3.0, "{s} при базе {base}");
        let e = early_stop_rate(&v);
        let tripled: Vec<(f64, u64)> = v.iter().map(|p| (p.0, p.1 * 3)).collect();
        let e3 = early_stop_rate(&tripled);
        assert_eq!(e.is_some(), e3.is_some(), "решение зависит от масштаба при базе {base}");
        if let (Some(a), Some(b)) = (e, e3) {
            assert!(a.is_finite() && (b / a - 3.0).abs() < 1e-6);
        }
        // ровный ряд (wobble ≤ 1%) всегда считается устоявшимся после 3 с
    });
    // ровный ряд без шума всегда устаивается
    for base in [25.0, 80.0, 940.0] {
        assert!(early_stop_rate(&steady_series(base, 5.0)).is_some(), "{base}");
    }
}

// ───────────── фальшивые серверы ─────────────

fn path_of(head: &str) -> (String, String) {
    let first = head.lines().next().unwrap_or("");
    let mut it = first.split_whitespace();
    (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string())
}

fn bytes_param(path: &str) -> usize {
    path.split("bytes=").nth(1).and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next()).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// Отдать `bytes` байт блоками; `pace` — (блок, пауза после блока) для ограничения скорости. Счёт отданного — в `sent`.
fn serve_down(c: &mut std::net::TcpStream, bytes: usize, pace: Option<(usize, std::time::Duration)>, sent: &AtomicU64) -> bool {
    if c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {bytes}\r\nConnection: close\r\n\r\n").as_bytes()).is_err() {
        return false;
    }
    let block = vec![7u8; pace.map_or(64 * 1024, |p| p.0)];
    let mut left = bytes;
    while left > 0 {
        let n = left.min(block.len());
        if c.write_all(&block[..n]).is_err() {
            return false;
        }
        sent.fetch_add(n as u64, Ordering::Relaxed);
        left -= n;
        if let Some((_, d)) = pace {
            std::thread::sleep(d);
        }
    }
    true
}

/// Сервер задержки: отвечает на пустые запросы по общему соединению, i-й запрос (с 0 — прогрев) ждёт `lat[i]` мс.
/// `drop_after` — после стольких запросов закрывает соединение и больше не отвечает.
fn latency_server(lat: Vec<u64>, drop_after: Option<usize>, status: &'static str) -> (String, Arc<AtomicUsize>) {
    let n = Arc::new(AtomicUsize::new(0));
    let n2 = n.clone();
    let base = local_server(move |mut c, _| loop {
        if read_head(&mut c).is_none() {
            return;
        }
        let i = n2.fetch_add(1, Ordering::SeqCst);
        if drop_after.is_some_and(|d| i >= d) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(*lat.get(i).unwrap_or(lat.last().unwrap_or(&0))));
        let body = if status.starts_with("500") { "oops" } else { "" };
        if c.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).is_err() {
            return;
        }
    });
    (base, n)
}

/// Эталон: сервер сам задерживает ответы на 20..160 мс (после прогрева). Истинная медиана 8 замеров — полусумма
/// двух средних, (80+100)/2 = 90; джиттер — все расхождения по 20 → 20. Допуск ±8 мс на накладные расходы.
#[test]
fn ping_stats_median_and_jitter_against_server_side_latency() {
    if !timing_ok("ping_stats_median_and_jitter_against_server_side_latency") {
        return;
    }
    let (base, _) = latency_server(vec![0, 20, 40, 60, 80, 100, 120, 140, 160], None, "200 OK");
    let (med, jit) = with_base(base, || ping_stats(8)).expect("локальный сервер отвечает");
    // накладные расходы планировщика (особенно на загруженной машине) только ДОБАВЛЯЮТ время: нижняя граница строгая,
    // верхняя — с запасом 25 мс
    assert!((8.0..=32.0).contains(&jit), "джиттер {jit} вместо 20");
    // медиана — полусумма двух средних: (80+100)/2 = 90; сверху запас на накладные расходы
    assert!((88.0..=115.0).contains(&med), "медиана программы {med}");
}

/// Для чётного числа замеров (8 — штатное) «медиана» — верхний из двух средних, а не их полусумма: завышает на половину
/// зазора между ними. Выбранное значение стабильно, но название «медиана» неточно.
#[test]
fn ping_stats_even_count_true_median() {
    let (base, _) = latency_server(vec![0, 20, 40, 60, 80, 100, 120, 140, 160], None, "200 OK");
    let (med, _) = with_base(base, || ping_stats(8)).unwrap();
    assert!((88.0..=96.0).contains(&med), "{med} (истинная медиана 90 мс)");
}

#[test]
fn ping_stats_closed_port_returns_none_fast() {
    // слушатель закрыт: порт отказывает в соединении (проверяем, что порт не успели занять параллельные тесты)
    let port = loop {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        drop(l);
        if std::net::TcpStream::connect(("127.0.0.1", p)).is_err() {
            break p;
        }
    };
    let (r, secs) = with_base(format!("http://127.0.0.1:{port}"), || {
        let t = Instant::now();
        (ping_stats(8), t.elapsed().as_secs_f64())
    });
    assert!(r.is_none());
    assert!(secs < 5.0, "{secs} с");
}

#[test]
fn ping_stats_server_closes_or_speaks_garbage() {
    let close = local_server(|c, _| drop(c));
    assert!(with_base(close, || ping_stats(8)).is_none());
    let garbage = local_server(|mut c, _| {
        read_head(&mut c);
        let _ = c.write_all(b"HELLO WORLD\r\n\r\n\x00\x01\x02");
    });
    assert!(with_base(garbage, || ping_stats(8)).is_none());
    // заголовок без конца
    let partial = local_server(|mut c, _| {
        read_head(&mut c);
        let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Le");
        std::thread::sleep(std::time::Duration::from_millis(300));
    });
    assert!(with_base(partial, || ping_stats(8)).is_none());
}

/// Частичный результат: сервер ответил на прогрев и 5 замеров и пропал. Замеров ≥3 — результат считается по 5.
#[test]
fn ping_stats_partial_series_is_reported_from_available_samples() {
    if !timing_ok("ping_stats_partial_series_is_reported_from_available_samples") {
        return;
    }
    let (base, _) = latency_server(vec![0, 10, 10, 10, 10, 10], Some(6), "200 OK");
    let (med, jit) = with_base(base, || ping_stats(8)).expect("5 удачных замеров достаточно");
    assert!((5.0..40.0).contains(&med), "{med}");
    assert!(jit < 25.0, "{jit}");
    // два замера — мало
    let (base, _) = latency_server(vec![0, 10, 10], Some(3), "200 OK");
    assert!(with_base(base, || ping_stats(8)).is_none());
}

/// Код ответа не важен (так задумано: «как http.client»): 500 с телом тоже даёт задержку. Граница допущения: страница-
/// заглушка (captive portal) или ошибка сервера выглядят как «сервер замера отвечает».
#[test]
fn ping_stats_counts_http_500_as_reachable_documented() {
    let (base, _) = latency_server(vec![0; 12], None, "500 Internal Server Error");
    assert!(with_base(base, || ping_stats(8)).is_some());
}

/// Молчащий сервер: чтение ограничено 4 с, прогрев повторяется один раз — итог не дольше ~12 с.
#[test]
#[ignore = "медленно (~20 с); запускается с --include-ignored"]
fn ping_stats_mute_server_times_out_in_bounds() {
    let base = local_server(|mut c, _| {
        read_head(&mut c);
        std::thread::sleep(std::time::Duration::from_secs(30));
    });
    let (r, secs) = with_base(base, || {
        let t = Instant::now();
        (ping_stats(8), t.elapsed().as_secs_f64())
    });
    assert!(r.is_none());
    assert!(secs < 25.0, "{secs} с");
}

/// Заявлено «чтение — 4 с, прогрев повторяется один раз» (≈9 с); на деле общий срок запроса (10 с) перекрывает
/// покоординатный таймаут чтения, и два прогрева занимают ≈20 с: окно замера висит вдвое дольше заявленного.
#[test]
fn ping_stats_mute_server_honors_read_timeout() {
    let base = local_server(|mut c, _| {
        read_head(&mut c);
        std::thread::sleep(std::time::Duration::from_secs(30));
    });
    let (_, secs) = with_base(base, || {
        let t = Instant::now();
        (ping_stats(8), t.elapsed().as_secs_f64())
    });
    assert!(secs < 13.0, "{secs} с");
}

// ───────────── передача данных ─────────────

/// Единицы на живом потоке: сервер отдаёт ровно 1 МБ/с (блок 100 КБ раз в 100 мс) одним потоком → 8 Мбит/с.
/// Если бы программа путала байты и биты или MiB и МБ, результат ушёл бы в 8 раз или на 4,9% (последнее не поймать).
#[test]
fn transfer_down_throttled_server_reports_about_8_mbit() {
    if !timing_ok("transfer_down_throttled_server_reports_about_8_mbit") {
        return;
    }
    let sent = Arc::new(AtomicU64::new(0));
    let s2 = sent.clone();
    let base = local_server(move |mut c, _| {
        let Some(h) = read_head(&mut c) else { return };
        let (_, p) = path_of(&h);
        serve_down(&mut c, bytes_param(&p), Some((100_000, std::time::Duration::from_millis(100))), &s2);
    });
    let prog = std::sync::Mutex::new(Vec::new());
    let r = with_base(base, || transfer(&mut |p| prog.lock().unwrap().push(p), true, 1, 4.0, 1 << 30, 50_000_000, false));
    let m = r.expect("ответ есть").expect("байт достаточно");
    assert!((5.5..=10.5).contains(&m), "ожидалось ≈8 Мбит/с, получено {m}");
    // независимый счёт: сервер отдал ≈ 4 с × 1 МБ/с
    let srv = sent.load(Ordering::Relaxed) as f64 * 8.0 / 4.0 / 1e6;
    assert!((5.0..=11.0).contains(&srv), "сервер отдал {srv} Мбит/с");
    // инварианты хода: конечные числа, доля 0..1, не убывает, скорость ≥ 0
    let prog = prog.lock().unwrap();
    assert!(!prog.is_empty());
    let mut last = 0.0;
    for p in prog.iter() {
        assert!(p.mbps.is_finite() && p.mbps >= 0.0 && p.mbps < 1000.0, "{p:?}");
        assert!((0.0..=1.0).contains(&p.fraction) && p.fraction >= last, "{p:?}");
        assert_eq!(p.phase, SpeedPhase::Down);
        last = p.fraction;
    }
}

/// Сервер отдаёт ≈100 МБ/с (блок 500 КБ раз в 5 мс). Эталон — собственные отметки сервера: сколько байт он отдал между
/// первой и последней записью. Отчёт программы должен совпасть с ним в пределах ±35% (и лежать в разумных 300..1000).
#[test]
fn transfer_down_paced_server_matches_server_side_window() {
    if !timing_ok("transfer_down_paced_server_matches_server_side_window") {
        return;
    }
    let sent = Arc::new(AtomicU64::new(0));
    let win: Arc<std::sync::Mutex<Option<(Instant, Instant)>>> = Arc::new(std::sync::Mutex::new(None));
    let (s2, w2) = (sent.clone(), win.clone());
    let base = local_server(move |mut c, _| {
        let Some(h) = read_head(&mut c) else { return };
        let (_, p) = path_of(&h);
        let n = bytes_param(&p);
        let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {n}\r\nConnection: close\r\n\r\n").as_bytes());
        let block = vec![3u8; 500_000];
        let mut left = n;
        while left > 0 {
            let k = left.min(block.len());
            if c.write_all(&block[..k]).is_err() {
                return;
            }
            s2.fetch_add(k as u64, Ordering::Relaxed);
            let now = Instant::now();
            let mut g = w2.lock().unwrap();
            *g = Some((g.map_or(now, |x| x.0), now));
            drop(g);
            left -= k;
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    let m = with_base(base, || transfer(&mut |_| {}, true, 1, 3.0, 1 << 34, 400_000_000, false)).unwrap().unwrap();
    let (a, b) = win.lock().unwrap().unwrap();
    let srv = sent.load(Ordering::Relaxed) as f64 * 8.0 / (b - a).as_secs_f64() / 1e6;
    assert!((300.0..1100.0).contains(&m), "ожидали порядка 800 Мбит/с, получено {m}");
    assert!((m / srv - 1.0).abs() < 0.35, "программа {m}, сервер {srv} Мбит/с");
}

/// Отдача: сервер сам читает тело (≈100 МБ/с: 500 КБ раз в 5 мс) и записывает, сколько принял и за какое время.
#[test]
fn transfer_up_matches_server_received_window() {
    if !timing_ok("transfer_up_matches_server_received_window") {
        return;
    }
    let got = Arc::new(AtomicU64::new(0));
    let win: Arc<std::sync::Mutex<Option<(Instant, Instant)>>> = Arc::new(std::sync::Mutex::new(None));
    let (g2, w2) = (got.clone(), win.clone());
    let base = local_server(move |mut c, _| {
        use std::io::Read;
        let Some(h) = read_head(&mut c) else { return };
        let len: usize =
            h.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok())).unwrap_or(0);
        let mut left = len;
        let mut buf = vec![0u8; 500_000];
        while left > 0 {
            match c.read(&mut buf[..left.min(500_000)]) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    g2.fetch_add(n as u64, Ordering::Relaxed);
                    let now = Instant::now();
                    let mut g = w2.lock().unwrap();
                    *g = Some((g.map_or(now, |x| x.0), now));
                    drop(g);
                    left -= n;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
        let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let m = with_base(base, || transfer(&mut |_| {}, false, 1, 3.0, 1 << 34, 200_000_000, false)).unwrap().unwrap();
    let (a, b) = win.lock().unwrap().unwrap();
    let srv = got.load(Ordering::Relaxed) as f64 * 8.0 / (b - a).as_secs_f64() / 1e6;
    assert!((300.0..1100.0).contains(&m), "ожидали порядка 800 Мбит/с, получено {m}");
    // в сокет «ушло» чуть больше, чем сервер уже принял (буферы), поэтому допуск шире, чем на приёме
    assert!((m / srv - 1.0).abs() < 0.45, "программа {m}, сервер принял {srv} Мбит/с");
}

/// Отказ сервера на отдаче (закрывает соединение, ничего не ответив): байты, лишь ушедшие в буфер сокета, не должны
/// превращаться в «скорость». Ложный зелёный — главное, что здесь ловится.
#[test]
fn transfer_up_dropped_connections_never_report_speed() {
    let base = local_server(|mut c, _| {
        read_head(&mut c);
        // закрываем, не читая тело и не отвечая
        let _ = c.shutdown(std::net::Shutdown::Both);
    });
    let r = with_base(base, || transfer(&mut |_| {}, false, 2, 3.0, 1 << 30, 2_000_000, false));
    assert!(matches!(r, Err(_) | Ok(None)), "ложная скорость: {r:?}");
}

/// Обрыв посреди ответа при скачивании: ни паники, ни зависания; всё, что успело прийти, учтено, ошибка не теряется.
#[test]
fn transfer_down_midstream_reset_is_bounded() {
    let base = local_server(|mut c, _| {
        let Some(h) = read_head(&mut c) else { return };
        let (_, p) = path_of(&h);
        let want = bytes_param(&p);
        let _ = c.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {want}\r\nConnection: close\r\n\r\n").as_bytes());
        let _ = c.write_all(&vec![1u8; 100_000]);
        // и закрыть — клиент ждёт ещё want-100000 байт
    });
    let (r, secs) = with_base(base, || {
        let t = Instant::now();
        (transfer(&mut |_| {}, true, 2, 3.0, 1 << 30, 10_000_000, false), t.elapsed().as_secs_f64())
    });
    assert!(secs < 8.0, "{secs} с");
    // итог: либо ошибка (мало байт — меньше порога 500 КБ), либо «не измерить»; но не огромная скорость
    if let Ok(Some(m)) = r {
        assert!(m < 1000.0, "{m}");
    }
}

/// 429 на скачивание: потоки останавливаются сразу и вернут BUSY; серверу посылается мало запросов.
#[test]
fn transfer_429_stops_all_streams_immediately() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = hits.clone();
    let base = local_server(move |mut c, _| {
        read_head(&mut c);
        h2.fetch_add(1, Ordering::SeqCst);
        let _ = c.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let (r, secs) = with_base(base, || {
        let t = Instant::now();
        (transfer(&mut |_| {}, true, 6, 10.0, 1 << 30, 50_000_000, true), t.elapsed().as_secs_f64())
    });
    assert_eq!(r, Err(BUSY.to_string()));
    assert!(secs < 3.0, "после 429 замер должен закончиться сразу: {secs} с");
    // 6 потоков → не больше 6 запросов (каждый поток вышел после первого 429), ни одного повторного
    assert!(hits.load(Ordering::SeqCst) <= 6, "{}", hits.load(Ordering::SeqCst));
}

/// Лимитная сеть: 429 — без второй попытки (один запрос — один отказ).
#[test]
fn retry_policy_metered_has_no_second_attempt() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = hits.clone();
    let always429 = local_server(move |mut c, _| {
        read_head(&mut c);
        h2.fetch_add(1, Ordering::SeqCst);
        let _ = c.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let r = with_base(always429, || transfer_with_retry(&mut |_| {}, true, true));
    assert_eq!(r, Err(BUSY.to_string()));
    assert_eq!(hits.load(Ordering::SeqCst), 1, "на лимитной сети повторов быть не должно");
}

/// Отдача: сервер ЧИТАЕТ тело целиком, но в конце отвечает 429 (отказ). Принятые-и-отвергнутые байты не должны давать
/// «скорость»: ожидание — BUSY. Фактически первый поток получает 429 и поднимает stop; остальные тоже получают 429,
/// но при поднятом stop их байты не возвращаются в счёт — `transfer` видит >500 КБ и отдаёт число.
#[test]
fn transfer_up_429_after_body_must_not_report_speed() {
    let base = local_server(|mut c, _| {
        use std::io::Read;
        let Some(h) = read_head(&mut c) else { return };
        let len: usize =
            h.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok())).unwrap_or(0);
        let mut left = len;
        let mut buf = vec![0u8; 65_536];
        while left > 0 {
            match c.read(&mut buf[..left.min(65_536)]) {
                Ok(0) | Err(_) => return,
                Ok(n) => left -= n,
            }
        }
        let _ = c.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let r = with_base(base, || transfer(&mut |_| {}, false, 6, 8.0, 300_000_000, 20_000_000, true));
    assert_eq!(r, Err(BUSY.to_string()), "ложная скорость при отказе сервера: {r:?}");
}

/// Сервер отвечает 429 на отдачу сразу, тела не читает и закрывает соединение (так отвечают многие серверы). Клиент
/// пишет тело и получает сброс соединения раньше, чем успевает прочитать 429. Ожидание: BUSY. Ложной скорости здесь нет
/// (проверено: результат — ошибка), но отказ не узнан.
#[test]
fn transfer_up_429_with_immediate_close_is_recognized_as_busy() {
    let base = local_server(|mut c, _| {
        read_head(&mut c);
        let _ = c.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let r = with_base(base, || transfer(&mut |_| {}, false, 6, 8.0, 300_000_000, 20_000_000, true));
    assert_eq!(r, Err(BUSY.to_string()), "отказ 429 не узнан: {r:?}");
}

/// Постоянный 429: вторая облегчённая попытка тоже получает отказ — итог BUSY, а не «0 Мбит/с» и не зависание.
#[test]
fn retry_after_429_then_429_again_is_busy_not_zero() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = hits.clone();
    let base = local_server(move |mut c, _| {
        read_head(&mut c);
        h2.fetch_add(1, Ordering::SeqCst);
        let _ = c.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let (r, secs) = with_base(base, || {
        let t = Instant::now();
        (transfer_with_retry(&mut |_| {}, true, false), t.elapsed().as_secs_f64())
    });
    assert_eq!(r, Err(BUSY.to_string()));
    assert!(secs < 5.0, "{secs} с");
    // 6 потоков первой попытки + 2 потока второй — не больше восьми запросов
    assert!(hits.load(Ordering::SeqCst) <= 8, "{}", hits.load(Ordering::SeqCst));
}

/// Полный `speed_test` против фальшивого сервера: статусы и единицы в итоговых строках; с `SPEED_BUSY=1` скорость
/// помечена «не проверено», а не красным. Занимает ~10–20 с.
#[test]
#[ignore = "медленно (10–25 с); запускается с --include-ignored"]
fn speed_test_end_to_end_on_fake_server_and_busy_switch() {
    let sent = Arc::new(AtomicU64::new(0));
    let s2 = sent.clone();
    let base = local_server(move |mut c, _| loop {
        use std::io::Read;
        let Some(h) = read_head(&mut c) else { return };
        let (m, p) = path_of(&h);
        if m == "POST" {
            let len: usize =
                h.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok())).unwrap_or(0);
            let mut left = len;
            let mut buf = vec![0u8; 65_536];
            while left > 0 {
                match c.read(&mut buf[..left.min(65_536)]) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => left -= n,
                }
            }
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let n = bytes_param(&p);
        if n == 0 {
            let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        serve_down(&mut c, n, None, &s2);
        return;
    });
    let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::remove_var("SPEED_BUSY");
    let rows = with_base(base.clone(), || speed_test(|_| {}));
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[0].title, SPEED_TITLES[0]);
    assert!(rows[0].value.contains(" мс · джиттер "), "{}", rows[0].value);
    for r in &rows[1..] {
        assert!(r.value.ends_with(" Мбит/с"), "{}", r.value);
        let n: f64 = r.value.split_whitespace().next().unwrap().parse().unwrap();
        assert!(n.is_finite() && n > 5.0, "{}", r.value);
    }
    std::env::set_var("SPEED_BUSY", "1");
    let rows = with_base(base, || speed_test(|_| {}));
    std::env::remove_var("SPEED_BUSY");
    for r in &rows[1..] {
        assert_eq!(r.status, INFO, "{r:?}");
        assert!(r.unchecked, "ограничение сервера — «не проверено», не авария: {r:?}");
        assert_eq!(r.value, BUSY);
    }
}

/// Сравнение с независимыми инструментами: задержка программы к настоящему серверу замера против (а) `curl` — десять
/// запросов по одному открытому соединению, берём время запроса 2..10 (то же, что меряет программа: запрос по уже
/// открытому соединению), и (б) RTT системного `ping`. Одноразово: ≈20 маленьких запросов и 5 ICMP. Печатает все цифры.
/// ICMP только справочно: через VPN, прозрачный прокси или роутер он может отвечать за единицы миллисекунд (как и TCP-
/// рукопожатие, о чём сказано в комментарии к `ping_stats`), поэтому сравнение со значением `ping` НЕ утверждается.
#[test]
#[ignore = "ходит в интернет (≈20 маленьких запросов и 5 ICMP); запускается вручную"]
fn real_http_latency_vs_system_tools() {
    let _g = SPEED_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let Some((med, jit)) = ping_stats(8) else {
        eprintln!("[сравнение] сервер замера не ответил — сравнивать нечего");
        return;
    };
    let url = format!("https://{SPEED_HOST}/__down?bytes=0");
    let mut args: Vec<String> = vec!["-s".into(), "-o".into(), "/dev/null".into(), "-w".into(), "%{time_total}\\n".into()];
    args.extend(std::iter::repeat_n(url, 10));
    let out = std::process::Command::new("curl").args(&args).output();
    let mut t: Vec<f64> = out
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse::<f64>().ok()).map(|s| s * 1000.0).collect())
        .unwrap_or_default();
    let icmp = std::process::Command::new("ping").args(["-c", "5", "-i", "0.3", "-W", "2", SPEED_HOST]).output();
    let text = icmp.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let avg = text
        .lines()
        .find(|l| l.contains("min/avg/max"))
        .and_then(|l| l.split('=').nth(1))
        .and_then(|v| v.split('/').nth(1))
        .and_then(|v| v.trim().parse::<f64>().ok());
    eprintln!(
        "[сравнение] программа: медиана {med:.1} мс, джиттер {jit:.1} мс; curl (запросы 2..10): {:?}; системный ping: среднее {avg:?} мс",
        t.get(1..).map(|v| v.iter().map(|x| x.round()).collect::<Vec<_>>())
    );
    if t.len() >= 6 {
        t.remove(0); // первый запрос — с рукопожатием и TLS
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let curl_med = t[t.len() / 2];
        eprintln!("[сравнение] медиана curl по открытому соединению: {curl_med:.1} мс");
        assert!((med - curl_med).abs() <= curl_med * 0.4 + 30.0, "программа {med:.1} мс, curl {curl_med:.1} мс");
    }
}

/// Бег по кругу «запуск/остановка замера»: сотни циклов `ping_stats` и `transfer` против локального сервера;
/// число потоков и дескрипторов не растёт. Длительность — REGRESS_SOAK_SECS (по умолчанию 30 с), CSV — REGRESS_CSV.
#[test]
#[ignore = "длительное испытание: см. REGRESSION_TESTS.md"]
fn soak_speed_start_stop_cycles() {
    let secs = soak_secs(30);
    let sent = Arc::new(AtomicU64::new(0));
    let s2 = sent.clone();
    let base = local_server(move |mut c, _| loop {
        let Some(h) = read_head(&mut c) else { return };
        let (_, p) = path_of(&h);
        let n = bytes_param(&p);
        if n == 0 {
            if c.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").is_err() {
                return;
            }
            continue;
        }
        serve_down(&mut c, n, None, &s2);
        return;
    });
    csv_line("t_s,cycle,rss_kb,threads,fds,cycle_ms,ok");
    let (r0, th0, fd0) = proc_stats();
    let t0 = Instant::now();
    let mut cycle = 0u64;
    let mut max_th = th0;
    let mut max_fd = fd0;
    with_base(base, || {
        while t0.elapsed().as_secs() < secs {
            let s = Instant::now();
            let ok = ping_stats(4).is_some()
                && matches!(transfer(&mut |_| {}, true, 2, 1.2, 20_000_000, 2_000_000, false), Ok(Some(m)) if m.is_finite() && m > 0.0);
            cycle += 1;
            let (rss, th, fd) = proc_stats();
            max_th = max_th.max(th);
            max_fd = max_fd.max(fd);
            csv_line(&format!("{},{cycle},{rss},{th},{fd},{},{}", t0.elapsed().as_secs(), s.elapsed().as_millis(), ok as u8));
            assert!(ok, "цикл {cycle} не дал результата");
        }
    });
    std::thread::sleep(std::time::Duration::from_secs(4)); // потоки замера успевают завершиться
    let (r1, th1, fd1) = proc_stats();
    csv_line(&format!("final,{cycle},{r1},{th1},{fd1},,"));
    assert!(cycle >= 3);
    // дескрипторы и потоки после остановки вернулись к исходному уровню (запас на потоки сервера и тестового раннера)
    assert!(fd1 <= fd0 + 20, "утечка дескрипторов: было {fd0}, стало {fd1} (максимум {max_fd})");
    assert!(th1 <= th0 + 40, "потоки не завершаются: было {th0}, стало {th1} (максимум {max_th})");
    let _ = r0;
}
