//! Настройки Аптечки на диске: папка настроек своя у каждой системы, файл ui.json.
//! На Linux это ~/.config/aptechka/ui.json — тот же файл, что у Python-версии ({"split", "log"}),
//! сюда же добавлено "off": [коды выключенных показателей].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Папка настроек: Linux — $XDG_CONFIG_HOME или ~/.config; macOS — ~/Library/Application Support;
/// Windows — %APPDATA%.
pub fn config_dir() -> PathBuf {
    // тесты подменяют папку через XDG_CONFIG_HOME на любой системе: на macOS и Windows иначе они писали бы
    // в настоящую папку настроек
    #[cfg(test)]
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return x.join("aptechka");
    }
    // без домашней папки — личная папка сеанса ($XDG_RUNTIME_DIR, 0700), а не общий /tmp (там её мог бы
    // заранее занять другой пользователь)
    let home = || {
        crate::store::home_dir().unwrap_or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| std::env::temp_dir().join(format!("aptechka-nohome-{}", std::process::id())))
        })
    };
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home().join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config"))
    };
    base.join("aptechka")
}

pub fn ui_path() -> PathBuf {
    // та же проверенная папка, куда пишет update (своя, 0700, не ссылка)
    crate::store::private_dir().join("ui.json")
}

/// Настройки. Читаются с диска, только если файл изменился (планировщик спрашивает их часто).
type Stamp = Option<(std::time::SystemTime, u64)>;
static CACHE: std::sync::Mutex<Option<(Stamp, serde_json::Value)>> = std::sync::Mutex::new(None);

pub fn load() -> serde_json::Value {
    let p = ui_path();
    let stamp: Stamp = std::fs::metadata(&p).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((s, v)) = c.as_ref() {
        if *s == stamp && stamp.is_some() {
            return v.clone();
        }
    }
    let v = read_state(&p).ok().flatten().unwrap_or(serde_json::json!({}));
    *c = Some((stamp, v.clone()));
    v
}

/// Разобрать ui.json. Невидимую метку BOM в начале (её ставит Блокнот Windows и PowerShell 5) пропускаем:
/// иначе файл не читался и настройки молча сбрасывались к начальным.
fn parse(s: &str) -> Option<serde_json::Value> {
    serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
}

/// Байты файла настроек в текст: UTF-8 (с BOM или без) и UTF-16 с меткой порядка байт — так по умолчанию
/// пишут `Out-File`/`Set-Content` в PowerShell 5 и «Юникод» в Блокноте.
fn decode(b: &[u8]) -> Option<String> {
    let utf16 = |rest: &[u8], be: bool| -> Option<String> {
        if !rest.len().is_multiple_of(2) {
            return None;
        }
        let u: Vec<u16> =
            rest.as_chunks::<2>().0.iter().map(|c| if be { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) }).collect();
        String::from_utf16(&u).ok()
    };
    match b {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        _ => String::from_utf8(b.to_vec()).ok(),
    }
}

/// Прочитать ui.json: Ok(None) — файла нет или он пуст; Ok(Some) — разобран (объект);
/// Err(true) — файл есть, читается, но не разбирается (его надо сохранить копией); Err(false) — не читается.
fn read_state(p: &std::path::Path) -> Result<Option<serde_json::Value>, bool> {
    match crate::store::read_bytes(p, 32 << 20) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(false),
        Ok(b) => {
            let Some(t) = decode(&b) else { return Err(true) };
            if t.trim_start_matches('\u{feff}').trim().is_empty() {
                return Ok(None);
            }
            match parse(&t) {
                Some(v) if v.is_object() => Ok(Some(v)),
                _ => Err(true),
            }
        }
    }
}

static LAST_ERR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn fail(msg: String) {
    crate::diag::log(&msg);
    *LAST_ERR.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
}

