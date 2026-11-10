//! Язык интерфейса: русский (исходный) или английский.
//!
//! Внутри программа «думает» по-русски: проверки, кэш, виджет и логика сравнивают русские строки
//! (`title == "Процессор"`). Поэтому переводим только на границе показа — когда текст рисуется
//! (gfx::Fonts), уходит в заголовок окна, экранный диктор, терминал или файл виджета macOS.
//!
//! Таблицы — engine/i18n/en/*.txt (build.rs склеивает их в одну), формат:
//!   ru: Проверить всё           — точная строка
//!   en: Check all
//!   ru: Скорость {} Мбит/с      — шаблон: {} — подставленное значение (format!), его тоже переводим
//!   en: Speed {} Mbit/s         — {} по порядку или {0}, {1} — по номеру, если порядок другой
//!   ru: ключ|ключа|ключей       — формы числа для diag::plural (1, 2–4, 5+)
//!   en: key|keys
//! Внутри строк: \n — перевод строки, \\ — обратная косая черта, {{ и }} — фигурные скобки.
//! Проверка покрытия: tools/i18n.py.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    pub fn id(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }
    fn from_id(s: &str) -> Option<Lang> {
        match s {
            "ru" => Some(Lang::Ru),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
}

const TABLE_SRC: &str = include_str!(concat!(env!("OUT_DIR"), "/i18n_en.txt"));

static LANG: AtomicU8 = AtomicU8::new(0);
static CHECKED_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Текущий язык. Выбор из настроек («lang» в ui.json) перечитывается не чаще раза в секунду —
/// так окно и виджет (отдельные процессы) переключаются вместе.
pub fn lang() -> Lang {
    // в тестах язык не зависит от системы (на CI система английская): по-русски, если тест не попросил иначе
    #[cfg(test)]
    {
        return TEST_LANG.with(|l| l.get());
    }
    #[allow(unreachable_code)]
    let cur = LANG.load(Ordering::Relaxed);
    let now = now_ms();
    if cur != 0 && now.saturating_sub(CHECKED_MS.load(Ordering::Relaxed)) < 1000 {
        return if cur == 2 { Lang::En } else { Lang::Ru };
    }
    CHECKED_MS.store(now, Ordering::Relaxed);
    // APTECHKA_LANG=ru|en (проверки, скриншоты) важнее выбора в меню
    let forced = std::env::var("APTECHKA_LANG").ok().as_deref().and_then(Lang::from_id);
    let l = forced.or_else(chosen).unwrap_or_else(system);
    LANG.store(if l == Lang::En { 2 } else { 1 }, Ordering::Relaxed);
    l
}

#[cfg(test)]
thread_local! {
    /// язык для тестов этого потока (см. lang)
    pub static TEST_LANG: std::cell::Cell<Lang> = const { std::cell::Cell::new(Lang::Ru) };
}

/// Язык, выбранный вручную (меню «Вид»), если выбирали.
pub fn chosen() -> Option<Lang> {
    crate::prefs::load().get("lang").and_then(|v| v.as_str()).and_then(Lang::from_id)
}

/// Выбрать язык вручную; None — снова «как в системе».
pub fn set(l: Option<Lang>) {
    crate::prefs::update(|v| match l {
        Some(l) => v["lang"] = serde_json::json!(l.id()),
        None => {
            if let Some(o) = v.as_object_mut() {
                o.remove("lang");
            }
        }
    });
    LANG.store(0, Ordering::Relaxed);
}

/// Язык системы: русский для русскоязычной системы, иначе английский. APTECHKA_LANG=ru|en — для проверок.
pub fn system() -> Lang {
    static SYS: OnceLock<Lang> = OnceLock::new();
    *SYS.get_or_init(|| {
        if let Some(l) = std::env::var("APTECHKA_LANG").ok().as_deref().and_then(Lang::from_id) {
            return l;
        }
        let tag = system_tag().unwrap_or_default();
        if is_russian_tag(&tag) {
            Lang::Ru
        } else {
            Lang::En
        }
    })
}

/// «ru_RU.UTF-8», «ru-RU», «be» … → считать русскоязычной? (белорусский, казахский, киргизский
/// интерфейсы тоже обычно с русским вторым языком, перевода на них нет)
fn is_russian_tag(tag: &str) -> bool {
    let t = tag.trim().to_ascii_lowercase();
    let lang = t.split(['_', '-', '.', '@']).next().unwrap_or("");
    matches!(lang, "ru" | "be" | "kk" | "ky")
}

#[cfg(windows)]
fn system_tag() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    // SAFETY: функция без параметров, только возвращает число
    let id = unsafe { GetUserDefaultUILanguage() } & 0x3ff;
    Some(
        match id {
            0x19 => "ru",
            0x23 => "be",
            0x3f => "kk",
            0x40 => "ky",
            _ => "en",
        }
        .to_string(),
    )
}

