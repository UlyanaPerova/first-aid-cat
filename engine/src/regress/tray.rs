//! Регрессионные тесты плашки лимитов и значка (`tray.rs`): подписи «сбросится / хватит до», выбор агента и окна,
//! подписи рядом со значком, слежение (уведомления и прогноз), разжимание сцены. Дочерний модуль `tray::regress`.
//!
//! Не покрыто здесь (нужны настоящая панель, D-Bus и расширение GNOME или macOS): обмен с расширением
//! (`PanelOpen`, кадры сценки), `mod mac`, `mod win`. См. REGRESSION_TESTS.md, раздел «Непроверенные сценарии».
#![allow(clippy::manual_is_multiple_of)] // is_multiple_of стабилен с Rust 1.90, а проект собирается с 1.89

use super::*;
use crate::limits::{Limits, Window as Win};
use crate::regress_common::*;

const RED: u32 = 0xB3001B;
const GREEN: u32 = 0x0B7A0B;

// ───────────── limit_line ─────────────

/// Границы «сбросится» (осталась пятая часть окна или меньше): 5 ч → 3600 с, неделя 10080 мин → 120 960 с (33,6 ч).
/// Выкладка: минуты × 60 / 5 = 300 × 12 = 3600; 10080 × 12 = 120 960.
#[test]
fn limit_line_good_news_boundaries() {
    let now = 1_800_000_000;
    let w = |dt: i64, min: u32| limit_line(None, now + dt, min, now);
    assert_eq!(w(3600, 300).0, "сбросится");
    assert_eq!(w(3601, 300).0, "сброс");
    assert_eq!(w(1, 300).0, "сбросится");
    assert_eq!(w(120_960, 10_080).0, "сбросится");
    assert_eq!(w(120_961, 10_080).0, "сброс");
    // длина окна неизвестна (0) — «сбросится» невозможно (порог 0), остаётся нейтральное «сброс»
    assert_eq!(w(10, 0).0, "сброс");
    // цвета
    assert_eq!(w(10, 300).2, GREEN);
    assert_eq!(w(7200, 300).2, GREY);
}

/// Граница «хватит до ≈»: красным — только если лимит кончится БОЛЬШЕ ЧЕМ за 10 минут до сброса.
#[test]
fn limit_line_bad_news_boundary_ten_minutes() {
    let now = 1_800_000_000;
    let resets = now + 5 * 3600;
    // eta + 600 == resets → не красное (строго меньше)
    assert_ne!(limit_line(Some(resets - 600), resets, 300, now).0, "хватит до ≈");
    // eta + 601 < resets не выполнено: resets − 601 + 600 < resets → true → красное
    let (w, t, c) = limit_line(Some(resets - 601), resets, 300, now);
    assert_eq!((w.as_str(), c), ("хватит до ≈", RED));
    assert!(!t.is_empty());
    // красное важнее «сбросится»: сброс через 30 минут, а лимит кончится через 5
    assert_eq!(limit_line(Some(now + 300), now + 1800, 300, now).0, "хватит до ≈");
    // время сброса неизвестно (0) и есть прогноз — красное
    assert_eq!(limit_line(Some(now + 300), 0, 300, now).0, "хватит до ≈");
}

/// Окно уже сбросилось и новых данных ещё нет: старый прогноз «хватит до ≈ <прошедшее время>» красным не должен
/// показываться — окно свободно. Между сбросом и следующим обновлением данных (десятки секунд) плашка так и рисуется:
/// `limit_line` не сверяет прогноз с моментом сброса.
#[test]
#[ignore = "известная ошибка: после сброса окна (resets_at ≤ now) прогноз ещё красным «хватит до ≈ …» вместо «уже сброшен» до следующего обновления данных"]
fn limit_line_stale_forecast_after_reset_is_not_red() {
    let now = 1_800_000_000;
    let (w, t, c) = limit_line(Some(now - 7200), now - 60, 300, now);
    assert_eq!((w.as_str(), t.as_str(), c), ("уже сброшен", "", GREY));
}

#[test]
fn limit_line_week_time_always_names_the_day() {
    let now = 1_800_000_000;
    for dt in [100, 3 * 3600, 30 * 3600, 5 * 86_400] {
        let (_, t, _) = limit_line(None, now + dt, 10_080, now);
        assert!(
            t.starts_with("сегодня в ")
                || t.starts_with("завтра в ")
                || t.contains(" в ") && t.split(' ').next().is_some_and(|d| ["пн", "вт", "ср", "чт", "пт", "сб", "вс"].contains(&d)),
            "{t}"
        );
    }
}

