//! Регрессионные тесты окна (`ui.rs`): строки замера скорости, список проблем у реплики кота, перерисовка кнопок под
//! всплывающими списками, нажатия при смене порядка и исчезновении строк. Дочерний модуль `ui::regress`.
//! Все настройки и журнал — во временной папке (`with_fake_home`): настоящие настройки не затрагиваются.
#![allow(clippy::manual_is_multiple_of)] // is_multiple_of стабилен с Rust 1.90, а проект собирается с 1.89

use super::*;
use crate::regress_common::*;

fn chk(key: &'static str, title: &str, status: &str, unit: &'static str, fixes: &[(&str, &str)]) -> Check {
    Check {
        key,
        title: title.into(),
        status: status.into(),
        value: format!("значение «{title}»"),
        detail: String::new(),
        fixes: fixes.iter().map(|f| crate::diag::Fix { id: f.0.into(), label: f.1.into(), arg: None }).collect(),
        unit,
        unchecked: false,
    }
}

fn click(u: &mut Ui, h: Hit) -> Vec<Cmd> {
    u.ensure_hits();
    let r = u.hit_rect(h);
    assert!(!r.is_empty(), "{h:?} не найден на экране");
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    u.mouse_move(x, y);
    u.mouse_down(x, y);
    u.mouse_up(x, y)
}

fn titles_of<'a>(it: impl Iterator<Item = &'a Check>) -> Vec<String> {
    it.map(|c| c.title.clone()).collect()
}

// ───────────── строки замера скорости ─────────────

/// Эталон по описанию: строка «Пинг» замера скрывается, ТОЛЬКО если в таблице есть строка «Задержка сети».
#[test]
fn speed_rows_hide_ping_only_when_latency_row_present() {
    with_fake_home("ui_speed", |_| {
        let mut u = Ui::new(1, 300, 500);
        let speed = vec![
            chk("net", "Пинг", "ok", "", &[]),
            chk("net", "Скорость ↓ скачивание", "ok", "", &[]),
            chk("net", "Скорость ↑ отдача", "ok", "", &[]),
        ];
        u.set_speed_rows(speed.clone());
        // без «Задержки сети» — все три
        assert_eq!(titles_of(u.st.speed_rows()), ["Пинг", "Скорость ↓ скачивание", "Скорость ↑ отдача"]);
        // с ней — без «Пинга»
        u.set_checks(vec![chk("net", "Задержка сети", "ok", "latency", &[])], false);
        assert_eq!(titles_of(u.st.speed_rows()), ["Скорость ↓ скачивание", "Скорость ↑ отдача"]);
        // похожее название — не повод скрывать
        u.set_checks(vec![chk("net", "Задержка сети (старая)", "ok", "", &[])], false);
        assert_eq!(u.st.speed_rows().count(), 3);
        // скорости нет совсем
        u.set_speed_rows(Vec::new());
        assert_eq!(u.st.speed_rows().count(), 0);
    });
}

/// Показатель «Задержка сети» ВЫКЛЮЧЕН в настройках: его строки в таблице нет, значит и пинг замера скрывать не из-за чего —
/// иначе не видно ни одной задержки. Сейчас `speed_rows` смотрит на сырой список проверок и скрывает «Пинг» всё равно.
#[test]
#[ignore = "известная ошибка: при выключенном показателе «Задержка сети» строка «Пинг» замера всё равно скрыта — задержки не видно нигде"]
fn speed_ping_stays_visible_when_latency_unit_is_off() {
    with_fake_home("ui_speed_off", |_| {
        crate::prefs::set_unit("latency", false);
        let mut u = Ui::new(1, 300, 500);
        u.set_speed_rows(vec![chk("net", "Пинг", "ok", "", &[]), chk("net", "Скорость ↓ скачивание", "ok", "", &[])]);
        u.set_checks(vec![chk("net", "Задержка сети", "ok", "latency", &[])], false);
        let net = CATS.iter().position(|c| c.key() == "net").unwrap() as u8;
        u.set_tab(net);
        let shown = titles_of(u.st.rows.iter());
        assert!(shown.iter().any(|t| t == "Задержка сети" || t == "Пинг"), "ни задержки, ни пинга: {shown:?}");
    });
}

// ───────────── список проблем ─────────────