#[cfg(not(windows))]
fn system_tag() -> Option<String> {
    // macOS: программа из Finder не получает LANG — язык интерфейса берём из AppleLanguages
    if cfg!(target_os = "macos") {
        let out = crate::cmd::exec(&["/usr/bin/defaults", "read", "-g", "AppleLanguages"], std::time::Duration::from_secs(3), crate::cmd::CHECK);
        // вывод: ( "ru-RU", "en-US" ) — первый в списке и есть язык интерфейса
        if let Some(first) = out.text().split('"').nth(1) {
            return Some(first.to_string());
        }
    }
    // порядок как у gettext: LANGUAGE (список через «:»), затем LC_ALL, LC_MESSAGES, LANG
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C."));
    let base = get("LC_ALL").or_else(|| get("LC_MESSAGES")).or_else(|| get("LANG"));
    // LANGUAGE действует, только если сама локаль не «C»
    if base.is_some() {
        if let Some(first) = get("LANGUAGE").and_then(|l| l.split(':').next().map(str::to_string)) {
            return Some(first);
        }
    }
    base
}

// ───────────────────────────── таблица ─────────────────────────────

struct Tpl {
    /// куски текста между {} (их на один больше, чем подстановок)
    pieces: Vec<String>,
    en: String,
    /// длина текста без подстановок — более подробный шаблон пробуем раньше
    weight: usize,
}

#[derive(Default)]
struct Table {
    exact: HashMap<String, String>,
    tpls: Vec<Tpl>,
    /// форма слова → (английское для 1, для остальных)
    plural: HashMap<String, (String, String)>,
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('s') => out.push(' '),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Куски шаблона между {}; None — подстановок нет (обычная строка). {{ и }} — сами скобки.
fn split_tpl(s: &str) -> Option<Vec<String>> {
    let mut pieces = vec![String::new()];
    let mut holes = 0;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                pieces.last_mut().unwrap().push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                pieces.last_mut().unwrap().push('}');
            }
            '{' => {
                // {} или {что-угодно} — подстановка
                for d in it.by_ref() {
                    if d == '}' {
                        break;
                    }
                }
                holes += 1;
                pieces.push(String::new());
            }
            _ => pieces.last_mut().unwrap().push(c),
        }
    }
    (holes > 0).then_some(pieces)
}

/// Обычная строка без подстановок: {{ → {, }} → }.
fn plain(s: &str) -> String {
    s.replace("{{", "{").replace("}}", "}")
}

fn parse(src: &str) -> Table {
    let mut t = Table::default();
    let mut ru: Option<String> = None;
    for line in src.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if let Some(r) = line.strip_prefix("ru: ") {
            ru = Some(unescape(r));
        } else if let Some(e) = line.strip_prefix("en: ") {
            let Some(r) = ru.take() else { continue };
            let e = unescape(e);
            if e.is_empty() {
                continue; // ещё не переведено
            }
            let rf: Vec<&str> = r.split('|').collect();
            let ef: Vec<&str> = e.split('|').collect();
            if rf.len() == 3 && ef.len() == 2 {
                for f in rf {
                    t.plural.insert(f.to_string(), (ef[0].to_string(), ef[1].to_string()));
                }
            } else if let Some(pieces) = split_tpl(&r) {
                let weight = pieces.iter().map(|p| p.chars().count()).sum();
                t.tpls.push(Tpl { pieces, en: e, weight });
            } else {
                t.exact.insert(plain(&r), plain(&e));
            }
        }
    }
    t.tpls.sort_by_key(|x| std::cmp::Reverse(x.weight));
    t
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| parse(TABLE_SRC))
}