// ───────────── Watch: уведомления и прогноз ─────────────

fn ag(name: &'static str, five: Option<(f64, i64)>, at: i64) -> Agent {
    Agent { name, limits: Some(Limits { five: five.map(|(p, r)| Win { pct: p, resets_at: r, minutes: 300 }), week: None, at }), ..Default::default() }
}

/// Случайная траектория занятости одного окна: пока окно не сбрасывается, предупреждения 80% и 95% приходят не чаще
/// одного раза каждое, в порядке роста; первая увиденная цифра только запоминается. Оракул — отдельный подсчёт «максимального
/// уровня».
#[test]
fn watch_alerts_each_level_once_per_window() {
    for_seeds("watch_alerts", |r| {
        let mut w = Watch::default();
        let n = r.range(2, 40);
        let mut pct = r.unit() * 100.0;
        let mut now = 1_000;
        let mut max_lv = 0u8; // 0/80/95 — самый высокий уровень, о котором уже предупреждали (по первой цифре)
        let lv = |p: f64| {
            if p >= 95.0 {
                95
            } else if p >= 80.0 {
                80
            } else {
                0
            }
        };
        let mut expect_alerts = 0;
        let mut got_alerts = 0;
        for i in 0..n {
            // только рост (сброса нет)
            if i > 0 {
                pct = (pct + r.unit() * 15.0).min(100.0);
            }
            now += 60;
            let out = w.update(&[ag("Claude Code", Some((pct, 1_000_000)), now)], now);
            got_alerts += out.len();
            if i == 0 {
                max_lv = lv(pct); // первую цифру только запоминаем
            } else if lv(pct) > max_lv {
                expect_alerts += 1;
                max_lv = lv(pct);
            }
        }
        assert_eq!(got_alerts, expect_alerts, "уведомлений {got_alerts}, ожидалось {expect_alerts}");
    });
}

/// Метаморфическое: повтор тех же цифр (то же время) не даёт ни одного нового уведомления и не меняет прогноз.
#[test]
fn watch_update_is_idempotent_for_repeated_data() {
    let mut w = Watch::default();
    w.update(&[ag("Claude Code", Some((40.0, 99_999)), 1_000)], 1_000);
    w.update(&[ag("Claude Code", Some((90.0, 99_999)), 2_200)], 2_200);
    let eta = w.runs_out("Claude Code", 0);
    for _ in 0..5 {
        assert!(w.update(&[ag("Claude Code", Some((90.0, 99_999)), 2_200)], 2_200).is_empty());
        assert_eq!(w.runs_out("Claude Code", 0), eta);
    }
}

/// Сброс окна: процент заметно упал → «снова свободен» (только если было ≥80%) и уровни обнуляются; падение меньше 5 п.п. —
/// не сброс.
#[test]
fn watch_reset_detection_thresholds() {
    let mut w = Watch::default();
    w.update(&[ag("A", Some((85.0, 10_000)), 100)], 100);
    // −4,9 п.п. без смены времени сброса — не сброс (шум округления)
    assert!(w.update(&[ag("A", Some((80.1, 10_000)), 200)], 200).is_empty());
    // −6 п.п., сброс сдвинулся → сброс: было ≥ 80 → «снова свободен»
    let o = w.update(&[ag("A", Some((74.0, 50_000)), 300)], 300);
    assert_eq!(o.len(), 1);
    assert!(o[0].0.contains("снова свободен"));
    // окно с малым процентом сбросилось — без уведомления (раньше и не доходило до 80)
    let mut w = Watch::default();
    w.update(&[ag("B", Some((30.0, 10_000)), 100)], 100);
    assert!(w.update(&[ag("B", Some((1.0, 50_000)), 200)], 200).is_empty());
}