/// Последняя необъявленная ошибка сохранения настроек (для строки в журнале окна); забирается один раз.
pub fn take_error() -> Option<String> {
    LAST_ERR.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Изменить ui.json: прочитать, поправить, записать целиком (чужие ключи сохраняются).
/// Под замком: окно и виджет, меняющие настройки одновременно, не затрут правки друг друга.
/// Файл, который не читается, не перезаписываем; не разбирается — сначала копия `.bad`.
pub fn update(f: impl FnOnce(&mut serde_json::Value)) {
    let dir = crate::store::private_dir();
    // без замка не пишем: две копии (окно и виджет) потеряли бы правки друг друга
    let Some(_lock) = crate::store::lock_wait(&dir.join("ui.lock"), std::time::Duration::from_secs(2)) else {
        fail("Настройки не сохранены: файл настроек занят другой копией Аптечки".into());
        return;
    };
    let p = dir.join("ui.json");
    let mut v = match read_state(&p) {
        Ok(v) => v.unwrap_or_else(|| serde_json::json!({})),
        Err(false) => {
            fail("Настройки не сохранены: файл настроек не читается, оставляю его как есть".into());
            return;
        }
        Err(true) => {
            // испорчен руками или не тот вид — сохранить рядом, а не затереть молча; не вышло копию — не пишем
            if let Err(e) = save_bad_copy(&p) {
                fail(format!("Настройки не сохранены: не удалось сделать копию испорченного файла настроек ({e})"));
                return;
            }
            serde_json::json!({})
        }
    };
    f(&mut v);
    if let Err(e) = crate::store::write_private(&p, serde_json::to_string_pretty(&v).unwrap_or_default().as_bytes()) {
        fail(format!("Настройки не сохранены: {e}"));
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Копия испорченного файла настроек: новое имя с временем (прежние копии не затираются), создаётся заново
/// (create_new: ссылка на месте имени не пройдёт), права 0600. Хранятся три последние.
fn save_bad_copy(p: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let data = crate::store::read_bytes(p, 32 << 20)?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let dir = p.parent().unwrap_or(Path::new("."));
    let base = p.file_name().and_then(|n| n.to_str()).unwrap_or("ui.json");
    let prefix = format!("{base}.bad.");
    // прежние копии — только свои обычные файлы (не ссылки); порядок по ключу (секунды, номер в секунде)
    let key = |n: &str| {
        let r = &n[prefix.len()..];
        let (a, b) = r.split_once('-').unwrap_or((r, "0"));
        (a.parse::<u64>().unwrap_or(0), b.parse::<u64>().unwrap_or(0))
    };
    let list = || -> Vec<(String, PathBuf)> {
        let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
                    .filter(|(n, _)| n.starts_with(&prefix))
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|(n, _)| key(n));
        v
    };
    // новая копия всегда «позже» всех прежних, даже если часы ушли назад или копий в одну секунду несколько
    let mut k = match list().last().map(|(n, _)| key(n)) {
        Some((s, n)) if s >= secs => (s, n + 1),
        _ => (secs, 0),
    };
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = loop {
        let name = if k.1 == 0 { format!("{prefix}{}", k.0) } else { format!("{prefix}{}-{}", k.0, k.1) };
        match o.open(dir.join(name)) {
            Ok(f) => break f,
            // имя занято (в том числе подложенной ссылкой: create_new её не открывает) — следующее
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && k.1 < 1000 => k.1 += 1,
            Err(e) => return Err(e),
        }
    };
    f.write_all(&data)?;
    let old = list();
    let extra = old.len().saturating_sub(3);
    for (_, path) in old.into_iter().take(extra) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

/// Явно выключенные и явно включённые показатели (всё прочее — как задано по умолчанию).
pub fn unit_choice() -> (HashSet<String>, HashSet<String>) {
    let v = load();
    let set = |k: &str| -> HashSet<String> {
        v.get(k).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    (set("off"), set("on"))
}

pub fn unit_enabled(id: &str, default_on: bool) -> bool {
    let (off, on) = unit_choice();
    if off.contains(id) {
        false
    } else if on.contains(id) {
        true
    } else {
        default_on
    }
}

/// Анимация картинки в окне (ui.json app_anim, по умолчанию да; APP_ANIM=0 — выключить): выключена —
/// кот замирает, окно просыпается раз в секунду только за данными.
pub fn app_anim() -> bool {
    std::env::var("APP_ANIM").map_or(true, |v| v != "0") && load().get("app_anim").and_then(|x| x.as_bool()).unwrap_or(true)
}

/// Проверки, которые велели не считать бедой (кнопка «Игнорировать»): их названия. Только до перезапуска
/// приложения — лежат в папке сеанса (ignored.json), окно при запуске их сбрасывает (за это время могла
/// появиться новая беда); виджет читает тот же файл.
pub fn ignored() -> HashSet<String> {
    let p = crate::store::runtime_dir().join("ignored.json");
    crate::store::read_capped(&p, 1 << 20)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .map(|v| v.into_iter().collect())
        .unwrap_or_default()
}

fn save_ignored(set: &HashSet<String>) {
    let p = crate::store::runtime_dir().join("ignored.json");
    if set.is_empty() {
        crate::store::remove_file_nofollow(&p);
        return;
    }
    let mut v: Vec<&String> = set.iter().collect();
    v.sort();
    let _ = crate::store::write_private(&p, serde_json::to_string(&v).unwrap_or_default().as_bytes());
}

/// Добавить проверки в «игнорируемые» (on) или убрать их оттуда.
pub fn set_ignored(titles: &[String], on: bool) {
    let mut set = ignored();
    for t in titles {
        if on {
            set.insert(t.clone());
        } else {
            set.remove(t);
        }
    }
    save_ignored(&set);
}

/// Окно запустилось: игнор снимается (за время, пока программа была закрыта, могла появиться новая беда).
pub fn clear_ignored() {
    save_ignored(&HashSet::new());
}

pub fn set_unit(id: &str, enabled: bool) {
    update(|v| {
        for (k, add) in [("off", !enabled), ("on", enabled)] {
            let mut a: Vec<String> =
                v.get(k).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            a.retain(|x| x != id);
            if add {
                a.push(id.to_string());
            }
            v[k] = serde_json::json!(a);
        }
    });
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn with_cfg<R>(name: &str, f: impl FnOnce(&std::path::Path) -> R) -> R {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("aptechka_prefs_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let old = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", &d);
        let dir = crate::store::private_dir();
        let r = f(&dir);
        match old {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&d);
        r
    }

    #[test]
    fn utf16_ui_json_is_kept() {
        with_cfg("u16", |dir| {
            let text = "{\"off\": [\"vpn\", \"gpu\"], \"split\": 7}";
            for be in [false, true] {
                let mut b: Vec<u8> = if be { vec![0xFE, 0xFF] } else { vec![0xFF, 0xFE] };
                for u in text.encode_utf16() {
                    b.extend(if be { u.to_be_bytes() } else { u.to_le_bytes() });
                }
                std::fs::write(dir.join("ui.json"), &b).unwrap();
                update(|v| v["log"] = serde_json::json!(true));
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("ui.json")).unwrap()).unwrap();
                assert_eq!(v["split"], 7);
                assert_eq!(v["off"][1], "gpu");
                assert_eq!(v["log"], true);
                assert!(!dir.read_dir().unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(".bad")));
            }
        });
    }

    #[test]
    fn broken_ui_json_gets_bad_copy_and_unreadable_is_untouched() {
        with_cfg("bad", |dir| {
            std::fs::write(dir.join("ui.json"), "{ не json").unwrap();
            update(|v| v["log"] = serde_json::json!(true));
            let bads = || -> Vec<std::path::PathBuf> {
                std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains("ui.json.bad.")).collect()
            };
            assert_eq!(bads().len(), 1);
            assert_eq!(std::fs::read_to_string(&bads()[0]).unwrap(), "{ не json");
            // повторная порча не затирает прежнюю копию; хранятся три последние
            for i in 0..5 {
                std::fs::write(dir.join("ui.json"), format!("{{ bad json {i}")).unwrap();
                update(|v| v["log"] = serde_json::json!(true));
            }
            assert_eq!(bads().len(), 3);
            assert!(
                bads().iter().any(|p| std::fs::read_to_string(p).unwrap() == "{ bad json 4"),
                "{:?}",
                bads().iter().map(|p| (p.clone(), std::fs::read_to_string(p).unwrap())).collect::<Vec<_>>()
            );
            // ссылка на месте будущей копии не пройдёт: чужой файл не затирается
            #[cfg(unix)]
            {
                let victim = dir.join("victim.txt");
                std::fs::write(&victim, "INTACT").unwrap();
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                for k in 0..3 {
                    let _ = std::os::unix::fs::symlink(&victim, dir.join(format!("ui.json.bad.{}", secs + k)));
                }
                std::fs::write(dir.join("ui.json"), "{ again").unwrap();
                update(|v| v["log"] = serde_json::json!(true));
                assert_eq!(std::fs::read_to_string(&victim).unwrap(), "INTACT");
            }
            // каталог на месте файла: не читается — ничего не пишем, ошибка доходит до пользователя
            std::fs::remove_file(dir.join("ui.json")).unwrap();
            std::fs::create_dir(dir.join("ui.json")).unwrap();
            let _ = take_error();
            update(|v| v["x"] = serde_json::json!(1));
            assert!(dir.join("ui.json").is_dir());
            assert!(take_error().is_some());
        });
    }
}