/// Подставить значения в английский шаблон: {} по порядку, {0} {1} — по номеру.
fn fill(en: &str, vals: &[String]) -> String {
    let mut out = String::new();
    let mut next = 0;
    let mut it = en.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '{' if it.peek() == Some(&'{') => {
                it.next();
                out.push('{');
            }
            '}' if it.peek() == Some(&'}') => {
                it.next();
                out.push('}');
            }
            '{' => {
                let mut inner = String::new();
                for d in it.by_ref() {
                    if d == '}' {
                        break;
                    }
                    inner.push(d);
                }
                let i = inner.trim().parse::<usize>().unwrap_or_else(|_| {
                    next += 1;
                    next - 1
                });
                out.push_str(vals.get(i).map_or("", String::as_str));
            }
            _ => out.push(c),
        }
    }
    out
}

/// Сопоставить строку с шаблоном: значения подстановок или None.
fn capture(pieces: &[String], s: &str) -> Option<Vec<String>> {
    let first = &pieces[0];
    let last = &pieces[pieces.len() - 1];
    let mut rest = s.strip_prefix(first.as_str())?;
    if rest.len() < last.len() || !rest.ends_with(last.as_str()) {
        return None;
    }
    rest = &rest[..rest.len() - last.len()];
    let mut vals = Vec::with_capacity(pieces.len() - 1);
    for p in &pieces[1..pieces.len() - 1] {
        // ближайшее вхождение (подстановка — как можно короче); пустой кусок между {}{} не бывает в наших шаблонах
        let i = if p.is_empty() { 0 } else { rest.find(p.as_str())? };
        vals.push(rest[..i].to_string());
        rest = &rest[i + p.len()..];
    }
    vals.push(rest.to_string());
    Some(vals)
}

fn has_cyr(s: &str) -> bool {
    s.chars().any(|c| matches!(c, 'А'..='я' | 'Ё' | 'ё'))
}

fn translate(t: &Table, s: &str, depth: u8) -> String {
    if !has_cyr(s) || depth > 6 {
        return s.to_string();
    }
    if let Some(e) = t.exact.get(s) {
        return e.clone();
    }
    // пробелы по краям — не часть фразы
    let core = s.trim();
    if core.len() != s.len() && !core.is_empty() {
        let i = s.find(core).unwrap_or(0);
        return format!("{}{}{}", &s[..i], translate(t, core, depth + 1), &s[i + core.len()..]);
    }
    // «5 ключей» из diag::plural
    if let Some((n, w)) = s.split_once(' ') {
        if let (Ok(n), Some((one, many))) = (n.parse::<i64>(), t.plural.get(w)) {
            return format!("{n} {}", if n == 1 { one } else { many });
        }
    }
    for tp in &t.tpls {
        if let Some(vals) = capture(&tp.pieces, s) {
            let vals: Vec<String> = vals.iter().map(|v| translate(t, v, depth + 1)).collect();
            return fill(&tp.en, &vals);
        }
    }
    // составная строка: части через разделители — переводим по частям
    for sep in ["\n", " · ", " — ", "; ", ". ", ", ", ": "] {
        if s.contains(sep) {
            let parts: Vec<String> = s.split(sep).map(|p| translate(t, p, depth + 1)).collect();
            return parts.join(sep);
        }
    }
    s.to_string()
}

/// Непереведённые строки — в файл APTECHKA_I18N_MISSING (только для проверки перевода в VM).
fn missing(s: &str) {
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Some(p) = PATH.get_or_init(|| std::env::var_os("APTECHKA_I18N_MISSING").map(Into::into)) else { return };
    let mut g = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if g.get_or_insert_with(HashSet::new).insert(s.to_string()) {
        use std::io::Write;
        // без перехода по ссылке, права 0600, файл не больше 1 МБ: журнал может содержать пользовательский текст
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        if let Ok(mut f) = o.open(p) {
            if f.metadata().is_ok_and(|m| m.is_file() && m.len() < (1 << 20)) {
                let _ = writeln!(f, "{}", s.replace('\\', "\\\\").replace('\n', "\\n"));
            }
        }
    }
}

