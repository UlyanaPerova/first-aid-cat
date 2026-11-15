//! Регрессионные тесты учёта токенов и лимитов (`limits.rs`). Подключается как дочерний модуль `limits::regress`
//! (видит закрытые функции). Журналы — только выдуманные, во временной папке; настоящие `~/.claude`, `~/.codex`
//! не читаются: состояние чтения у каждого теста своё (`Mutex<Option<Files>>`), а не общие `CLAUDE`/`CODEX`.
#![allow(clippy::manual_is_multiple_of)] // is_multiple_of стабилен с Rust 1.90, а проект собирается с 1.89

use super::*;
use crate::regress_common::*;
use crate::ui::localtime;

fn new_state() -> Mutex<Option<Files>> {
    Mutex::new(None)
}

/// Строка журнала Claude Code (ответ модели с usage).
fn cl_line(id: &str, ts: &str, model: &str, inp: u64, out: u64, cr: u64, cw: u64) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{ts}\",\"cwd\":\"/home/u/proj\",\"message\":{{\"id\":\"{id}\",\"model\":\"{model}\",\"usage\":{{\"input_tokens\":{inp},\"output_tokens\":{out},\"cache_read_input_tokens\":{cr},\"cache_creation_input_tokens\":{cw}}}}}}}\n"
    )
}

fn scan_claude(state: &Mutex<Option<Files>>, root: &Path) {
    scan(state, &[root.join("projects")], 4, &[b"\"usage\""], &mut claude_line);
}

fn total(state: &Mutex<Option<Files>>) -> Tok {
    let mut t = Tok::default();
    for x in sum_usage(state).values() {
        t.add(x);
    }
    t
}