/// Прогноз «когда кончится»: ручной расчёт. 40% → 50% за 1200 с: темп 10/1200 %/с; до 100% осталось 50 → 6000 с.
/// Нужно ≥ 10 минут наблюдений, рост, и чтобы кончилось ДО сброса.
#[test]
fn watch_forecast_hand_calculation_and_guards() {
    let mut w = Watch::default();
    w.update(&[ag("A", Some((40.0, 100_000)), 1_000)], 1_000);
    w.update(&[ag("A", Some((50.0, 100_000)), 2_200)], 2_200);
    assert_eq!(w.runs_out("A", 0), Some(2_200 + 6_000));
    // меньше 10 минут наблюдений — прогноза нет
    let mut w = Watch::default();
    w.update(&[ag("A", Some((40.0, 100_000)), 1_000)], 1_000);
    w.update(&[ag("A", Some((50.0, 100_000)), 1_599)], 1_599);
    assert_eq!(w.runs_out("A", 0), None);
    // ровно 600 с — уже есть
    w.update(&[ag("A", Some((50.0, 100_000)), 1_600)], 1_600);
    assert!(w.runs_out("A", 0).is_some());
    // темпа нет (не растёт) и 100% — прогноза нет
    let mut w = Watch::default();
    w.update(&[ag("A", Some((50.0, 100_000)), 1_000)], 1_000);
    w.update(&[ag("A", Some((50.0, 100_000)), 5_000)], 5_000);
    assert_eq!(w.runs_out("A", 0), None);
    let mut w = Watch::default();
    w.update(&[ag("A", Some((90.0, 100_000)), 1_000)], 1_000);
    w.update(&[ag("A", Some((100.0, 100_000)), 5_000)], 5_000);
    assert_eq!(w.runs_out("A", 0), None);
    // неизвестный агент / окно
    assert_eq!(w.runs_out("нет", 0), None);
    assert_eq!(w.runs_out("A", 1), None);
}

/// Бесконечно малый рост за час: темп ≈ 2·10⁻¹⁸ %/с, «до 100%» → 2,5·10¹⁹ с — больше i64. В релизной сборке сложение
/// переполняется и прогноз становится далёким ПРОШЛЫМ (красная надпись «хватит до ≈ 1969…»).
#[test]
#[ignore = "известная ошибка: runs_out при ничтожном росте переполняет i64 (eta < t1), показывая красный прогноз из прошлого"]
fn watch_forecast_tiny_growth_does_not_overflow() {
    let mut w = Watch::default();
    let p1 = f64::from_bits(50.0f64.to_bits() + 1); // следующее число после 50,0
    w.update(&[ag("A", Some((50.0, 0)), 0)], 0);
    w.update(&[ag("A", Some((p1, 0)), 3_600)], 3_600);
    match w.runs_out("A", 0) {
        None => {}
        Some(eta) => assert!(eta > 3_600, "прогноз из прошлого: {eta}"),
    }
}

/// Случайные последовательности без паники: время идёт назад, агенты исчезают и появляются, окна без процента.
#[test]
fn watch_update_random_sequences_never_panic() {
    for_seeds("watch_fuzz", |r| {
        let mut w = Watch::default();
        let mut now = 5_000i64;
        for _ in 0..r.range(1, 60) {
            now += r.range(-500, 4_000);
            let mut v = Vec::new();
            for name in ["Claude Code", "Codex"] {
                if r.chance(70) {
                    let five = r.chance(80).then(|| (r.unit() * 100.0, now + r.range(-100, 20_000)));
                    let week = r.chance(50).then(|| Win { pct: r.unit() * 100.0, resets_at: now + r.range(-100, 900_000), minutes: 10_080 });
                    let at = if r.chance(10) { 0 } else { now + r.range(-50, 50) };
                    v.push(Agent {
                        name,
                        limits: Some(Limits { five: five.map(|(p, rs)| Win { pct: p, resets_at: rs, minutes: 300 }), week, at }),
                        ..Default::default()
                    });
                }
            }
            let _ = w.update(&v, now);
            for name in ["Claude Code", "Codex"] {
                for which in 0..2 {
                    if let Some(eta) = w.runs_out(name, which) {
                        let _ = limit_line(Some(eta), now + 100, 300, now);
                    }
                }
            }
        }
    });
}

// ───────────── подписи, выбор агента и окна ─────────────

fn agent_with(name: &'static str, five: Option<f64>, week: Option<f64>, now: i64) -> Agent {
    Agent {
        name,
        limits: Some(Limits {
            five: five.map(|p| Win { pct: p, resets_at: now + 1000, minutes: 300 }),
            week: week.map(|p| Win { pct: p, resets_at: now + 100_000, minutes: 10_080 }),
            at: now,
        }),
        today: 1_500_000,
        week: 9_000_000,
        days: vec![0; 14],
    }
}