#[test]
fn problems_order_dedup_and_filters() {
    with_fake_home("ui_prob", |_| {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(
            vec![
                chk("net", "Ж1", "warn", "", &[]),
                chk("sys", "К1", "bad", "", &[]),
                chk("net", "Ок", "ok", "", &[]),
                chk("net", "Инфо", "info", "", &[]),
                chk("net", "Идёт", "checking", "", &[]),
                chk("net", "Ж2", "warn", "", &[]),
                chk("gpu", "К2", "bad", "", &[]),
            ],
            false,
        );
        u.set_speed_rows(vec![chk("net", "Скорость ↓ скачивание", "bad", "", &[])]);
        let got: Vec<(String, String)> = u.st.problems().into_iter().map(|p| (p.0, p.1)).collect();
        // красные раньше жёлтых; внутри группы — порядок проверок (сортировка устойчивая)
        let names: Vec<&str> = got.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(names, ["К1", "К2", "Скорость ↓ скачивание", "Ж1", "Ж2"]);
        assert!(got[..3].iter().all(|p| p.1 == "bad") && got[3..].iter().all(|p| p.1 == "warn"));
        // игнорируемые исчезают из списка
        crate::prefs::set_ignored(&["К1".to_string(), "Ж2".to_string()], true);
        u.set_checks(u.st.checks.clone(), false); // перечитать игнор
        let names: Vec<String> = u.st.problems().into_iter().map(|p| p.0).collect();
        assert_eq!(names, ["К2", "Скорость ↓ скачивание", "Ж1"]);
    });
}

/// Одно и то же название встречается в двух строках (не рядом): в списке проблем оно должно быть один раз. `dedup_by`
/// убирает только соседние повторы, а после сортировки по статусу одинаковые названия не всегда соседи.
#[test]
#[ignore = "известная ошибка: St::problems убирает повторы названий только соседние — «Ж1, Ж2, Ж1» остаётся с двумя «Ж1»"]
fn problems_has_no_duplicate_titles() {
    with_fake_home("ui_dup", |_| {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(vec![chk("net", "Ж1", "warn", "", &[]), chk("net", "Ж2", "warn", "", &[]), chk("net", "Ж1", "warn", "", &[])], false);
        let names: Vec<String> = u.st.problems().into_iter().map(|p| p.0).collect();
        assert_eq!(names, ["Ж1", "Ж2"]);
    });
}

/// Свойство: порядок входных строк не меняет МНОЖЕСТВО проблем и их статус; красные всегда раньше жёлтых; названия без повторов
/// подряд.
#[test]
fn problems_permutation_invariants() {
    with_fake_home("ui_prob_perm", |_| {
        for_seeds_n("problems", 60, |r| {
            let n = r.range(0, 25) as usize;
            let mut checks: Vec<Check> = (0..n)
                .map(|i| {
                    let st = *r.pick(&["ok", "warn", "bad", "info", "checking"]);
                    chk(r.pick(&["net", "sys", "gpu", "ai"]), &format!("T{}", r.below(12)), st, "", &[("f", "F")]).tap(|c| c.value = i.to_string())
                })
                .collect();
            let mut u = Ui::new(1, 300, 500);
            u.set_checks(checks.clone(), false);
            let base: std::collections::BTreeSet<String> = u.st.problems().into_iter().map(|p| p.0).collect();
            let list = u.st.problems();
            let first_warn = list.iter().position(|p| p.1 == "warn").unwrap_or(list.len());
            assert!(list[first_warn..].iter().all(|p| p.1 == "warn"), "красное после жёлтого");
            assert!(list.windows(2).all(|w| w[0].0 != w[1].0), "повтор названия подряд");
            // перестановка
            for i in (1..checks.len()).rev() {
                let j = r.below(i as u64 + 1) as usize;
                checks.swap(i, j);
            }
            u.set_checks(checks.clone(), false);
            // множество названий у проблем не зависит от порядка, если у названия все дубли одного статуса;
            // иначе дубль с красным и жёлтым даёт две записи — проверяем только вложение
            let perm: std::collections::BTreeSet<String> = u.st.problems().into_iter().map(|p| p.0).collect();
            assert_eq!(base, perm, "набор проблем зависит от порядка строк");
        });
    });
}

trait Tap: Sized {
    fn tap(self, f: impl FnOnce(&mut Self)) -> Self;
}
impl Tap for Check {
    fn tap(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }
}