/// Время «сейчас минус N секунд» в формате журнала (UTC, «Z»), чтобы записи попадали в окно DAYS.
fn ts_ago(secs: i64) -> String {
    let t = crate::ui::now_secs() - secs;
    let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // гражданская дата из числа дней — наивно, циклом по годам (независимо от iso_secs)
    let (mut y, mut d) = (1970i64, days);
    let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    loop {
        let yl = if leap(y) { 366 } else { 365 };
        if d >= yl {
            d -= yl;
            y += 1;
        } else {
            break;
        }
    }
    let ml = [31, if leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    while d >= ml[m] {
        d -= ml[m];
        m += 1;
    }
    format!("{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", m + 1, d + 1, rem / 3600, rem % 3600 / 60, rem % 60)
}

// ───────────── время ─────────────

/// Эталон: Python `datetime(..., tzinfo=utc).timestamp()` (значения посчитаны отдельно, не этой программой).
#[test]
fn iso_secs_reference_table_from_python() {
    let t = [
        ("1970-01-01T00:00:00Z", 0),
        ("1999-12-31T23:59:59Z", 946_684_799),
        ("2000-02-29T12:00:00Z", 951_825_600),
        ("2000-03-01T00:00:00Z", 951_868_800),
        ("2024-02-29T23:59:59Z", 1_709_251_199),
        ("2026-11-04T23:03:29Z", 1_793_833_409),
        ("2026-11-15T00:00:00Z", 1_794_700_800),
        ("2038-01-19T03:14:07Z", 2_147_483_647),
        ("2038-01-19T03:14:08Z", 2_147_483_648), // за пределом i32: не должно переполниться
        ("2100-02-28T23:59:59Z", 4_107_542_399),
        ("2100-03-01T00:00:00Z", 4_107_542_400), // 2100 — не високосный
        ("1969-12-31T23:59:59Z", -1),
        ("1900-03-01T00:00:00Z", -2_203_891_200),
        ("2400-02-29T00:00:00Z", 13_574_563_200),
        ("9999-12-31T23:59:59Z", 253_402_300_799),
    ];
    for (s, want) in t {
        assert_eq!(iso_secs(s), Some(want), "{s}");
    }
    // дробные секунды и суффикс игнорируются
    assert_eq!(iso_secs("2026-11-04T23:03:29.405Z"), Some(1_793_833_409));
    assert_eq!(iso_secs("2026-11-04T23:03:29.999999999Z"), Some(1_793_833_409));
}

#[test]
fn iso_secs_rejects_garbage() {
    for s in [
        "",
        "вчера",
        "2026",
        "2026-11-04",
        "2026-11-04T",
        "2026-13-04T00:00:00Z",
        "2026-00-04T00:00:00Z",
        "2026-11-32T00:00:00Z",
        "2026-11-00T00:00:00Z",
        "2026/11/04T00:00:00Z",
        "2026-11-04 23:03:29Z",
        "2026-11-04T2x:03:29Z",
        "abcd-ef-ghTij:kl:mnZ",
        "2026-11-04T23:03",
    ] {
        assert_eq!(iso_secs(s), None, "{s:?}");
    }
    // обрезка посреди многобайтного знака не должна паниковать
    assert_eq!(iso_secs("2026-11-04T23:03:2ы"), None);
    assert_eq!(iso_secs("2026-11-04Tыы:03:29Z"), None);
}

/// Невозможные поля времени и даты принимаются молча (час 24..99, 31 февраля): такая строка журнала
/// попала бы не в тот день. Реальные журналы пишут настоящее время, так что практически не страшно.
#[test]
#[ignore = "известная ошибка: iso_secs принимает час 25, минуту 99 и 31 февраля (дата «перетекает» в март)"]
fn iso_secs_rejects_impossible_fields() {
    assert_eq!(iso_secs("2026-11-04T25:00:00Z"), None);
    assert_eq!(iso_secs("2026-11-04T23:99:00Z"), None);
    assert_eq!(iso_secs("2026-11-04T23:00:99Z"), None);
    assert_eq!(iso_secs("2026-02-31T00:00:00Z"), None);
    assert_eq!(iso_secs("2026-04-31T00:00:00Z"), None);
}

/// Время со смещением («+03:00») тихо читается как UTC: ошибка на 3 часа. Оба агента пишут «Z», это граница допущения.
#[test]
fn iso_secs_offset_is_ignored_documented() {
    assert_eq!(iso_secs("2026-11-04T23:03:29+03:00"), iso_secs("2026-11-04T23:03:29Z"));
}

/// Свойство: совпадает с наивным подсчётом дней по календарю (цикл по годам и месяцам).
#[test]
fn iso_secs_matches_naive_calendar() {
    for_seeds("iso_naive", |r| {
        let y = r.range(1970, 2400);
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let ml = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        let m = r.range(1, 12);
        let d = r.range(1, ml[m as usize - 1]);
        let (hh, mm, ss) = (r.range(0, 23), r.range(0, 59), r.range(0, 59));
        let mut days = 0i64;
        for yy in 1970..y {
            days += if (yy % 4 == 0 && yy % 100 != 0) || yy % 400 == 0 { 366 } else { 365 };
        }
        days += ml[..m as usize - 1].iter().sum::<i64>() + d - 1;
        let want = days * 86_400 + hh * 3600 + mm * 60 + ss;
        let s = format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z");
        assert_eq!(iso_secs(&s), Some(want), "{s}");
    });
}

/// Метаморфическое: +1 секунда в строке ↔ +1 к значению; +1 день — +86400 (в пределах месяца).
#[test]
fn iso_secs_is_additive() {
    for_seeds("iso_add", |r| {
        let (y, m, d) = (r.range(1971, 2100), r.range(1, 12), r.range(1, 27));
        let (h, mi, s) = (r.range(0, 23), r.range(0, 59), r.range(0, 58));
        let a = iso_secs(&format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")).unwrap();
        assert_eq!(iso_secs(&format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{:02}Z", s + 1)), Some(a + 1));
        assert_eq!(iso_secs(&format!("{y:04}-{m:02}-{:02}T{h:02}:{mi:02}:{s:02}Z", d + 1)), Some(a + 86_400));
    });
}

/// День недели: Python `datetime.weekday()`.
#[test]
fn weekday_reference_table_from_python() {
    let t = [
        (1970, 1, 1, "чт"),
        (1999, 12, 31, "пт"),
        (2000, 2, 29, "вт"),
        (2000, 3, 1, "ср"),
        (2024, 2, 29, "чт"),
        (2026, 11, 4, "ср"),
        (2026, 11, 15, "вс"),
        (2038, 1, 19, "вт"),
        (2100, 2, 28, "вс"),
        (2100, 3, 1, "пн"),
        (1900, 3, 1, "чт"),
        (2400, 2, 29, "вт"),
        (9999, 12, 31, "пт"),
    ];
    for (y, m, d, w) in t {
        assert_eq!(weekday(y, m, d), w, "{y}-{m}-{d}");
    }
    // шаг в сутки сдвигает день недели ровно на один по кругу (метаморфическое), весь 400-летний цикл
    let names = ["вс", "пн", "вт", "ср", "чт", "пт", "сб"];
    let (mut y, mut m, mut d) = (1970i64, 1i32, 1i32);
    let mut idx = 4usize; // чт
    let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    for _ in 0..(400 * 366) {
        assert_eq!(weekday(y, m, d), names[idx], "{y}-{m}-{d}");
        idx = (idx + 1) % 7;
        let ml = [31, if leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m as usize - 1];
        d += 1;
        if d > ml {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
}

/// Границы местного дня: ни одна секунда не теряется и не попадает в два дня; день растёт не быстрее суток.
#[test]
fn local_day_monotonic_and_calendar_shaped() {
    let base = crate::ui::now_secs() - 40 * 86_400;
    let mut prev = local_day(base);
    for i in 1..(30 * 24) {
        let d = local_day(base + i * 3600);
        assert!(d >= prev, "день пошёл назад: {prev} -> {d}");
        let (y, m, dd) = (d / 10_000, d / 100 % 100, d % 100);
        assert!((1..=12).contains(&m) && (1..=31).contains(&dd) && y > 2000, "{d}");
        // за час день сменился не больше чем на следующий календарный
        assert!(d == prev || (d - prev == 1 || d % 100 == 1), "{prev} -> {d}");
        prev = d;
    }
}

// ───────────── форматы ─────────────

#[test]
fn fmt_tokens_reference_table_from_python() {
    // эталон: «%.1f» Python для млн/млрд, целое деление для тысяч
    let t: [(u64, &str); 17] = [
        (0, "0"),
        (999, "999"),
        (1000, "1 тыс."),
        (1049, "1 тыс."),
        (52_300, "52 тыс."),
        (999_999, "999 тыс."),
        (1_000_000, "1,0 млн"),
        (1_049_999, "1,0 млн"),
        (1_050_000, "1,1 млн"),
        (1_950_000, "1,9 млн"),
        (3_400_000, "3,4 млн"),
        (999_949_999, "999,9 млн"),
        (1_000_000_000, "1,0 млрд"),
        (1_050_000_000, "1,1 млрд"),
        (12_345_678_901, "12,3 млрд"),
        (u64::MAX, "18446744073,7 млрд"),
        (7, "7"),
    ];
    for (n, want) in t {
        assert_eq!(fmt_tokens(n), want, "{n}");
    }
}

/// 999 950 000 … 999 999 999 токенов выводятся как «1000,0 млн» вместо «1,0 млрд» (округление до 0,1 переносит
/// значение через границу разряда). Видно в строках расхода и в подсказке плашки.
#[test]
#[ignore = "известная ошибка: fmt_tokens(999_999_999) даёт «1000,0 млн», а не «1,0 млрд»"]
fn fmt_tokens_never_shows_thousand_millions() {
    for n in [999_950_000u64, 999_999_000, 999_999_999] {
        let s = fmt_tokens(n);
        assert!(!s.starts_with("1000"), "{n} -> {s}");
    }
}

#[test]
fn fmt_tokens_monotone_unit_property() {
    for_seeds("fmt_tok", |r| {
        let n = r.below(2_000_000_000_000);
        let s = fmt_tokens(n);
        assert!(!s.is_empty() && !s.contains("NaN") && !s.contains("inf"), "{s}");
        // единица соответствует порядку числа (с запасом на округление у границ)
        if n < 1000 {
            assert_eq!(s, n.to_string());
        } else if n < 1_000_000 {
            assert!(s.ends_with("тыс."));
        } else if n < 999_950_000 {
            assert!(s.ends_with("млн"));
        }
    });
}

#[test]
fn until_and_ago_boundaries() {
    let now = 1_000_000;
    // (секунд до события, ожидаемое) — выложено вручную: d = s/86400, h = s%86400/3600, m = s%3600/60
    let t = [
        (-100, "через 1 мин"), // прошедшее не уходит в минус
        (0, "через 1 мин"),
        (59, "через 1 мин"),
        (60, "через 1 мин"),
        (119, "через 1 мин"),
        (120, "через 2 мин"),
        (3599, "через 59 мин"),
        (3600, "через 1 ч 0 мин"),
        (7140, "через 1 ч 59 мин"),
        (86_399, "через 23 ч 59 мин"),
        (86_400, "через 1 дн 0 ч"),
        (4 * 86_400 + 23 * 3600 + 3599, "через 4 дн 23 ч"),
    ];
    for (d, want) in t {
        assert_eq!(until(now + d, now), want, "d={d}");
    }
    let a =
        [(0, "0 мин"), (59, "0 мин"), (60, "1 мин"), (3599, "59 мин"), (3600, "1 ч"), (86_399, "23 ч"), (86_400, "1 дн."), (10 * 86_400, "10 дн.")];
    for (s, want) in a {
        assert_eq!(ago(s), want, "s={s}");
    }
}

fn ymd(t: i64) -> (i64, i32, i32) {
    let l = localtime(t);
    (l.year, l.mon, l.mday)
}

/// `when`/`when_day`: ожидаемое собрано руками из `ui::localtime` (другая функция) и сравнения календарных дат.
#[test]
fn when_and_when_day_match_hand_made_expectation() {
    let now = crate::ui::now_secs();
    for_seeds("when", |r| {
        let at = now + r.range(-3 * 86_400, 9 * 86_400);
        let l = localtime(at);
        let hm = format!("{:02}:{:02}", l.hour, l.min);
        let (same, next) = (ymd(at) == ymd(now), ymd(at) == ymd(now + 86_400));
        let want = if same {
            format!("в {hm}")
        } else if next {
            format!("завтра в {hm}")
        } else {
            format!("{:02}.{:02} в {hm}", l.mday, l.mon)
        };
        assert_eq!(when(at, now), want);
        let want_day = if same {
            format!("сегодня в {hm}")
        } else if next {
            format!("завтра в {hm}")
        } else {
            format!("{} {:02}.{:02} в {hm}", weekday(l.year, l.mon, l.mday), l.mday, l.mon)
        };
        assert_eq!(when_day(at, now), want_day);
    });
}

// ───────────── лимиты ─────────────

#[test]
fn window_clamps_and_rejects_nonfinite() {
    let w = |pct: &str, rs: &str| {
        window(serde_json::from_str::<serde_json::Value>(&format!("{{\"p\":{pct},\"resets_at\":{rs}}}")).ok().as_ref(), "p", 300)
    };
    assert_eq!(w("-5", "10").unwrap().pct, 0.0);
    assert_eq!(w("150", "10").unwrap().pct, 100.0);
    assert_eq!(w("1e308", "10").unwrap().pct, 100.0);
    assert_eq!(w("42.5", "10").unwrap().pct, 42.5);
    assert!(w("\"42\"", "10").is_none(), "строка вместо числа");
    assert!(w("null", "10").is_none());
    assert!(w("true", "10").is_none());
    // сброс — не число или дробное: считается неизвестным (0), а не падением
    assert_eq!(w("1", "\"x\"").unwrap().resets_at, 0);
    assert_eq!(w("1", "1700000000.5").unwrap().resets_at, 0);
    assert_eq!(w("1", "null").unwrap().resets_at, 0);
    assert!(window(None, "p", 1).is_none());
}

#[test]
fn now_pct_reset_boundary() {
    let w = Window { pct: 77.0, resets_at: 1000, minutes: 300 };
    assert_eq!(w.now_pct(999), 77.0);
    assert_eq!(w.now_pct(1000), 0.0, "ровно в момент сброса окно уже сброшено");
    assert_eq!(w.now_pct(1001), 0.0);
    let unknown = Window { pct: 77.0, resets_at: 0, minutes: 0 };
    assert_eq!(unknown.now_pct(i64::MAX), 77.0, "сброс неизвестен — процент не обнуляется");
}

#[test]
fn limits_top_picks_highest_non_reset() {
    let l = Limits {
        five: Some(Window { pct: 30.0, resets_at: 5000, minutes: 300 }),
        week: Some(Window { pct: 90.0, resets_at: 1000, minutes: 10080 }),
        at: 1,
    };
    assert_eq!(l.top(500), Some(90.0));
    assert_eq!(l.top(2000), Some(30.0), "недельное уже сброшено");
    assert_eq!(l.top(9000), Some(0.0));
    assert_eq!(Limits::default().top(1), None);
}

#[test]
fn parse_claude_limits_variants() {
    let ok = |t: &str| parse_claude_limits(t);
    // только одно окно
    let l = ok(r#"{"rate_limits":{"seven_day":{"used_percentage":21,"resets_at":9000}},"at":5}"#).unwrap();
    assert!(l.five.is_none() && l.week.is_some() && l.week.unwrap().minutes == 10_080 && l.at == 5);
    // минуты фиксированы по ключу: пять часов = 300, неделя = 10080
    let l = ok(r#"{"rate_limits":{"five_hour":{"used_percentage":1.5,"resets_at":9000}}}"#).unwrap();
    assert_eq!((l.five.unwrap().minutes, l.at), (300, 0));
    // нет окон / пустые / битые — None, а не ложные нули
    for t in [
        r#"{"rate_limits":{}}"#,
        r#"{"rate_limits":null}"#,
        r#"{"rate_limits":{"five_hour":{}}}"#,
        r#"{"rate_limits":{"five_hour":{"used_percentage":"x"}}}"#,
        "",
        "[]",
        "{",
        "null",
        "\u{0}",
    ] {
        assert_eq!(ok(t), None, "{t:?}");
    }
}

/// Неправдоподобное время сброса отбрасывается вместе с окном: сброс не может быть дальше длины окна от мига получения
/// данных (+10 минут на расхождение часов). Выкладка для 5 ч, at = 1 000 000: предел 1 000 000 + 18 000 + 600 = 1 018 600;
/// для недели: 1 000 000 + 604 800 + 600 = 1 605 400. Так в плашку попал «сброс через 26709 дн» (пробные данные
/// проверки: 4102444800 — 1 января 2100 года) — такое должно читаться как «неизвестно».
#[test]
fn implausible_reset_time_drops_the_window() {
    let w = |reset: i64, minutes: u32| Window { pct: 12.5, resets_at: reset, minutes };
    let l = |five: i64, week: i64| Limits { five: Some(w(five, 300)), week: Some(w(week, 10_080)), at: 1_000_000 };
    // на пределе — ещё годится, на секунду дальше — уже нет (каждое окно само по себе)
    assert_eq!(l(1_018_600, 1_605_400).checked(0), Some(l(1_018_600, 1_605_400)));
    let got = l(1_018_601, 1_605_400).checked(0).unwrap();
    assert_eq!((got.five, got.week.map(|x| x.resets_at)), (None, Some(1_605_400)));
    let got = l(1_018_600, 1_605_401).checked(0).unwrap();
    assert_eq!((got.five.map(|x| x.resets_at), got.week), (Some(1_018_600), None));
    // пробные данные проверки (2100 год), миллисекунды вместо секунд, отрицательное — окна нет вовсе
    for bad in [4_102_444_800, 1_794_700_800_000, -1, i64::MAX] {
        assert_eq!(l(bad, bad).checked(0), None, "{bad}");
    }
    // время сброса неизвестно (0) и уже прошедшее — допустимы
    assert!(l(0, 0).checked(0).is_some());
    assert!(l(999_000, 5).checked(0).is_some());
    // миг получения неизвестен (at = 0) — меряем от «сейчас»
    let no_at = Limits { five: Some(w(2_000_000, 300)), week: None, at: 0 };
    assert!(no_at.checked(1_990_000).is_some());
    assert_eq!(no_at.checked(1_000_000), None);
    // длина окна неизвестна (Codex без window_minutes) — предел 8 суток: 1 000 000 + 691 200 + 600
    let unk = |reset: i64| Limits { five: Some(w(reset, 0)), week: None, at: 1_000_000 };
    assert!(unk(1_691_800).checked(0).is_some());
    assert_eq!(unk(1_691_801).checked(0), None);
}

/// То же сквозь чтение файлов: в папке сеанса лежат пробные данные с невозможным сбросом — программа не показывает их,
/// а берёт правдоподобную сохранённую копию; если и та невозможна — честное «неизвестно».
#[test]
fn implausible_limits_file_is_not_shown() {
    with_fake_home("cl_bad", |_| {
        let now = crate::ui::now_secs();
        let body =
            |p: u32, reset: i64| format!("{{\"rate_limits\":{{\"five_hour\":{{\"used_percentage\":{p},\"resets_at\":{reset}}}}},\"at\":{now}}}");
        crate::store::write_private_quick(&claude_limits_path(), body(12, 4_102_444_800).as_bytes()).unwrap();
        assert_eq!(claude_limits(), None);
        crate::store::write_private_quick(&claude_limits_saved_path(), body(33, now + 3_600).as_bytes()).unwrap();
        assert_eq!(claude_limits().unwrap().five.unwrap().pct, 33.0);
        crate::store::write_private_quick(&claude_limits_saved_path(), body(33, now + 86_400).as_bytes()).unwrap();
        assert_eq!(claude_limits(), None);
    });
}

#[test]
fn limit_row_status_and_text() {
    let now = 1_794_700_000;
    let w = |pct: f64, reset: i64, min: u32| Window { pct, resets_at: reset, minutes: min };
    let st = |pct: f64| limit_row("A", "на 5 ч", &w(pct, now + 600, 300), now, now).status;
    assert_eq!(st(0.0), OK);
    assert_eq!(st(89.99), OK);
    assert_eq!(st(90.0), WARN);
    assert_eq!(st(99.99), WARN);
    assert_eq!(st(100.0), BAD);
    // окно сброшено: «0%», статус «хорошо» (в окне ничего не занято), без «данные … назад»
    let c = limit_row("A", "на 5 ч", &w(99.0, now - 1, 300), now - 7200, now);
    assert_eq!((c.status.as_str(), c.value.as_str()), (OK, "0% · уже сброшен"));
    // нет времени сброса — просто процент
    assert_eq!(limit_row("A", "на 5 ч", &w(42.4, 0, 300), now, now).value, "42%");
    // старые данные: ровно 15 минут — ещё не старые, 15 мин + 1 с — «данные 15 мин назад»
    let fresh = limit_row("A", "на 5 ч", &w(10.0, now + 100_000, 300), now - 900, now).value;
    assert!(!fresh.contains("назад"), "{fresh}");
    let stale = limit_row("A", "на 5 ч", &w(10.0, now + 100_000, 300), now - 901, now).value;
    assert!(stale.ends_with(" · данные 15 мин назад"), "{stale}");
    // округление процента: 0,5 → «0» или «1» (так считает format!), но не «NaN»
    assert!(!limit_row("A", "x", &w(0.5, 0, 300), now, now).value.contains("NaN"));
    // подпись окна
    assert_eq!(what_of(&w(1.0, 5, 300)), "на 5 ч");
    assert_eq!(what_of(&w(1.0, 5, 10_080)), "на неделю");
}

/// Окно «5 часов» без минут и без времени сброса подписывается «на неделю» (формула `what_of`): у Codex, если в
/// журнале нет ни window_minutes, ни resets_at, первичное окно получит чужую подпись.
#[test]
#[ignore = "известная ошибка: what_of подписывает окно без минут и без сброса «на неделю» даже для первичного (5-часового) окна"]
fn what_of_unknown_primary_is_not_week() {
    let primary_unknown = Window { pct: 10.0, resets_at: 0, minutes: 0 };
    assert_eq!(what_of(&primary_unknown), "на 5 ч");
}

// ───────────── учёт токенов (журналы) ─────────────

/// Нормальная сессия, эталон — ручной подсчёт: три разных ответа.
#[test]
fn claude_normal_session_hand_totals() {
    let t = Tmp::new("cl_norm");
    std::fs::create_dir_all(t.p("projects/p1")).unwrap();
    let mut body = String::new();
    body += &cl_line("m1", &ts_ago(3600), "claude-a", 10, 5, 100, 1);
    body += &cl_line("m2", &ts_ago(3000), "claude-a", 20, 7, 200, 2);
    body += &cl_line("m3", &ts_ago(2000), "claude-b", 1, 1, 0, 0);
    std::fs::write(t.p("projects/p1/s.jsonl"), body).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    // input 10+20+1=31; output 5+7+1=13; cache_read 100+200=300; cache_write 1+2=3; всего 347
    assert_eq!(total(&st), Tok { input: 31, output: 13, cache_read: 300, cache_write: 3 });
    assert_eq!(total(&st).total(), 347);
    // раскладка по моделям: у claude-b один ответ
    let by_model: BTreeMap<String, u64> = sum_usage(&st).into_iter().fold(BTreeMap::new(), |mut m, ((_, k), v)| {
        *m.entry(k).or_default() += v.total();
        m
    });
    assert_eq!(by_model["claude-a"], 10 + 5 + 100 + 1 + 20 + 7 + 200 + 2);
    assert_eq!(by_model["claude-b"], 2);
}

#[test]
fn claude_empty_missing_and_foreign_lines() {
    let t = Tmp::new("cl_empty");
    let st = new_state();
    // каталога нет
    scan_claude(&st, &t.0);
    assert_eq!(total(&st), Tok::default());
    // пустой файл, файл из пустых строк, мусор, строки без usage, чужие типы, «синтетический» ответ
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    std::fs::write(t.p("projects/a/empty.jsonl"), "").unwrap();
    std::fs::write(t.p("projects/a/blank.jsonl"), "\n\n\n").unwrap();
    let junk = [
        "{\"type\":\"assistant\",\"timestamp\":\"x\",\"message\":{\"usage\":{\"input_tokens\":1}}\n".to_string(), // битый JSON с needle
        "usage без json\n".to_string(),
        format!("{{\"type\":\"user\",\"timestamp\":\"{}\",\"message\":{{\"usage\":{{\"input_tokens\":5}}}}}}\n", ts_ago(10)),
        format!("{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"id\":\"s\",\"model\":\"<synthetic>\",\"usage\":{{\"input_tokens\":9}}}}}}\n", ts_ago(10)),
        format!("{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"id\":\"n\",\"model\":\"m\"}}}}\n", ts_ago(10)), // без usage
        "{\"type\":\"assistant\",\"message\":{\"id\":\"t\",\"model\":\"m\",\"usage\":{\"input_tokens\":4}}}\n".to_string(), // без времени
        "\u{0}\u{1}\u{2} usage\n".to_string(),
    ];
    std::fs::write(t.p("projects/a/junk.jsonl"), junk.concat()).unwrap();
    // не-jsonl и вложенность глубже предела
    std::fs::write(t.p("projects/a/note.txt"), cl_line("z", &ts_ago(10), "m", 1, 1, 1, 1)).unwrap();
    std::fs::create_dir_all(t.p("projects/a/b/c/d/e/f")).unwrap();
    std::fs::write(t.p("projects/a/b/c/d/e/f/deep.jsonl"), cl_line("deep", &ts_ago(10), "m", 1, 1, 1, 1)).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st), Tok::default(), "в этих файлах учитывать нечего");
}

/// Повторное чтение без изменений и дописывание: двойного счёта нет.
#[test]
fn claude_rescan_no_double_count_and_append_incremental() {
    let t = Tmp::new("cl_re");
    std::fs::create_dir_all(t.p("projects/p")).unwrap();
    let f = t.p("projects/p/s.jsonl");
    std::fs::write(&f, cl_line("a", &ts_ago(500), "m", 10, 1, 0, 0)).unwrap();
    let st = new_state();
    for _ in 0..5 {
        scan_claude(&st, &t.0);
        assert_eq!(total(&st).input, 10);
    }
    use std::io::Write;
    let mut fh = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
    fh.write_all(cl_line("b", &ts_ago(400), "m", 5, 2, 0, 0).as_bytes()).unwrap();
    // дописана только половина строки (без «\n») — пока не считается, дочитается позже
    let c = cl_line("c", &ts_ago(300), "m", 7, 3, 0, 0);
    let (a, b) = c.as_bytes().split_at(c.len() / 2);
    fh.write_all(a).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!((total(&st).input, total(&st).output), (15, 3));
    fh.write_all(b).unwrap();
    for _ in 0..3 {
        scan_claude(&st, &t.0);
    }
    assert_eq!((total(&st).input, total(&st).output), (22, 6), "дописанная строка учтена ровно один раз");
}

/// Один ответ в нескольких файлах (историю сессии скопировали) и в одном файле несколько раз — один раз.
#[test]
fn claude_same_answer_in_many_files_counts_once() {
    let t = Tmp::new("cl_dup");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    std::fs::create_dir_all(t.p("projects/b")).unwrap();
    let l = cl_line("same", &ts_ago(100), "m", 10, 1, 0, 0);
    std::fs::write(t.p("projects/a/1.jsonl"), format!("{l}{l}{l}")).unwrap();
    std::fs::write(t.p("projects/b/2.jsonl"), format!("{l}{}", cl_line("only-b", &ts_ago(90), "m", 3, 1, 0, 0))).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 13, "10 один раз + 3");
    // порядок обхода каталогов не влияет на сумму (метаморфическое): повторим с новым состоянием
    let st2 = new_state();
    scan_claude(&st2, &t.0);
    assert_eq!(total(&st2), total(&st));
}

/// Строки без `message.id` не дедуплицируются: идентичные строки считаются каждый раз (граница допущения).
#[test]
fn claude_lines_without_id_are_not_deduplicated_documented() {
    let t = Tmp::new("cl_noid");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    let l =
        format!("{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"model\":\"m\",\"usage\":{{\"input_tokens\":10}}}}}}\n", ts_ago(10));
    std::fs::write(t.p("projects/a/1.jsonl"), format!("{l}{l}")).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 20);
}

/// Файл стал короче (переписан/обрезан) — читается заново: итог = содержимое нового файла, а не сумма старого и нового.
#[test]
fn claude_truncated_file_is_reread_not_added() {
    let t = Tmp::new("cl_trunc");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    let f = t.p("projects/a/s.jsonl");
    let big: String = (0..10).map(|i| cl_line(&format!("old{i}"), &ts_ago(1000), "m", 100, 0, 0, 0)).collect();
    std::fs::write(&f, big).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 1000);
    std::fs::write(&f, cl_line("new0", &ts_ago(10), "m", 7, 0, 0, 0)).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 7);
    // тот же ответ после ротации (копия в новом файле) снова виден, потому что старая запись забыта до сбора «seen»
    std::fs::write(&f, cl_line("new0", &ts_ago(10), "m", 7, 0, 0, 0)).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 7);
}

/// Файл переписан новым содержимым, которое ДЛИННЕЕ прежней позиции чтения: сокращения нет, и программа не замечает
/// замены — читает с середины нового файла. Эталон: сумма новых ответов.
#[test]
#[ignore = "известная ошибка: файл, переписанный более длинным содержимым, не распознаётся как новый (учёт идёт по позиции, без проверки отпечатка начала файла)"]
fn claude_rewritten_longer_file_is_detected() {
    let t = Tmp::new("cl_rewrite");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    let f = t.p("projects/a/s.jsonl");
    std::fs::write(&f, cl_line("old", &ts_ago(1000), "m", 100, 0, 0, 0)).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    let new: String = (0..5).map(|i| cl_line(&format!("n{i}"), &ts_ago(10), "m", 1, 0, 0, 0)).collect();
    std::fs::write(&f, new).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 5, "после замены файла должны быть видны все 5 новых ответов");
}