/// Таблица подписей (число рядом со значком): окно 0 — самый заполненный, 1 — 5 ч, 2 — неделя; выбор 0 — одно число,
/// 1 — у каждого агента своё. Округление: «{:.0}» — до ближайшего чётного на половинках (Python '%.0f': 0.5→0, 1.5→2, 2.5→2).
#[test]
fn label_table() {
    let now = 5_000;
    let cl = agent_with("Claude Code", Some(10.0), Some(40.0), now);
    let co = agent_with("Codex", Some(64.5), Some(2.5), now);
    let both = [cl.clone(), co.clone()];
    let sp = "\u{2002}";
    // одно число, самый заполненный из двух окон у всех: max(10,40,64.5,2.5)=64.5 → «64» (половинка → чётное)
    assert_eq!(label_with(&both, now, 0, 0), format!("{sp}64%"));
    // только 5 часов: max(10, 64.5)
    assert_eq!(label_with(&both, now, 0, 1), format!("{sp}64%"));
    // только неделя: max(40, 2.5)
    assert_eq!(label_with(&both, now, 0, 2), format!("{sp}40%"));
    // у каждого своё: «Cl 40%  Co 64%» (окно 0); разделитель — широкий пробел
    assert_eq!(label_with(&both, now, 1, 0), format!("{sp}Cl 40%{sp}Co 64%"));
    assert_eq!(label_with(&both, now, 1, 2), format!("{sp}Cl 40%{sp}Co 2%"));
    // окно без данных у агента — его в подписи нет; нет данных ни у кого — пусто
    let no_week = agent_with("Codex", Some(5.0), None, now);
    assert_eq!(label_with(std::slice::from_ref(&no_week), now, 1, 2), "");
    assert_eq!(label_with(&[], now, 1, 0), "");
    // округление половинок
    for (p, want) in [(0.5, "0"), (1.5, "2"), (2.5, "2"), (99.5, "100"), (99.49, "99"), (100.0, "100"), (0.0, "0")] {
        let a = agent_with("Claude Code", Some(p), None, now);
        assert_eq!(label_with(std::slice::from_ref(&a), now, 0, 1), format!("{sp}{want}%"), "pct {p}");
    }
    // окно уже сброшено — 0%, а не старый процент
    let mut old = agent_with("Claude Code", Some(90.0), None, now);
    old.limits.as_mut().unwrap().five.as_mut().unwrap().resets_at = now - 1;
    assert_eq!(label_with(std::slice::from_ref(&old), now, 0, 1), format!("{sp}0%"));
}

#[test]
fn tooltip_table() {
    let now = 5_000;
    assert_eq!(tooltip(&[], now), "Лимиты ИИ: пока нет данных");
    let a = agent_with("Claude Code", Some(42.0), None, now);
    assert_eq!(tooltip(std::slice::from_ref(&a), now), "Claude Code: 5 ч 42% · сегодня 1,5 млн");
    let none = Agent { name: "Codex", limits: None, today: 812, ..Default::default() };
    assert_eq!(tooltip(&[a, none], now), "Claude Code: 5 ч 42% · сегодня 1,5 млн\nCodex: сегодня 812");
}