/// Кнопка «Игнорировать всё»/«Вернуть всё»: когда все нынешние беды уже игнорируются, надо предлагать «Вернуть всё».
/// Строки замера скорости не проходят через `apply_ignored`, поэтому игнорируемая красная скорость остаётся в
/// `problem_titles` и подпись навсегда «Игнорировать всё» (хотя игнорировать больше нечего).
#[test]
#[ignore = "известная ошибка: problem_titles не учитывает игнорируемые, а problems — учитывает; при игнорируемой красной строке скорости кнопка остаётся «Игнорировать всё»"]
fn ignore_all_label_after_everything_is_ignored() {
    with_fake_home("ui_ign", |_| {
        let mut u = Ui::new(1, 300, 500);
        u.set_speed_rows(vec![chk("net", "Скорость ↓ скачивание", "bad", "", &[])]);
        assert_eq!(u.st.ignore_all_label(), Some("Игнорировать всё"));
        crate::prefs::set_ignored(&["Скорость ↓ скачивание".to_string()], true);
        u.set_speed_rows(u.st.speed_checks.clone()); // перечитать игнор
        assert!(u.st.problems().is_empty());
        assert_eq!(u.st.ignore_all_label(), Some("Вернуть всё"), "игнорировать уже нечего");
    });
}

/// Номер пункта вне списка (список успел сократиться) и пустой список: без паники, список закрывается, вкладка не прыгает.
#[test]
fn problem_click_with_stale_index_is_harmless() {
    with_fake_home("ui_stale", |_| {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(vec![chk("sys", "К", "bad", "", &[])], false);
        let tab = u.st.tab;
        u.st.popup = Popup::Problems;
        let cmds = u.activate(Hit::Problem(200));
        assert!(cmds.is_empty());
        assert_eq!(u.st.popup, Popup::None);
        assert_eq!(u.st.tab, tab, "вкладка сменилась по несуществующему пункту");
        // совсем пустой список
        u.set_checks(Vec::new(), false);
        u.st.popup = Popup::Problems;
        let _ = u.activate(Hit::Problem(0));
        assert_eq!(u.st.popup, Popup::None);
    });
}

/// Много проблем: рисуются 250, номера пунктов различны и ведут на «свои» строки.
#[test]
fn problems_list_is_capped_and_indices_unique() {
    with_fake_home("ui_many", |_| {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks((0..300).map(|i| chk("net", &format!("беда {i:03}"), "bad", "", &[])).collect(), false);
        assert_eq!(u.st.problems().len(), 300);
        u.st.popup = Popup::Problems;
        u.hits_ok = false;
        u.ensure_hits();
        let idx: Vec<u8> = u.hits.iter().filter_map(|(_, h)| if let Hit::Problem(i) = h { Some(*i) } else { None }).collect();
        assert_eq!(idx.len(), 250, "рисуется не больше 250 пунктов");
        let uniq: std::collections::BTreeSet<_> = idx.iter().collect();
        assert_eq!(uniq.len(), idx.len());
        assert_eq!((*idx.iter().min().unwrap(), *idx.iter().max().unwrap()), (0, 249));
    });
}

/// Нажали на пункт списка, пока шла проверка и порядок проблем поменялся: отпускание не должно вести к ДРУГОЙ проблеме,
/// чем та, на которой нажали. Для кнопок починки/«Игнорировать» нажатие в таком случае отменяется (`rebuild_rows`), для
/// пунктов списка проблем этого нет.
#[test]
#[ignore = "известная ошибка: нажатие на пункт списка проблем не отменяется при обновлении проверок — отпускание уводит на вкладку другой проблемы"]
fn pressed_problem_item_survives_list_reorder() {
    with_fake_home("ui_reorder", |_| {
        let mut u = Ui::new(1, 300, 500);
        u.set_checks(vec![chk("sys", "Первая", "bad", "", &[]), chk("net", "Вторая", "bad", "", &[])], false);
        let net = CATS.iter().position(|c| c.key() == "net").unwrap() as u8;
        let sys = CATS.iter().position(|c| c.key() == "sys").unwrap() as u8;
        u.set_tab(net);
        click(&mut u, Hit::BalloonMore);
        assert_eq!(u.st.popup, Popup::Problems);
        u.ensure_hits();
        let r = u.hit_rect(Hit::Problem(0)); // «Первая» (sys)
        let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
        u.mouse_down(x, y);
        // пришли новые строки: теперь на этом месте «Вторая»
        u.set_checks(vec![chk("net", "Вторая", "bad", "", &[]), chk("sys", "Первая", "bad", "", &[])], false);
        u.ensure_hits();
        u.mouse_up(x, y);
        assert_ne!(u.st.tab, net, "ушли на вкладку «Вторая» (net), хотя нажимали на «Первая» (sys)");
        assert_eq!(u.st.tab, sys);
    });
}

// ───────────── перерисовка кнопок под всплывающими списками ─────────────

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