/// Один ответ потерян, если старый файл исчез, а копия с той же историей уже была прочитана с пропуском повторов:
/// характеристика поведения (граница допущения), без утверждения «ошибка» — файлы сессий не удаляются.
#[test]
fn claude_deleting_original_after_copy_loses_answer_documented() {
    let t = Tmp::new("cl_del");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    let l = cl_line("x", &ts_ago(100), "m", 10, 0, 0, 0);
    std::fs::write(t.p("projects/a/orig.jsonl"), &l).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    std::fs::write(t.p("projects/a/copy.jsonl"), &l).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 10);
    std::fs::remove_file(t.p("projects/a/orig.jsonl")).unwrap();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 0, "ответ числился за исчезнувшим файлом; копия его уже пропустила");
}

/// Строка длиннее LINE_CAP пропускается без накопления в памяти, а следующие считаются.
#[test]
fn claude_oversized_line_skipped_and_next_counted() {
    let t = Tmp::new("cl_big");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    let mut body = String::from("{\"type\":\"assistant\",\"usage\":\"");
    body.push_str(&"x".repeat(LINE_CAP + 1000));
    body.push_str("\"}\n");
    body += &cl_line("after", &ts_ago(10), "m", 11, 0, 0, 0);
    std::fs::write(t.p("projects/a/s.jsonl"), body).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    assert_eq!(total(&st).input, 11);
}