/// Настройки значка: каждая комбинация сохранённых значений читается в допустимый выбор, и «хотя бы за одним агентом
/// следим» выполняется всегда. Папка настроек — временная.
#[test]
fn settings_choices_table_and_invariants() {
    with_fake_home("tray_prefs", |_| {
        let set = |k: &str, v: serde_json::Value| crate::prefs::update(|p| p[k] = v);
        // agents_choice: список СНЯТЫХ агентов
        set("limits_hide", serde_json::json!([]));
        assert_eq!(agents_choice(), 0);
        set("limits_hide", serde_json::json!(["Claude Code"]));
        assert_eq!(agents_choice(), 2, "снят Claude — следим за Codex (2)");
        set("limits_hide", serde_json::json!(["Codex"]));
        assert_eq!(agents_choice(), 1);
        set("limits_hide", serde_json::json!(["Claude Code", "Codex"]));
        assert_eq!(agents_choice(), 0, "сняты оба — значит, за всеми");
        set("limits_hide", serde_json::json!(["Gemini", 7, null]));
        assert_eq!(agents_choice(), 0);
        set("limits_hide", serde_json::json!("Codex"));
        assert_eq!(agents_choice(), 0, "не список — как пусто");
        // set_agents_choice → agents_choice: туда и обратно; вне диапазона — «за всеми»
        for c in [0u8, 1, 2] {
            set_agents_choice(c);
            assert_eq!(agents_choice(), c);
            assert!(AGENTS.iter().any(|a| shown(a)), "не следим ни за кем при выборе {c}");
        }
        set_agents_choice(9);
        assert_eq!(agents_choice(), 0);
        assert!(AGENTS.iter().all(|a| shown(a)));

        // window_choice: прямое значение, затем старая настройка tray_number 2..5 (чётное — 5 часов, нечётное — неделя)
        let w = |tw: serde_json::Value, tn: serde_json::Value| {
            crate::prefs::update(|p| {
                p["tray_window"] = tw;
                p["tray_number"] = tn;
            });
            window_choice()
        };
        use serde_json::Value::Null;
        for (tw, tn, want) in [
            (0, Null, 0),
            (1, Null, 1),
            (2, Null, 2),
            (3, Null, 0),
            (99, serde_json::json!(2), 1),
            (99, serde_json::json!(3), 2),
            (99, serde_json::json!(4), 1),
            (99, serde_json::json!(5), 2),
            (99, serde_json::json!(1), 0),
            (99, serde_json::json!(6), 0),
            (99, Null, 0),
        ] {
            assert_eq!(w(serde_json::json!(tw), tn.clone()), want, "tray_window={tw} tray_number={tn}");
        }
        assert_eq!(w(serde_json::json!("1"), Null), 0, "строка вместо числа — как нет");
        assert_eq!(w(serde_json::json!(-1), Null), 0);
        set_window_choice(200);
        assert_eq!(window_choice(), 2, "верхняя граница зажимается");

        // number_choice
        crate::prefs::update(|p| {
            p["tray_number"] = serde_json::json!(1);
        });
        assert_eq!(number_choice(), 1);
        for n in [0u64, 2, 3, 4, 5, 77] {
            crate::prefs::update(|p| p["tray_number"] = serde_json::json!(n));
            assert_eq!(number_choice(), 0, "{n}");
        }
        crate::prefs::update(|p| {
            p.as_object_mut().unwrap().remove("tray_number");
            p["tray_label"] = serde_json::json!("each");
        });
        assert_eq!(number_choice(), 1, "старая настройка «у каждого агента»");
        set_number_choice(200);
        assert_eq!(number_choice(), 1);
    });
}

/// Случайные сохранённые настройки (в том числе мусор разных типов): ни паник, значения всегда в допустимых границах.
#[test]
fn settings_fuzz_stays_in_bounds() {
    with_fake_home("tray_prefs_fuzz", |_| {
        for_seeds_small("tray_prefs_fuzz", 60, |r| {
            let vals = [
                serde_json::json!(null),
                serde_json::json!(0),
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(5),
                serde_json::json!(-3),
                serde_json::json!(1.5),
                serde_json::json!("x"),
                serde_json::json!(["Codex"]),
                serde_json::json!(["Claude Code", "Codex", "?"]),
                serde_json::json!({"a": 1}),
                serde_json::json!(u64::MAX),
            ];
            for k in ["limits_hide", "tray_window", "tray_number", "tray_label", "limits_notify"] {
                let v = r.pick(&vals).clone();
                crate::prefs::update(|p| p[k] = v);
            }
            assert!(agents_choice() <= AGENTS.len() as u8);
            assert!(window_choice() <= 2);
            assert!(number_choice() <= 1);
            assert!(AGENTS.iter().any(|a| shown(a)));
            let _ = notify_on();
        });
    });
}

fn for_seeds_small(name: &str, n: u64, f: impl Fn(&mut Rng)) {
    for i in 0..n {
        let seed = 0x5EED_2000_0000 + i;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut Rng(seed))));
        if let Err(e) = r {
            eprintln!("[{name}] ПАДЕНИЕ на зерне {seed:#x}");
            std::panic::resume_unwind(e);
        }
    }
}

// ───────────── разжатие сцены ─────────────

/// Независимый кодировщик (как `sp.rle` в рабочем скрипте художницы): пары «значение, длина−1», длина ≤ 71.
fn encode(v: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[i] && j - i < 70 {
            j += 1;
        }
        out.push((48 + v[i]) as char);
        out.push((48 + (j - i) as u8) as char);
        i = j + 1;
    }
    out
}

