//! Регрессионные тесты лечения «Перезапустить GNOME Shell» (`fixes.rs`): разбор вывода `systemctl show` и отказ,
//! когда сбой службы оболочки завершил бы сеанс. Дочерний модуль `fixes::regress`.
//!
//! ЖЁСТКОЕ ПРАВИЛО этого файла: сама `shell_restart()` здесь НЕ вызывается ни при каких условиях (она посылает сигнал
//! `gnome-shell`; на GNOME 46 у владелицы это завершает сеанс). Проверяется только чистый разбор текста.

use super::*;
use crate::regress_common::*;

/// Вывод `systemctl --user show unit -p OnFailure -p ActiveState` в разных состояниях. Эталон — по смыслу правила:
/// отказ только если служба АКТИВНА и при сбое зовёт gnome-session-failed.
#[test]
fn shell_unit_table() {
    let t = [
        // (вывод, ждём «завершит сеанс»)
        ("OnFailure=gnome-session-failed.target org.gnome.Shell-disable-extensions.service\nActiveState=active\n", true),
        ("ActiveState=active\nOnFailure=gnome-session-failed.target\n", true), // порядок строк не важен
        ("OnFailure=gnome-session-failed.target\nActiveState=inactive\n", false),
        ("OnFailure=gnome-session-failed.target\nActiveState=failed\n", false),
        ("OnFailure=gnome-session-failed.target\nActiveState=activating\n", false),
        ("OnFailure=gnome-session-failed.target\nActiveState=deactivating\n", false),
        ("OnFailure=\nActiveState=active\n", false),
        ("OnFailure=other.target\nActiveState=active\n", false),
        ("ActiveState=active\n", false),
        ("OnFailure=gnome-session-failed.target\n", false),
        ("", false),
        ("\n\n", false),
        ("garbage", false),
        // с пробелами и \r\n вокруг значения активности
        ("OnFailure=gnome-session-failed.target\r\nActiveState=active\r\n", true),
        ("OnFailure=gnome-session-failed.target\n  ActiveState=active  \n", true),
    ];
    for (out, want) in t {
        assert_eq!(shell_unit_ends_session(out), want, "{out:?}");
    }
}

/// Случайные тексты из частей вывода: результат совпадает с независимой формулировкой правила; порядок строк и посторонние
/// строки результат не меняют (метаморфическое).
#[test]
fn shell_unit_random_outputs_match_rule_and_ignore_order() {
    for_seeds("shell_unit", |r| {
        let actives = ["active", "inactive", "failed", "activating", "reloading", ""];
        let onfail = [
            "gnome-session-failed.target",
            "gnome-session-failed.target org.gnome.Shell-disable-extensions.service",
            "other.target",
            "",
            "x gnome-session-failed y",
        ];
        let mut lines: Vec<String> = Vec::new();
        let (mut has_active, mut has_fail) = (false, false);
        if r.chance(85) {
            let a = *r.pick(&actives);
            has_active |= a == "active";
            lines.push(format!("ActiveState={a}"));
        }
        if r.chance(85) {
            let f = *r.pick(&onfail);
            has_fail |= f.contains("gnome-session-failed");
            lines.push(format!("OnFailure={f}"));
        }
        for _ in 0..r.range(0, 4) {
            lines.push(format!("Noise{}={}", r.below(5), r.below(100)));
        }
        let text = lines.join("\n") + "\n";
        assert_eq!(shell_unit_ends_session(&text), has_active && has_fail, "{text:?}");
        // перемешать строки
        let mut l2 = lines.clone();
        for i in (1..l2.len()).rev() {
            let j = r.below(i as u64 + 1) as usize;
            l2.swap(i, j);
        }
        assert_eq!(shell_unit_ends_session(&(l2.join("\n") + "\n")), has_active && has_fail);
        // дописать мусор — не меняет
        assert_eq!(shell_unit_ends_session(&(text.clone() + "ActiveState=inactive-ish\nFoo=bar\n")), has_active && has_fail);
    });
}

/// Если `systemctl` не ответил (нет шины, тайм-аут, пустой вывод), сигнал оболочке не отправляется: разбор не видит ни
/// одной строки ActiveState — «не ответил», и shell_restart_ends_session() даёт отказ (раньше было «при сомнении — делать»).
#[test]
fn empty_systemctl_output_means_refusal() {
    assert!(!shell_unit_ends_session(""));
    assert!(!shell_unit_answered(""));
    assert!(shell_unit_answered("OnFailure=\nActiveState=inactive\n"));
}

/// Текст отказа содержит способ сделать вручную (Alt+F2, r, Enter) — без него отказ был бы тупиком.
#[test]
fn refusal_message_tells_how_to_do_it_by_hand() {
    assert!(SHELL_RESTART_BY_HAND.contains("Alt+F2") && SHELL_RESTART_BY_HAND.contains("Enter"));
    assert!(SHELL_RESTART_BY_HAND.contains("сеанс"));
}