/// Поля токенов нетипичных видов: строка, дробное, отрицательное — молча 0 (учёт не падает, но и не угадывает).
#[test]
fn claude_odd_token_field_types_count_as_zero() {
    let t = Tmp::new("cl_odd");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    let mk = |id: &str, inp: &str| {
        format!("{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{\"id\":\"{id}\",\"model\":\"m\",\"usage\":{{\"input_tokens\":{inp},\"output_tokens\":1}}}}}}\n", ts_ago(10))
    };
    let body = [mk("a", "\"12\""), mk("b", "12.0"), mk("c", "-5"), mk("d", "null"), mk("e", "7")].concat();
    std::fs::write(t.p("projects/a/s.jsonl"), body).unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    assert_eq!((total(&st).input, total(&st).output), (7, 5));
}

/// Переполнение: два ответа по u64::MAX токенов. Сумма не должна «обернуться» в маленькое число (в релизной сборке
/// `+` не проверяется и оборачивается; в отладочной — паника).
#[test]
#[ignore = "известная ошибка: Tok::add / Tok::total складывают u64 без насыщения (в релизе значение оборачивается по модулю 2^64)"]
fn tok_add_does_not_wrap() {
    let mut a = Tok { input: u64::MAX, ..Default::default() };
    a.add(&Tok { input: 2, ..Default::default() });
    assert!(a.input >= u64::MAX - 1, "обернулось: {}", a.input);
    let b = Tok { input: u64::MAX, output: 1, ..Default::default() };
    assert!(b.total() >= u64::MAX - 1, "total обернулся: {}", b.total());
}

