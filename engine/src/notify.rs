//! Уведомление на рабочем столе: в категории, где что-то игнорируют («Игнорировать»), появилась новая беда —
//! её иначе не заметить (итог спокоен, кот не грустит). Шлёт только проверяющий процесс (sched.rs, у кого замок
//! проверок), поэтому окно и виджет вместе не присылают его дважды.
//! Linux — notify-send, macOS — osascript «display notification», Windows — всплывающее уведомление PowerShell.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use crate::diag::Check;

/// Беды прошлого прогона (по названию, игнорируемые тоже): новая — та, которой там не было.
static PREV: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn is_problem(c: &Check) -> bool {
    matches!(c.status.as_str(), "bad" | "warn")
}

/// Новые беды в категориях, где что-то игнорируется. Первый прогон только запоминается.
fn fresh<'a>(prev: Option<&HashSet<String>>, all: &'a [Check], ignored: &HashSet<String>) -> Vec<&'a Check> {
    let Some(prev) = prev else { return Vec::new() };
    let watched: HashSet<&str> = all.iter().filter(|c| ignored.contains(&c.title)).map(|c| c.key).collect();
    all.iter().filter(|c| is_problem(c) && !prev.contains(&c.title) && !ignored.contains(&c.title) && watched.contains(c.key)).collect()
}

/// Новые результаты проверок (от проверяющего процесса).
pub fn watch(all: &[Check]) {
    let ignored = crate::prefs::ignored();
    let now: HashSet<String> = all.iter().filter(|c| is_problem(c)).map(|c| c.title.clone()).collect();
    let mut prev = PREV.lock().unwrap_or_else(|e| e.into_inner());
    let new = fresh(prev.as_ref(), all, &ignored);
    // «проверяю…» в начале — ещё не результат: прошлое не забываем
    if !all.iter().any(|c| c.status == "checking") {
        *prev = Some(now);
    }
    let Some(first) = new.first() else { return };
    let cat = crate::catalog::CATS.iter().find(|c| c.key() == first.key).map(|c| crate::i18n::t(c.name())).unwrap_or_default();
    let tr = crate::i18n::t;
    let mut body = format!("{} — {}", tr(&first.title), tr(&first.value));
    if new.len() > 1 {
        body.push_str(&tr(&format!(" И ещё {}.", new.len() - 1)));
    }
    let title = tr(&format!("Аптечка: новая беда в «{cat}»"));
    crate::diag::log(&format!("уведомление: {title}: {body}"));
    send(title, body);
}

/// Показать уведомление (в фоне: окно и проверки не ждут).
pub(crate) fn send(title: String, body: String) {
    if cfg!(test) || ["UI_SHOT", "UI_FAKE", "FRAMES", "WIDGET_SHOT"].iter().any(|v| std::env::var_os(v).is_some()) {
        return;
    }
    std::thread::spawn(move || {
        let t = Duration::from_secs(10);
        #[cfg(target_os = "macos")]
        let cmd: Vec<String> = [
            "osascript",
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            &title,
            &body,
        ]
        .map(String::from)
        .to_vec();
        #[cfg(windows)]
        let cmd: Vec<String> = {
            let q = crate::agents::ps_quote;
            let script = format!(
                "$m=[Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
                 $x=[Windows.Data.Xml.Dom.XmlDocument,Windows.Data.Xml.Dom.XmlDocument,ContentType=WindowsRuntime]::new();\
                 $x.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text>'+[Security.SecurityElement]::Escape({})+'</text><text>'+[Security.SecurityElement]::Escape({})+'</text></binding></visual></toast>');\
                 $m::CreateToastNotifier('{{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
                q(&title),
                q(&body)
            );
            ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &crate::elevate::ps_encoded(&script)]
                .map(String::from)
                .to_vec()
        };
        #[cfg(not(any(target_os = "macos", windows)))]
        let cmd: Vec<String> = ["notify-send", "-a", &crate::i18n::t("Аптечка"), "-i", "dialog-warning", &title, &body].map(String::from).to_vec();
        let args: Vec<&str> = cmd.iter().map(String::as_str).collect();
        let r = crate::cmd::exec(&args, t, crate::cmd::FIX);
        if r.code() != 0 {
            crate::diag::log("уведомление: система не показала (нет notify-send или уведомления выключены)");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(key: &'static str, title: &str, status: &str) -> Check {
        Check::new(key, title, status, "")
    }

    /// Новая беда в категории с игнорируемым — да; в другой категории, старая беда, игнорируемая — нет.
    #[test]
    fn fresh_only_in_watched_category() {
        let ig: HashSet<String> = ["Диск".to_string()].into();
        let prev: HashSet<String> = ["Диск".to_string(), "DNS".to_string()].into();
        let all = [c("sys", "Диск", "bad"), c("sys", "Память", "warn"), c("net", "Прокси", "bad"), c("net", "DNS", "bad")];
        let f: Vec<&str> = fresh(Some(&prev), &all, &ig).iter().map(|c| c.title.as_str()).collect();
        assert_eq!(f, ["Память"]);
        // первый прогон — только запоминается
        assert!(fresh(None, &all, &ig).is_empty());
        // ничего не игнорируется — не беспокоим (о беде и так скажут итог и кот)
        assert!(fresh(Some(&prev), &all, &HashSet::new()).is_empty());
    }
}