/// Перевести строку для показа. По-русски (и для строк без кириллицы) — та же строка без копирования.
pub fn tr(s: &str) -> Cow<'_, str> {
    // одна буква — часть уже разобранной строки (рисование по буквам), фразой она не бывает
    if lang() == Lang::Ru || !has_cyr(s) || s.chars().nth(1).is_none() {
        return Cow::Borrowed(s);
    }
    static CACHE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    if let Some(v) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|c| c.get(s)) {
        return Cow::Owned(v.clone());
    }
    let v = translate(table(), s, 0);
    // в журнал — строка целиком (по кускам после разбивки не понять, откуда она)
    if has_cyr(&v) {
        missing(s);
    }
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.get_or_insert_with(HashMap::new);
    // меняющиеся строки (проценты, время) не должны копиться без конца
    if c.len() > 4096 {
        c.clear();
    }
    c.insert(s.to_string(), v.clone());
    Cow::Owned(v)
}

/// Перевод сразу в String.
pub fn t(s: &str) -> String {
    tr(s).into_owned()
}

/// Две строки на выбор: короткие подписи, которых нет в русской логике (например, имя программы).
pub fn pick(ru: &'static str, en: &'static str) -> &'static str {
    if lang() == Lang::En {
        en
    } else {
        ru
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(src: &str) -> Table {
        parse(src)
    }

    #[test]
    fn exact_template_plural_and_parts() {
        let t = tb("ru: Проверить всё\nen: Check all\n\
                    ru: Скорость {} Мбит/с\nen: Speed {} Mbit/s\n\
                    ru: Нашёл {} в {}\nen: Found {0} in {1}\n\
                    ru: ключ|ключа|ключей\nen: key|keys\n\
                    ru: файл|файла|файлов\nen: file|files\n\
                    ru: в норме\nen: fine\n\
                    ru: Пусто {{x}}\nen: Empty {{x}}\n\
                    ru: Не переведено\nen: \n");
        assert_eq!(translate(&t, "Проверить всё", 0), "Check all");
        assert_eq!(translate(&t, "Скорость 12,5 Мбит/с", 0), "Speed 12,5 Mbit/s");
        assert_eq!(translate(&t, "Нашёл 5 ключей в 1 файл", 0), "Found 5 keys in 1 file");
        assert_eq!(translate(&t, "Нашёл 21 ключ в 3 файла", 0), "Found 21 keys in 3 files");
        assert_eq!(translate(&t, "в норме · Проверить всё", 0), "fine · Check all");
        assert_eq!(translate(&t, "  в норме ", 0), "  fine ");
        assert_eq!(translate(&t, "Пусто {x}", 0), "Empty {x}");
        assert_eq!(translate(&t, "Не переведено", 0), "Не переведено");
        assert_eq!(translate(&t, "plain latin", 0), "plain latin");
    }

    #[test]
    fn russian_tags() {
        for t in ["ru_RU.UTF-8", "ru", "ru-RU", "be_BY", "kk-KZ"] {
            assert!(is_russian_tag(t), "{t}");
        }
        for t in ["en_US.UTF-8", "de", "", "rus", "uk_UA"] {
            assert!(!is_russian_tag(t), "{t}");
        }
    }

    /// Встроенные таблицы: число подстановок в переводе = в оригинале, формы числа — 3 и 2.
    #[test]
    fn shipped_tables_consistent() {
        let mut ru: Option<String> = None;
        let mut seen: HashMap<String, String> = HashMap::new();
        for (n, line) in TABLE_SRC.lines().enumerate() {
            if let Some(r) = line.strip_prefix("ru: ") {
                assert!(ru.is_none(), "строка {}: ru без en", n + 1);
                ru = Some(r.to_string());
            } else if let Some(e) = line.strip_prefix("en: ") {
                let r = ru.take().unwrap_or_else(|| panic!("строка {}: en без ru", n + 1));
                if e.is_empty() {
                    continue;
                }
                let holes = |s: &str| split_tpl(&unescape(s)).map_or(0, |p| p.len() - 1);
                let (rf, ef) = (r.split('|').count(), e.split('|').count());
                if !(rf == 3 && ef == 2) {
                    assert_eq!(holes(&r), holes(e), "строка {}: разное число {{}} в «{r}» и «{e}»", n + 1);
                }
                if let Some(prev) = seen.insert(r.clone(), e.to_string()) {
                    assert_eq!(prev, e, "строка {}: «{r}» переведено по-разному", n + 1);
                }
            }
        }
    }
}