/// Деление по местным суткам: ответ за секунду до местной полуночи и в полночь — в разных днях; число считается
/// по `ui::localtime` (независимо от `local_day`).
#[test]
fn claude_local_midnight_split() {
    let now = crate::ui::now_secs();
    // ближайшая прошедшая местная полночь: идём назад по минутам, пока не увидим 00:00:xx
    let mut m = now - now % 60;
    while localtime(m).hour != 0 || localtime(m).min != 0 {
        m -= 60;
    }
    let midnight = (m - 3600..=m).find(|t| localtime(*t).hour == 0 && localtime(*t).min == 0 && localtime(*t).sec == 0).unwrap();
    let before = localtime(midnight - 1);
    let after = localtime(midnight);
    let iso = |t: i64| ts_from(t);
    let t = Tmp::new("cl_mid");
    std::fs::create_dir_all(t.p("projects/a")).unwrap();
    std::fs::write(
        t.p("projects/a/s.jsonl"),
        [cl_line("a", &iso(midnight - 1), "m", 1, 0, 0, 0), cl_line("b", &iso(midnight), "m", 2, 0, 0, 0)].concat(),
    )
    .unwrap();
    let st = new_state();
    scan_claude(&st, &t.0);
    let us = sum_usage(&st);
    let key = |l: crate::ui::Tm| (l.year as i32) * 10_000 + l.mon * 100 + l.mday;
    assert_eq!(us.get(&(key(before), "m".to_string())).map(|t| t.input), Some(1));
    assert_eq!(us.get(&(key(after), "m".to_string())).map(|t| t.input), Some(2));
}