/// После закрытия любого списка (щелчком мимо, Esc, повторным нажатием) в перерисовку попадают все четыре «вдавливаемые»
/// области: строка меню, кнопка «Сессии», кнопка «Полечить» и реплика кота.
#[test]
fn every_popup_close_path_redraws_all_anchors() {
    with_fake_home("ui_anchors", |_| {
        for popup in [Popup::Heal, Popup::Sessions, Popup::Menu(0), Popup::Problems] {
            for how in ["outside", "escape", "toggle"] {
                let mut u = Ui::new(1, 300, 500);
                u.set_checks(vec![chk("net", "Беда", "bad", "", &[])], false);
                u.ensure_hits();
                let g = u.st.geo();
                let anchors = [g.menubar, g.claude, g.tools[TOOL_HEAL as usize], g.balloon];
                u.st.popup = popup;
                let _ = u.take_dirty();
                match how {
                    "outside" => u.close_popup(),
                    "escape" => u.key_escape(),
                    _ => u.toggle_popup(popup),
                }
                assert_eq!(u.st.popup, Popup::None, "{popup:?} {how}");
                let dirty = u.take_dirty();
                for (i, a) in anchors.iter().enumerate() {
                    if a.is_empty() {
                        continue;
                    }
                    assert!(dirty.iter().any(|d| overlaps(*d, *a)), "{popup:?} {how}: область {i} не перерисована");
                }
            }
        }
    });
}

// ───────────── случайные последовательности действий ─────────────

/// Случайные нажатия, смена строк, всплывающие списки и Esc: ни паник; `RunFix` приходит только для починки,
/// существовавшей в строках на момент отпускания.
#[test]
fn random_interaction_sequences_are_safe() {
    with_fake_home("ui_fuzz", |_| {
        for_seeds_n("ui_fuzz", 25, |r| {
            let mut u = Ui::new(1 + r.below(2) as i32, 300, 500);
            let pool = [("net", "A"), ("net", "B"), ("sys", "C"), ("gpu", "D"), ("ai", "E"), ("theme", "F")];
            let make = |r: &mut Rng| -> Vec<Check> {
                let n = r.range(0, 8) as usize;
                (0..n)
                    .map(|_| {
                        let (k, t) = *r.pick(&pool);
                        let st = *r.pick(&["ok", "warn", "bad", "info"]);
                        let fx: &[(&str, &str)] = if r.chance(60) { &[("fix_a", "Починить A")] } else { &[] };
                        chk(k, t, st, "", fx)
                    })
                    .collect()
            };
            u.set_checks(make(r), false);
            for _ in 0..60 {
                match r.below(8) {
                    0 => {
                        let c = make(r);
                        u.set_checks(c, false);
                    }
                    1 => {
                        let c = make(r);
                        u.set_speed_rows(c);
                    }
                    2 => u.key_escape(),
                    3 => {
                        let t = r.below(N_TABS as u64) as u8;
                        u.set_tab(t);
                    }
                    4 => u.st.popup = *r.pick(&[Popup::None, Popup::Heal, Popup::Sessions, Popup::Menu(0), Popup::Problems]),
                    _ => {
                        u.ensure_hits();
                        let hit = u.hits.get(r.below(u.hits.len().max(1) as u64) as usize).map(|(rc, _)| *rc);
                        if let Some(rc) = hit {
                            let (x, y) = (rc.x + rc.w / 2, rc.y + rc.h / 2);
                            u.mouse_move(x, y);
                            u.mouse_down(x, y);
                            if r.chance(40) {
                                let c = make(r);
                                u.set_checks(c, false);
                            }
                            let ids: Vec<String> =
                                u.st.checks.iter().chain(u.st.speed_rows()).flat_map(|c| c.fixes.iter().map(|f| f.id.clone())).collect();
                            for cmd in u.mouse_up(x, y) {
                                if let Cmd::RunFix { id, .. } = cmd {
                                    assert!(ids.contains(&id), "запущена починка «{id}», которой нет в текущих строках");
                                }
                            }
                        }
                    }
                }
                let _ = u.take_dirty();
            }
        });
    });
}

fn for_seeds_n(name: &str, n: u64, f: impl Fn(&mut Rng)) {
    for i in 0..n {
        let seed = 0x5EED_3000_0000 + i;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut Rng(seed))));
        if let Err(e) = r {
            eprintln!("[{name}] ПАДЕНИЕ на зерне {seed:#x}");
            std::panic::resume_unwind(e);
        }
    }
}