#[test]
fn unrle_round_trips_independent_encoder() {
    for_seeds("unrle", |r| {
        let n = r.range(0, 3000) as usize;
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            let (val, run) = (r.below(75) as u8, r.range(1, 200) as usize);
            v.extend(std::iter::repeat_n(val, run.min(n - v.len())));
        }
        let mut s = encode(&v);
        // перевод строки каждые 60 знаков (чётное число), как в файлах данных
        let mut wrapped = String::new();
        for (i, c) in s.drain(..).enumerate() {
            if i > 0 && i % 60 == 0 {
                wrapped.push('\n');
            }
            wrapped.push(c);
        }
        assert_eq!(unrle(&wrapped, n), v);
    });
    // граничные случаи: пусто, одна пара, нечётный хвост отбрасывается, пробелы и переводы строк пропускаются
    assert!(unrle("", 0).is_empty());
    assert_eq!(unrle("0\n", 0), Vec::<u8>::new(), "хвост без пары");
    assert_eq!(unrle("1 0", 1), vec![1]);
    // «v0» — значение 70 один раз; «0v» — значение 0, длина 'v'−'0'+1 = 71 → всего 72
    assert_eq!(unrle("v0\n0v", 0).len(), 72);
}

/// Сцены вшиты верно: слой кота, фон и палитра разжимаются в точные размеры.
#[test]
fn scene_images_have_exact_size() {
    use crate::scene_px::{BG, DONE, H, PAL, SLEEP, W, WRITE};
    assert_eq!(unrle(BG, (W * H) as usize).len(), (W * H) as usize);
    for l in [&SLEEP, &WRITE, &DONE] {
        let (_, _, w, h, d) = *l;
        assert_eq!(unrle(d, (w * h) as usize).len(), (w * h) as usize);
    }
    let (w, h, px) = scene_img(&WRITE);
    assert_eq!((w, h, px.len()), (W, H, (W * H) as usize));
    // каждая точка — цвет из палитры (ни «0» из-за ошибки индекса)
    assert!(px.iter().all(|p| PAL.contains(p)));
}

/// Масштабирование ближайшей точкой: каждая точка исходника остаётся цельной (цвета выходного — подмножество входного);
/// целочисленное увеличение k×k повторяет точки; размеры — округлением.
#[test]
fn nearest_resample_and_upscale_properties() {
    for_seeds("resample", |r| {
        let (w, h) = (r.range(1, 20) as i32, r.range(1, 20) as i32);
        let px: Vec<u32> = (0..w * h).map(|_| r.below(5) as u32 * 0x111111).collect();
        let src: Img = (w, h, px.clone());
        let k = r.range(1, 4) as i32;
        let (uw, uh, up) = upscale(src.clone(), k);
        assert_eq!((uw, uh), (w * k, h * k));
        for y in 0..uh {
            for x in 0..uw {
                assert_eq!(up[(y * uw + x) as usize], px[((y / k) * w + x / k) as usize]);
            }
        }
        let s = 0.3 + r.unit() * 1.7;
        let (dw, dh, dp) = resample_nearest(&src, s as f32);
        assert_eq!(dp.len(), (dw * dh) as usize);
        assert!(dw >= 1 && dh >= 1);
        assert!(dp.iter().all(|c| px.contains(c)), "появился цвет, которого нет в исходнике");
    });
}

/// Длительное испытание отрисовки плашки: сборка шаблона и кадров анимации по кругу, RSS/потоки/дескрипторы — в CSV.
#[test]
#[ignore = "длительное испытание: см. REGRESSION_TESTS.md"]
fn soak_tray_template_and_frames() {
    let secs = soak_secs(30);
    csv_line("t_s,renders,rss_kb,threads,fds,worst_ms,ok");
    let t0 = std::time::Instant::now();
    let mut fonts = Fonts::new();
    let (mut n, mut worst) = (0u64, 0u128);
    let mut next = 0;
    while t0.elapsed().as_secs() < secs {
        let k = 1 + (n % 3) as i32;
        let tpl = Tpl::new(k);
        let agents = sample_agents();
        let (w, h) = tpl.size(agents.len());
        let mut buf = vec![0u32; (w * h) as usize];
        let s = std::time::Instant::now();
        {
            let mut c = Canvas::new(&mut buf, w as usize, w, h);
            let fr = crate::scene_anim::frame((n % 3) as u8, n * 120);
            paint(&mut fonts, &agents, &Watch::default(), crate::ui::now_secs(), &tpl, &fr, &mut c);
        }
        worst = worst.max(s.elapsed().as_millis());
        assert!(buf.iter().any(|p| *p != 0), "плашка нарисована пустой");
        n += 1;
        if t0.elapsed().as_secs() >= next {
            let (rss, th, fd) = proc_stats();
            csv_line(&format!("{},{n},{rss},{th},{fd},{worst},1", t0.elapsed().as_secs()));
            next += 5;
        }
    }
}