/// Формат журнала из Unix-времени (наивно, как `ts_ago`).
fn ts_from(t: i64) -> String {
    let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (mut y, mut d) = (1970i64, days);
    let leap = |y: i64| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    while d >= if leap(y) { 366 } else { 365 } {
        d -= if leap(y) { 366 } else { 365 };
        y += 1;
    }
    let ml = [31, if leap(y) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    while d >= ml[m] {
        d -= ml[m];
        m += 1;
    }
    format!("{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", m + 1, d + 1, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// «Сегодня» и «за 7 дней»: границы окна — сегодня и сегодня-6 включительно; сумма в строке совпадает с первичными данными.
#[test]
fn usage_row_window_boundaries_and_primary_match() {
    let now = crate::ui::now_secs();
    // события по одному в середине суток каждого из последних 10 суток: токенов (i+1)*10
    let mut us = Usage::new();
    let mut exp_today = 0;
    let mut exp_week = 0;
    for back in 0..10i64 {
        let t = now - back * 86_400;
        let n = (back as u64 + 1) * 10;
        us.entry((local_day(t), "m".into())).or_default().add(&Tok { input: n, ..Default::default() });
        if back == 0 {
            exp_today += n;
        }
        if back <= 6 {
            exp_week += n;
        }
    }
    assert_eq!(exp_week, (1..=7).map(|i| i * 10).sum::<u64>());
    assert_eq!(sums(&us, now), (exp_today, exp_week));
    let row = usage_row("A", &us, now).unwrap();
    assert_eq!(row.value, format!("сегодня {} · за 7 дней {}", fmt_tokens(exp_today), fmt_tokens(exp_week)));
    assert!(usage_row("A", &Usage::new(), now).is_none());
    // всё старше недели — строки нет
    let mut old = Usage::new();
    old.entry((local_day(now - 8 * 86_400), "m".into())).or_default().add(&Tok { input: 5, ..Default::default() });
    assert!(usage_row("A", &old, now).is_none());
}

/// Несколько одновременных «сессий» пишут в свои файлы (и часть ответов — общие), пока читатель сканирует: суммы
/// монотонны и в конце равны ручному подсчёту; ничего не считается дважды.
#[test]
fn claude_parallel_writers_and_scanner() {
    let t = Tmp::new("cl_par");
    for i in 0..4 {
        std::fs::create_dir_all(t.p(&format!("projects/s{i}"))).unwrap();
    }
    let writers = 4usize;
    let per = 150usize;
    let root = t.0.clone();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut hs = Vec::new();
    for w in 0..writers {
        let (root, done) = (root.clone(), done.clone());
        hs.push(std::thread::spawn(move || {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(root.join(format!("projects/s{w}/log.jsonl"))).unwrap();
            for i in 0..per {
                // свой ответ (1 токен) и общий для всех «shared-i» (по 1000 токенов на ответ; в каждом файле один и тот же)
                f.write_all(cl_line(&format!("own-{w}-{i}"), &ts_ago(60), "m", 1, 0, 0, 0).as_bytes()).unwrap();
                f.write_all(cl_line(&format!("shared-{i}"), &ts_ago(60), "m", 1000, 0, 0, 0).as_bytes()).unwrap();
                if i % 20 == 0 {
                    std::thread::yield_now();
                }
            }
            done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
    }
    let st = new_state();
    let mut last = 0;
    let t0 = std::time::Instant::now();
    while done.load(std::sync::atomic::Ordering::SeqCst) < writers && t0.elapsed().as_secs() < 60 {
        scan_claude(&st, &t.0);
        let now_total = total(&st).input;
        assert!(now_total >= last, "сумма упала: {last} -> {now_total}");
        // верхняя граница: ни при каком промежуточном состоянии не больше итога
        assert!(now_total <= (writers * per) as u64 + 1000 * per as u64, "{now_total}");
        last = now_total;
    }
    for h in hs {
        h.join().unwrap();
    }
    scan_claude(&st, &t.0);
    scan_claude(&st, &t.0);
    // свои: writers*per*1; общие: per*1000 (каждый один раз, несмотря на 4 копии)
    assert_eq!(total(&st).input, (writers * per) as u64 + per as u64 * 1000);
}

/// Метаморфическое/случайное: те же ответы, разложенные по файлам случайно (и с дублями), дают ту же сумму.
#[test]
fn claude_totals_independent_of_file_layout() {
    for_seeds("cl_layout", |r| {
        let n = r.range(1, 40) as usize;
        let answers: Vec<(String, u64, u64, u64, u64)> =
            (0..n).map(|i| (format!("id{i}"), r.below(1000), r.below(1000), r.below(1000), r.below(1000))).collect();
        let want = answers.iter().fold(Tok::default(), |mut a, x| {
            a.add(&Tok { input: x.1, output: x.2, cache_read: x.3, cache_write: x.4 });
            a
        });
        let t = Tmp::new("cl_lay");
        let files = r.range(1, 5) as usize;
        let mut bodies = vec![String::new(); files];
        for (id, i, o, cr, cw) in &answers {
            // каждый ответ — в 1..=files случайных файлов (дубли), хотя бы в одном
            let copies = r.range(1, files as i64) as usize;
            let start = r.below(files as u64) as usize;
            for c in 0..copies {
                bodies[(start + c) % files] += &cl_line(id, &ts_ago(120), "m", *i, *o, *cr, *cw);
            }
        }
        std::fs::create_dir_all(t.p("projects/a")).unwrap();
        for (k, b) in bodies.iter().enumerate() {
            std::fs::write(t.p(&format!("projects/a/f{k}.jsonl")), b).unwrap();
        }
        let st = new_state();
        scan_claude(&st, &t.0);
        assert_eq!(total(&st), want);
        // повторный проход ничего не меняет
        scan_claude(&st, &t.0);
        assert_eq!(total(&st), want);
    });
}

/// Две корневые папки Claude: настоящая `~/.claude` и XDG-папка, одна — ссылка на другую. Ответы не удваиваются.
#[cfg(unix)]
#[test]
fn claude_dirs_symlinked_roots_do_not_double() {
    let t = Tmp::new("cl_sym");
    std::fs::create_dir_all(t.p("home/.claude/projects/a")).unwrap();
    std::fs::create_dir_all(t.p("home/.config")).unwrap();
    std::os::unix::fs::symlink(t.p("home/.claude"), t.p("home/.config/claude")).unwrap();
    std::fs::write(t.p("home/.claude/projects/a/s.jsonl"), cl_line("x", &ts_ago(10), "m", 9, 0, 0, 0)).unwrap();
    let dirs = claude_dirs_from(None, Some(t.p("home")), None);
    assert_eq!(dirs.len(), 2);
    let roots: Vec<PathBuf> = dirs.iter().map(|d| d.join("projects")).collect();
    let st = new_state();
    scan(&st, &roots, 4, &[b"\"usage\""], &mut claude_line);
    assert_eq!(total(&st).input, 9);
}

/// Окружение: относительные CLAUDE_CONFIG_DIR и XDG_CONFIG_HOME игнорируются; абсолютные работают.
#[test]
fn claude_dirs_env_validation() {
    with_fake_home("cl_env", |home| {
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::create_dir_all(home.join("x/claude")).unwrap();
        std::env::set_var("CLAUDE_CONFIG_DIR", "relative/dir");
        // относительная CLAUDE_CONFIG_DIR отброшена → обычные папки
        assert_eq!(claude_dirs(), vec![home.join(".claude")]);
        std::env::set_var("CLAUDE_CONFIG_DIR", home.join("x/claude"));
        assert_eq!(claude_dirs(), vec![home.join("x/claude")]);
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::set_var("XDG_CONFIG_HOME", "rel");
        assert_eq!(claude_dirs(), vec![home.join(".claude")], "относительная XDG_CONFIG_HOME → ~/.config");
        std::env::set_var("XDG_CONFIG_HOME", home.join("x"));
        assert_eq!(claude_dirs(), vec![home.join(".claude"), home.join("x/claude")]);
    });
}

// ───────────── Codex ─────────────

fn cx_ctx(model: &str, cwd: &str) -> String {
    format!("{{\"type\":\"turn_context\",\"payload\":{{\"model\":\"{model}\",\"cwd\":\"{cwd}\"}}}}\n")
}
#[allow(clippy::too_many_arguments)]
fn cx_tok(ts: &str, inp: u64, cached: u64, out: u64, total_in: u64, total_out: u64, p5: f64, p7: f64, rs5: i64, rs7: i64) -> String {
    format!(
        "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":{total_in},\"output_tokens\":{total_out}}},\"last_token_usage\":{{\"input_tokens\":{inp},\"cached_input_tokens\":{cached},\"output_tokens\":{out}}}}},\"rate_limits\":{{\"primary\":{{\"used_percent\":{p5},\"window_minutes\":300,\"resets_at\":{rs5}}},\"secondary\":{{\"used_percent\":{p7},\"window_minutes\":10080,\"resets_at\":{rs7}}}}}}}}}\n"
    )
}
fn scan_codex(st: &Mutex<Option<Files>>, root: &Path) {
    scan(st, &[root.join("sessions")], 4, &[b"token_count", b"turn_context"], &mut codex_line);
}

/// Схема Codex: input включает кэш; «новые на входе» = input − cached, кэш отдельно. Ручной подсчёт.
#[test]
fn codex_schema_input_includes_cache() {
    let t = Tmp::new("cx");
    std::fs::create_dir_all(t.p("sessions/2026/11/15")).unwrap();
    let mut b = cx_ctx("gpt-x", "/w/proj");
    b += &cx_tok(&ts_ago(500), 1000, 600, 50, 1000, 50, 10.0, 5.0, crate::ui::now_secs() + 3_000, crate::ui::now_secs() + 80_000);
    b += &cx_tok(&ts_ago(400), 2000, 1500, 70, 3000, 120, 12.0, 6.0, crate::ui::now_secs() + 3_000, crate::ui::now_secs() + 80_000);
    std::fs::write(t.p("sessions/2026/11/15/r.jsonl"), b).unwrap();
    let st = new_state();
    scan_codex(&st, &t.0);
    // новые: (1000-600)+(2000-1500)=900; ответ 120; кэш 2100
    assert_eq!(total(&st), Tok { input: 900, output: 120, cache_read: 2100, cache_write: 0 });
    // лимиты берутся из самого свежего события, а не складываются
    let l = st.lock().unwrap().as_ref().unwrap().values().filter_map(|s| s.limits).max_by_key(|l| l.at).unwrap();
    assert_eq!((l.five.unwrap().pct, l.week.unwrap().pct), (12.0, 6.0));
    // проекты: токены ушли в папку проекта
    assert_eq!(sum_projects(&st).values().sum::<u64>(), 900 + 120 + 2100);
    assert_eq!(sum_projects(&st).keys().next().unwrap().1, "proj");
}

/// Кэш больше входа (битые данные): вычитание насыщается нулём, итог не «минус»; кэш всё равно учтён.
#[test]
fn codex_cached_larger_than_input_saturates() {
    let t = Tmp::new("cx_sat");
    std::fs::create_dir_all(t.p("sessions/a")).unwrap();
    std::fs::write(t.p("sessions/a/r.jsonl"), cx_tok(&ts_ago(5), 10, 50, 1, 10, 1, 1.0, 1.0, 1, 1)).unwrap();
    let st = new_state();
    scan_codex(&st, &t.0);
    assert_eq!(total(&st), Tok { input: 0, output: 1, cache_read: 50, cache_write: 0 });
}

/// Журнал Codex повторяет событие token_count с тем же `total_token_usage` (так бывает, когда обновились только
/// лимиты); сумма `last_token_usage` по всем событиям завышает расход. На настоящем журнале (проверено только по
/// числам, без содержимого) сумма событий дала +4% по входу и +22% по ответам относительно итога сессии.
/// Эталон: итоговый `total_token_usage` последнего события.
#[test]
#[ignore = "известная ошибка: повтор события token_count (то же total_token_usage) считается дважды — расход Codex завышен"]
fn codex_repeated_token_count_event_counts_once() {
    let t = Tmp::new("cx_dup");
    std::fs::create_dir_all(t.p("sessions/a")).unwrap();
    let e = cx_tok(&ts_ago(100), 1000, 0, 100, 1000, 100, 1.0, 1.0, crate::ui::now_secs() + 3_000, crate::ui::now_secs() + 80_000);
    std::fs::write(t.p("sessions/a/r.jsonl"), format!("{e}{e}")).unwrap();
    let st = new_state();
    scan_codex(&st, &t.0);
    assert_eq!((total(&st).input, total(&st).output), (1000, 100), "итог сессии — 1000/100, событие продублировано");
}

/// Лимиты Codex: старое событие, пришедшее позже по файлу, не затирает более свежее (по времени).
#[test]
fn codex_limits_latest_by_timestamp_not_file_order() {
    let t = Tmp::new("cx_lim");
    std::fs::create_dir_all(t.p("sessions/a")).unwrap();
    let mut b = cx_tok(&ts_ago(100), 1, 0, 1, 1, 1, 50.0, 10.0, crate::ui::now_secs() + 3_000, crate::ui::now_secs() + 80_000);
    b += &cx_tok(&ts_ago(5000), 1, 0, 1, 2, 2, 99.0, 99.0, crate::ui::now_secs() + 3_000, crate::ui::now_secs() + 80_000);
    std::fs::write(t.p("sessions/a/r.jsonl"), b).unwrap();
    let st = new_state();
    scan_codex(&st, &t.0);
    let l = st.lock().unwrap().as_ref().unwrap().values().filter_map(|s| s.limits).next().unwrap();
    assert_eq!(l.five.unwrap().pct, 50.0);
}

// ───────────── копия лимитов Claude и строка состояния ─────────────

#[test]
fn claude_limits_saved_copy_fallback_chain() {
    with_fake_home("cl_lim", |_| {
        let body = |p: u32| format!("{{\"rate_limits\":{{\"five_hour\":{{\"used_percentage\":{p},\"resets_at\":1000}}}},\"at\":7}}");
        // ни сессии, ни копии
        assert_eq!(claude_limits(), None);
        // только копия (после перезагрузки папка сеанса пуста)
        crate::store::write_private_quick(&claude_limits_saved_path(), body(11).as_bytes()).unwrap();
        assert_eq!(claude_limits().unwrap().five.unwrap().pct, 11.0);
        // свежие из папки сеанса важнее копии
        crate::store::write_private_quick(&claude_limits_path(), body(22).as_bytes()).unwrap();
        assert_eq!(claude_limits().unwrap().five.unwrap().pct, 22.0);
        // файл сеанса испорчен — откат на копию, а не «нет данных»
        crate::store::write_private_quick(&claude_limits_path(), b"{\"rate_limits\":").unwrap();
        assert_eq!(claude_limits().unwrap().five.unwrap().pct, 11.0);
        // оба испорчены — None (честное «неизвестно», не нули)
        crate::store::write_private_quick(&claude_limits_saved_path(), b"\xff\xfe").unwrap();
        assert_eq!(claude_limits(), None);
        // файл-ссылка и огромный файл не читаются
        let big = vec![b' '; 10_000];
        crate::store::write_private_quick(&claude_limits_path(), &big).unwrap();
        assert_eq!(claude_limits(), None);
    });
}

/// Права файлов с лимитами — только владелец; в копии нет ничего, кроме `rate_limits` и `at` (проверяется чтением
/// записанного файла). Запись самой строки состояния проверяет e2e-скрипт `tools/regress_e2e.py` на настоящем бинарнике.
#[cfg(unix)]
#[test]
fn claude_limits_files_are_private() {
    with_fake_home("cl_perm", |_| {
        let p = claude_limits_saved_path();
        crate::store::write_private_quick(&p, b"{}").unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    });
}

/// Свойство: разбор лимитов никогда не паникует и не возвращает NaN/∞/процент вне 0..100 на случайном JSON.
#[test]
fn parse_claude_limits_fuzz_invariants() {
    for_seeds("lim_fuzz", |r| {
        let vals = [
            "0",
            "1",
            "-1",
            "99.9",
            "100",
            "1e999",
            "-1e999",
            "\"7\"",
            "null",
            "true",
            "[]",
            "{}",
            "1e-320",
            "18446744073709551615",
            "9223372036854775808",
        ];
        let keys = ["used_percentage", "resets_at", "x"];
        let mut s = String::from("{\"rate_limits\":{");
        let mut parts = Vec::new();
        for w in ["five_hour", "seven_day", "other"] {
            if r.chance(70) {
                let mut inner: Vec<String> = Vec::new();
                for k in keys {
                    if r.chance(80) {
                        inner.push(format!("\"{k}\":{}", r.pick(&vals)));
                    }
                }
                parts.push(format!("\"{w}\":{{{}}}", inner.join(",")));
            }
        }
        s += &parts.join(",");
        s += "},\"at\":";
        s += r.pick(&vals);
        s += "}";
        if let Some(l) = parse_claude_limits(&s) {
            for w in [l.five, l.week].into_iter().flatten() {
                assert!(w.pct.is_finite() && (0.0..=100.0).contains(&w.pct), "{s} -> {w:?}");
                let _ = w.now_pct(0);
                let _ = until(w.resets_at, 5);
                let _ = limit_row("A", "x", &w, l.at, 5);
            }
        }
    });
}

/// Длительное испытание учёта токенов: растущие и ротируемые журналы, несколько писателей; каждые 5 с — CSV
/// (время, RSS КБ, потоки, дескрипторы, длительность прохода мс, верно ли). Запуск:
///   REGRESS_SOAK_SECS=3600 REGRESS_CSV=/tmp/soak_limits.csv cargo test --release soak_limits -- --ignored --nocapture
#[test]
#[ignore = "длительное испытание: см. REGRESSION_TESTS.md"]
fn soak_limits_growing_logs() {
    let secs = soak_secs(60);
    let t = Tmp::new("soak_lim");
    for i in 0..4 {
        std::fs::create_dir_all(t.p(&format!("projects/s{i}"))).unwrap();
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let written = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)); // сумма input, записанная писателями (эталон)
    let mut hs = Vec::new();
    for w in 0..4 {
        let (root, stop, written) = (t.0.clone(), stop.clone(), written.clone());
        hs.push(std::thread::spawn(move || {
            use std::io::Write;
            let path = root.join(format!("projects/s{w}/log.jsonl"));
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
            let mut n = 0u64;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                f.write_all(cl_line(&format!("w{w}-{n}"), &ts_ago(30), "m", 3, 1, 0, 0).as_bytes()).unwrap();
                written.fetch_add(3, std::sync::atomic::Ordering::SeqCst);
                n += 1;
                if n % 5000 == 0 {
                    // ротация: старый файл переименовывается, новый начинается заново (ответы уже учтены по id)
                    drop(f);
                    let _ = std::fs::rename(&path, root.join(format!("projects/s{w}/old{n}.jsonl.bak")));
                    f = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }));
    }
    csv_line("t_s,rss_kb,threads,fds,scan_ms,total_input,written_input,ok");
    let st = new_state();
    let t0 = std::time::Instant::now();
    let mut next_report = 0u64;
    let mut worst = 0u128;
    while t0.elapsed().as_secs() < secs {
        let s = std::time::Instant::now();
        scan_claude(&st, &t.0);
        let ms = s.elapsed().as_millis();
        worst = worst.max(ms);
        if t0.elapsed().as_secs() >= next_report {
            let (rss, th, fd) = proc_stats();
            let (got, w) = (total(&st).input, written.load(std::sync::atomic::Ordering::SeqCst));
            // читатель отстаёт от писателей не больше чем на то, что дописано за проход
            csv_line(&format!("{},{rss},{th},{fd},{ms},{got},{w},{}", t0.elapsed().as_secs(), (got <= w) as u8));
            assert!(got <= w, "учтено больше, чем записано: {got} > {w}");
            next_report += 5;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for h in hs {
        h.join().unwrap();
    }
    scan_claude(&st, &t.0);
    // ротированные .bak не *.jsonl — не читаются; итог равен записанному, если все записанные файлы видны
    let (got, w) = (total(&st).input, written.load(std::sync::atomic::Ordering::SeqCst));
    csv_line(&format!("final,,,,{worst},{got},{w},{}", (got <= w) as u8));
    assert!(got <= w);
}
