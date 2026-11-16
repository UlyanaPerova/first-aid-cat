//! Цифровой след: что из секретов человека лежит открытым текстом и кому оно доступно.
//!
//! Честное ограничение: троян, работающий под этим же пользователем, прочитает всё, что может прочитать сам
//! человек, — права на файлы от него не спасают. Поэтому проверка о другом: что лежит открытым текстом
//! (и уйдёт целиком одним махом — при краже, в резервную копию, в облако, в чужой скрипт), что видят другие
//! пользователи этого компьютера, и что лучше убрать.
//!
//! Что смотрим:
//! - ключи API и токены: в истории команд, в файлах оболочки, в .env, в чатах ИИ-агентов (их видела модель —
//!   такой ключ считай засвеченным), в файлах входа программ (git, npm, pip, docker, aws, gh) и самих агентов;
//! - доступы к VPN: файлы с адресами серверов и ключами (Happ, v2ray, WireGuard, Outline, Hiddify, AmneziaVPN);
//! - ключи SSH без пароля;
//! - домашняя папка, открытая другим пользователям; шифрование диска.
//!
//! Сами секреты Аптечка не показывает и никуда не пишет: только вид («ключ OpenAI»), файл и сколько.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

#[cfg_attr(not(unix), allow(unused_imports))]
use crate::diag::{fx, Check, BAD, INFO, OK, WARN};

/// Вид секрета по началу строки: (начало, название, сколько знаков после начала минимум).
const KINDS: [(&str, &str, usize); 15] = [
    ("sk-ant-", "Anthropic", 40),
    ("sk-proj-", "OpenAI", 40),
    ("sk-svcacct-", "OpenAI", 40),
    ("sk-or-v1-", "OpenRouter", 40),
    ("sk-", "OpenAI или похожий", 40),
    ("github_pat_", "GitHub", 40),
    ("ghp_", "GitHub", 30),
    ("gho_", "GitHub", 30),
    ("glpat-", "GitLab", 18),
    ("xoxb-", "Slack", 30),
    ("xoxp-", "Slack", 30),
    ("AKIA", "AWS", 16),
    ("AIza", "Google", 35),
    ("hf_", "Hugging Face", 30),
    ("-----BEGIN", "закрытый ключ", 0),
];

const TAIL_MAX: usize = 256;

/// Где в тексте лежит секрет: байты [start, end), первые `keep` из них — узнаваемое начало («sk-proj-»),
/// `first` — первый кусок секрета (у закрытого ключа кусков несколько — по строке base64).
pub(crate) struct Span {
    pub start: usize,
    pub end: usize,
    pub keep: usize,
    pub name: &'static str,
    pub first: bool,
}

/// Какие виды секретов есть в тексте (без самих значений).
pub fn secret_kinds(text: &str) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    for s in secret_spans(text) {
        if !found.contains(&s.name) {
            found.push(s.name);
        }
    }
    found
}

/// Секреты в тексте вместе с местом (по ним Аптечка вымарывает ключи из чатов: убирает ровно то, о чём
/// говорит проверка).
pub(crate) fn secret_spans(text: &str) -> Vec<Span> {
    let mut found: Vec<Span> = Vec::new();
    let tok = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    for (prefix, name, min) in KINDS {
        let mut from = 0;
        while let Some(i) = text[from..].find(prefix) {
            let at = from + i;
            from = at + prefix.len();
            // начало слова: перед ключом не буква/цифра (иначе «task-…», «risk-…»)
            // и не внутри base64 (картинки в чатах): перед ключом не «+» и не «/»
            if text[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/') {
                continue;
            }
            if prefix == "-----BEGIN" {
                // настоящий ключ — заголовок и сразу данные (строка base64 ≥ 60 знаков), а не просто заголовок
                // в коде или в объяснении; в файлах чатов перевод строки записан как «\n»
                let line: String = text[at..].chars().take(60).collect();
                if let Some(h) = line.find("PRIVATE KEY-----").filter(|_| !line.contains("ENCRYPTED")) {
                    let after = &text[at + h + "PRIVATE KEY-----".len()..];
                    let after = after.trim_start_matches(['\r', '\n']).trim_start_matches("\\n");
                    let body = after.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=').count();
                    if body >= 60 {
                        pem_spans(text, at + h + "PRIVATE KEY-----".len(), name, &mut found);
                    }
                }
                continue;
            }
            // ключи короче 256 знаков; без предела «sk-sk-sk-…» читалась бы до конца для каждого «sk-» —
            // время росло как квадрат длины (100 КБ такого текста — минута)
            let tail: String = text[from..].chars().take_while(|c| tok(*c)).take(TAIL_MAX).collect();
            if tail.len() < min {
                continue;
            }
            // у ключей вперемешку буквы и цифры; «sk-aaaaaaaa…» и слова через дефис — не ключ
            let digits = tail.chars().filter(|c| c.is_ascii_digit()).count();
            let letters = tail.chars().filter(|c| c.is_ascii_alphabetic()).count();
            let upper = tail.chars().filter(|c| c.is_ascii_uppercase()).count();
            // примеры из документации и тестов: официальный пример AWS (…EXAMPLE), строки с повтором
            // (настоящий ключ — случайный, куски в нём не повторяются)
            let example = tail.contains("EXAMPLE") || tail.to_ascii_lowercase().contains("xxxx") || repeats(&tail);
            let plausible = !example
                && if prefix == "AIza" {
                    // ключ Google — ровно 39 знаков (AIza + 35)
                    tail.len() == 35 && !text[from + tail.len()..].starts_with(['+', '/'])
                } else if prefix == "AKIA" {
                    tail.len() >= 16 && tail.chars().take(16).all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                } else {
                    digits >= 3 && letters >= 8 && (upper >= 2 || prefix != "sk-")
                };
            if plausible {
                // «sk-ant-…» тоже начинается с «sk-»: общий вид не добавляем, если есть точный
                let generic = prefix == "sk-"
                    && (text[at..].starts_with("sk-ant-")
                        || text[at..].starts_with("sk-proj-")
                        || text[at..].starts_with("sk-svcacct-")
                        || text[at..].starts_with("sk-or-"));
                if !generic {
                    // ключ целиком, а не только первые TAIL_MAX знаков
                    let full = text[from..].bytes().take_while(|c| tok(*c as char)).count();
                    found.push(Span { start: at, end: from + full, keep: prefix.len(), name, first: true });
                }
            }
        }
    }
    found
}

/// Куски закрытого ключа: каждая строка base64 между заголовком и «-----END» (в чатах перевод строки —
/// «\n» из двух знаков; его самого не трогаем). Без «-----END» поблизости — только первая строка.
fn pem_spans(text: &str, header_end: usize, name: &'static str, out: &mut Vec<Span>) {
    let rest = &text[header_end..];
    let rb = rest.as_bytes();
    let b64 = |c: u8| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'=';
    let esc = |i: usize| rb[i] == b'\\' && matches!(rb.get(i + 1), Some(b'n' | b'r'));
    let mut lim = rest.len().min(20_000);
    while !rest.is_char_boundary(lim) {
        lim -= 1;
    }
    let mut i = 0;
    while i < lim && (matches!(rb[i], b'\r' | b'\n') || esc(i)) {
        i += if rb[i] == b'\\' { 2 } else { 1 };
    }
    let end = rest[..lim].find("-----END");
    let stop = end.unwrap_or(lim);
    let mut first = true;
    while i < stop {
        if b64(rb[i]) {
            let s = i;
            while i < stop && b64(rb[i]) {
                i += 1;
            }
            out.push(Span { start: header_end + s, end: header_end + i, keep: 0, name, first });
            first = false;
            if end.is_none() {
                break;
            }
        } else if esc(i) {
            i += 2;
        } else {
            i += 1;
        }
    }
}

/// В строке есть повторяющийся кусок из 8 знаков — так выглядят выдуманные примеры, не случайные ключи.
fn repeats(t: &str) -> bool {
    let b = t.as_bytes();
    if b.len() < 24 {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    b.windows(8).any(|w| !seen.insert(w))
}

/// Текст без секретов: всё, что распознаётся как ключ или токен, заменено на «•••» (для названий сессий
/// и прочего показа). Плюс общая чистка адресов и «ключ=значение» (proxyaddr::redact).
pub fn mask_secrets(text: &str) -> String {
    let mut out = text.to_string();
    let tok = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    for (prefix, _, min) in KINDS {
        if prefix == "-----BEGIN" {
            if let Some(i) = out.find("-----BEGIN") {
                out.truncate(i);
                out.push_str("•••");
            }
            continue;
        }
        let mut from = 0;
        while let Some(i) = out[from..].find(prefix) {
            let at = from + i;
            let tail_start = at + prefix.len();
            let tail_len: usize = out[tail_start..].chars().take_while(|c| tok(*c)).map(|c| c.len_utf8()).sum();
            if tail_len >= min.max(16) {
                out.replace_range(at..tail_start + tail_len, "•••");
                from = at + "•••".len();
            } else {
                from = tail_start;
            }
        }
    }
    crate::proxyaddr::redact(&out)
}

/// Копия файла оболочки для «backups/»: строки с ключами заменены меткой (копия живёт 7 дней и не должна
/// хранить то, что «Цифровой след» велит убрать), блоки «-----BEGIN … PRIVATE KEY-----» — целиком.
/// Остальные строки — как были, включая окончания.
pub fn redact_for_backup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_pem = false;
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let hidden = if in_pem || body.contains("-----BEGIN") {
            in_pem = !body.contains("-----END");
            true
        } else {
            !secret_kinds(body).is_empty() || decode_escapes(body).as_deref().is_some_and(|d| !secret_kinds(d).is_empty())
        };
        if hidden {
            out.push_str("# "); // метка для файла копии
            out.push_str("строка с ключом скрыта Аптечкой");
            out.push_str(&line[body.len()..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// Раскодировать \uXXXX (JSON) и %XX (адреса) — только если они в тексте есть; None — нечего раскодировать.
fn decode_escapes(t: &str) -> Option<String> {
    decode_escapes_map(t, None)
}

/// То же, а `map` (если дан) получает для каждого байта результата — откуда он в исходном тексте
/// (плюс в конце — длину исходного): так место ключа в раскодированном тексте находится и в самом файле.
fn decode_escapes_map(t: &str, mut map: Option<&mut Vec<usize>>) -> Option<String> {
    if !t.contains("\\u") && !t.contains('%') {
        return None;
    }
    let b = t.as_bytes();
    let mut out = String::with_capacity(t.len());
    let mut i = 0;
    let hex = |s: &[u8]| std::str::from_utf8(s).ok().and_then(|h| u32::from_str_radix(h, 16).ok());
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'u') && i + 6 <= b.len() {
            if let Some(c) = hex(&b[i + 2..i + 6]).and_then(char::from_u32) {
                out.push(c);
                note(&mut map, i, c);
                i += 6;
                continue;
            }
        }
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(c) = hex(&b[i + 1..i + 3]).filter(|c| *c < 0x80).and_then(char::from_u32) {
                out.push(c);
                note(&mut map, i, c);
                i += 3;
                continue;
            }
        }
        // остальное — как есть (по символам UTF-8)
        let ch = t[i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        note(&mut map, i, ch);
        i += ch.len_utf8().max(1);
    }
    if let Some(m) = map {
        m.push(t.len());
    }
    Some(out)
}

fn note(map: &mut Option<&mut Vec<usize>>, at: usize, c: char) {
    if let Some(m) = map {
        m.extend(std::iter::repeat_n(at, c.len_utf8()));
    }
}

/// Папки чатов ИИ-агентов (те же, что знает модуль агентов, agents.rs) и расширение файлов в них.
const CHAT_DIRS: [(&str, &str); 9] = [
    (".claude/projects", ".jsonl"),
    (".codex/sessions", ".jsonl"),
    (".gemini/tmp", ".json"),
    (".gemini/tmp", ".jsonl"),
    (".qwen/tmp", ".json"),
    (".qwen/projects", ".jsonl"),
    (".kimi/sessions", ".jsonl"),
    (".kimi-code", ".jsonl"),
    (".local/share/opencode/storage", ".json"),
];

/// Поиск в файле по кускам (большие чаты не читаются целиком в память), не дольше `until`.
/// None — не успели или не прочитать.
fn scan_file(p: &Path, until: Instant) -> Option<Vec<&'static str>> {
    scan_state(p, until).ok().flatten()
}

/// Низкий приоритет текущему потоку: Linux — nice 19 и диск «лучшее усилие, последний» (не IDLE: тот может
/// не дождаться диска вовсе); Windows — фоновый режим потока (и процессор, и диск). macOS — как есть.
pub(crate) fn background_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: обычные системные вызовы для своего потока (tid), без указателей
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 19);
        // IOPRIO_WHO_PROCESS = 1; класс 2 (best-effort) << 13 | уровень 7
        libc::syscall(libc::SYS_ioprio_set, 1, tid as libc::c_long, (2 << 13) | 7);
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentThread() -> *mut core::ffi::c_void;
            fn SetThreadPriority(h: *mut core::ffi::c_void, p: i32) -> i32;
        }
        const THREAD_MODE_BACKGROUND_BEGIN: i32 = 0x0001_0000;
        // SAFETY: псевдоописатель своего потока
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        }
    }
}

/// Linux: просмотр файла — подряд (система читает вперёд), а по окончании его страницы выбрасываются из
/// кэша: они чистые, ничего не теряется, просто не вытесняют чужое.
pub(crate) struct CacheDrop {
    #[cfg(target_os = "linux")]
    fd: i32,
}

impl CacheDrop {
    pub(crate) fn new(f: &std::fs::File) -> CacheDrop {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::io::AsRawFd;
            let fd = f.as_raw_fd();
            // SAFETY: fd живёт, пока жив f; CacheDrop создаётся после f и уничтожается раньше
            unsafe {
                libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_SEQUENTIAL);
            }
            CacheDrop { fd }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = f;
            CacheDrop {}
        }
    }
}

impl Drop for CacheDrop {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        // SAFETY: см. new — файл ещё открыт
        unsafe {
            libc::posix_fadvise(self.fd, 0, 0, libc::POSIX_FADV_DONTNEED);
        }
    }
}

/// Итог просмотра одного файла: Ok(Some) — просмотрен, Ok(None) — файла нет (это не «неполный охват»),
/// Err — файл есть, но прочитан не весь (не успели, нет прав, не обычный файл): «ключей нет» тут не доказано.
fn scan_state(p: &Path, until: Instant) -> Result<Option<Vec<&'static str>>, ()> {
    use std::io::Read;
    let mut f = match crate::store::open_regular(p) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    // читаем подряд один раз: система может читать вперёд, а после — выбросить эти страницы из кэша
    // (сотни мегабайт чатов не вытесняют из памяти то, что нужно другим программам)
    let _cache = CacheDrop::new(&f);
    let mut found: Vec<&'static str> = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut tail: Vec<u8> = Vec::new();
    loop {
        if Instant::now() > until {
            return Err(());
        }
        let n = f.read(&mut buf).map_err(|_| ())?;
        if n == 0 {
            break;
        }
        // стык кусков: хвост прошлого куска (200 байт) + новый — ключ на границе не потеряется
        let mut chunk = std::mem::take(&mut tail);
        chunk.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&chunk);
        // ключ мог быть записан с экранированием JSON (\u0073…) или в адресе (%73…): ищем и в раскодированном
        let decoded = decode_escapes(&text);
        for k in secret_kinds(&text).into_iter().chain(decoded.as_deref().map(secret_kinds).unwrap_or_default()) {
            if !found.contains(&k) {
                found.push(k);
            }
        }
        tail = chunk[chunk.len().saturating_sub(200)..].to_vec();
    }
    Ok(Some(found))
}

/// Память проверенных файлов: (размер, время изменения) → найденные виды. Без самих секретов. Только в памяти
/// процесса: на диск не пишется и со сменой сеанса пропадает.
type Seen = std::collections::HashMap<String, (u64, u64, Vec<String>)>;

static SEEN: std::sync::Mutex<Option<Seen>> = std::sync::Mutex::new(None);

fn load_seen() -> Seen {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

fn store_seen(s: &Seen) {
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
}

fn stamp(p: &Path) -> Option<(u64, u64)> {
    let m = std::fs::metadata(p).ok()?;
    // время с наносекундами: перезапись того же размера в ту же секунду тоже заметна
    Some((m.len(), m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64))
}

/// Файлы с расширением в папке (до глубины 4, без ссылок), новые первыми, не больше `max`.
/// Второе значение — список обрезан (файлов больше, чем просмотрим): «ключей не нашла» тогда неполно.
fn files_in(dir: &Path, exts: &[&str], max: usize) -> (Vec<PathBuf>, bool) {
    let mut out: Vec<(PathBuf, u64)> = Vec::new();
    let mut cut = false;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() && depth < 4 {
                stack.push((p, depth + 1));
            } else if ft.is_file() && exts.iter().any(|x| p.to_string_lossy().ends_with(x)) {
                let t = stamp(&p).map_or(0, |s| s.1);
                out.push((p, t));
            }
            if out.len() > 20_000 {
                cut = true;
                stack.clear();
                break;
            }
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.1));
    cut |= out.len() > max;
    (out.into_iter().take(max).map(|x| x.0).collect(), cut)
}

/// Права файла открывают его другим пользователям (группе или всем).
#[cfg(unix)]
fn open_to_others(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o077 != 0)
}
#[cfg(not(unix))]
fn open_to_others(_: &Path) -> bool {
    false
}

/// Есть ли на компьютере другие люди (учётные записи с домашней папкой и входом).
#[cfg(all(unix, not(target_os = "macos")))]
fn other_humans() -> usize {
    let me = crate::store::home_str();
    crate::store::read_text("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter(|l| {
            let f: Vec<&str> = l.split(':').collect();
            f.len() >= 7
                && f[2].parse::<u32>().is_ok_and(|u| (1000..60000).contains(&u))
                && f[5] != me
                && !f[6].ends_with("nologin")
                && !f[6].ends_with("false")
        })
        .count()
}

/// macOS: местные учётные записи живут не в /etc/passwd, а в службе каталогов — спрашиваем dscl (только чтение).
#[cfg(target_os = "macos")]
fn other_humans() -> usize {
    let me = crate::store::home_str();
    let me = me.rsplit('/').next().unwrap_or("").to_string();
    count_dscl_humans(&crate::diag::run(&["dscl", ".", "-list", "/Users", "UniqueID"], 5).1, &me)
}

/// Разбор вывода «dscl . -list /Users UniqueID» («имя  номер» по строке): обычные люди — номер от 501,
/// без служебных (_имя, nobody, daemon, root, Guest) и без хозяина окна.
#[cfg(any(target_os = "macos", test))]
fn count_dscl_humans(out: &str, me: &str) -> usize {
    out.lines()
        .filter(|l| {
            let mut f = l.split_whitespace();
            let (Some(name), Some(uid), None) = (f.next(), f.next(), f.next()) else { return false };
            uid.parse::<u32>().is_ok_and(|u| (501..60000).contains(&u))
                && name != me
                && !name.starts_with('_')
                && !["nobody", "daemon", "root", "Guest"].contains(&name)
        })
        .count()
}
#[cfg(not(unix))]
#[allow(dead_code)]
fn other_humans() -> usize {
    0
}

/// Файлы входа: (путь от домашней папки, чей).
const CREDENTIALS: [(&str, &str); 16] = [
    (".git-credentials", "git"),
    (".netrc", "netrc"),
    (".npmrc", "npm"),
    (".pypirc", "pip"),
    (".docker/config.json", "Docker"),
    (".aws/credentials", "AWS"),
    (".config/gh/hosts.yml", "GitHub CLI"),
    (".codex/auth.json", "Codex"),
    (".claude/.credentials.json", "Claude Code"),
    (".gemini/oauth_creds.json", "Gemini CLI"),
    (".config/opencode/auth.json", "OpenCode"),
    (".local/share/opencode/auth.json", "OpenCode"),
    (".kimi/credentials.json", "Kimi"),
    (".qwen/oauth_creds.json", "Qwen Code"),
    (".cursor/cli-config.json", "Cursor"),
    (".env", "файл .env"),
];

/// Настройки VPN-клиентов с адресами серверов и ключами доступа.
const VPN_FILES: [(&str, &str); 8] = [
    (".config/Happ/config.json", "Happ"),
    (".config/Happ", "Happ"),
    (".config/v2rayN", "v2rayN"),
    (".config/hiddify", "Hiddify"),
    (".local/share/app.hiddify.com", "Hiddify"),
    (".config/AmneziaVPN.ORG", "AmneziaVPN"),
    (".config/Outline", "Outline"),
    (".config/nekoray", "Nekoray"),
];

/// Истории команд оболочек и интерпретаторов.
const HISTORIES: [&str; 6] = [
    ".bash_history",
    ".zsh_history",
    ".local/share/fish/fish_history",
    ".python_history",
    ".node_repl_history",
    "AppData/Roaming/Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt",
];

pub fn check() -> Vec<Check> {
    let Some(h) = crate::store::home_dir() else { return Vec::new() };
    // поток проверки — свой на каждый запуск: снижаем ему приоритет процессора и диска, чтобы чтение сотен
    // мегабайт не мешало работать (на слабом ноутбуке с HDD особенно)
    background_priority();
    let mut out = Vec::new();
    // с низким приоритетом холодный проход дольше; не успели — «просмотрено не всё», память проверенного
    // сохраняется, и следующий проход доберёт остальное
    let until = Instant::now() + Duration::from_secs(60);
    let short = |p: &Path| p.strip_prefix(&h).map(|r| format!("~/{}", r.display())).unwrap_or_else(|_| p.display().to_string());
    let mut exposed: Vec<PathBuf> = Vec::new(); // открыты другим — для кнопки «закрыть»

    // 1. история команд: ключ, введённый в терминале, лежит там годами
    let mut hist: Vec<(String, Vec<&str>)> = Vec::new();
    // файлы, которые есть, но просмотрены не до конца (не успели, нет прав, не обычный файл)
    let mut skipped: Vec<String> = Vec::new();
    for f in HISTORIES {
        let p = h.join(f);
        match scan_state(&p, until) {
            Ok(Some(k)) if !k.is_empty() => hist.push((short(&p), k)),
            Err(()) => skipped.push(short(&p)),
            _ => {}
        }
    }
    if !hist.is_empty() {
        let kinds: Vec<&str> = hist.iter().flat_map(|x| x.1.iter().copied()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        out.push(
            Check::new("sys", "След: ключи в истории команд", WARN, &kinds.join(", "))
                .hint(&format!(
                    "В {} лежат ключи, набранные в терминале. Любая программа, запущенная от твоего имени, прочитает их за секунду. \
                Кнопка уберёт эти строки из истории; ключи лучше заменить новыми.",
                    hist.iter().map(|x| x.0.as_str()).collect::<Vec<_>>().join(", ")
                ))
                .fixes(vec![fx("footprint_history", "Убрать ключи из истории", None)]),
        );
    }

    // 2. файлы оболочки и .env: ключи в переменных окружения
    let mut env_hits: Vec<String> = Vec::new();
    for f in [".bashrc", ".profile", ".bash_profile", ".zshrc", ".zprofile", ".zshenv", ".config/fish/config.fish", ".env"] {
        let p = h.join(f);
        match scan_state(&p, until) {
            Ok(Some(k)) if !k.is_empty() => {
                env_hits.push(short(&p));
                if open_to_others(&p) {
                    exposed.push(p);
                }
            }
            Err(()) => skipped.push(short(&p)),
            _ => {}
        }
    }
    if !env_hits.is_empty() {
        let hint = if cfg!(target_os = "macos") {
            "Ключ в переменной окружения видят все программы, запущенные из терминала. Надёжнее — хранилище паролей системы (Связка ключей)."
        } else if cfg!(windows) {
            "Ключ в переменной окружения видят все программы, запущенные из терминала. Надёжнее — хранилище паролей системы (Диспетчер учётных данных)."
        } else {
            "Ключ в переменной окружения видят все программы, запущенные из терминала. Надёжнее — хранилище паролей системы («Пароли и ключи»)."
        };
        out.push(Check::new("sys", "След: ключи в настройках оболочки", INFO, &env_hits.join(", ")).hint(hint));
    }

    // 3. чаты ИИ-агентов: ключ, вставленный в чат, видела модель — его стоит заменить
    let mut seen = load_seen();
    let mut chats_with: Vec<(String, Vec<String>)> = Vec::new();
    let mut partial = false;
    let mut capped = false; // в какой-то папке чатов файлов больше лимита
                            // те же папки, что знает модуль агентов (agents.rs); SQLite (Cursor, новый OpenCode) не разбираем — честно
                            // отмечаем, что они не просмотрены
    let unsupported: Vec<&str> = [(".cursor/chats", "Cursor"), (".local/share/opencode/opencode.db", "OpenCode (база SQLite)")]
        .iter()
        .filter(|(p, _)| h.join(p).exists())
        .map(|(_, n)| *n)
        .collect();
    for (d, ext) in CHAT_DIRS {
        let (files, cut) = files_in(&h.join(d), &[ext], 3000);
        capped |= cut;
        for p in files {
            let key = p.to_string_lossy().into_owned();
            let Some((len, mt)) = stamp(&p) else { continue };
            let kinds = match seen.get(&key) {
                Some((l, m, k)) if *l == len && *m == mt => k.clone(),
                _ => match scan_file(&p, until) {
                    Some(k) => {
                        let k: Vec<String> = k.into_iter().map(String::from).collect();
                        seen.insert(key.clone(), (len, mt, k.clone()));
                        k
                    }
                    None => {
                        partial = true;
                        continue;
                    }
                },
            };
            if !kinds.is_empty() {
                chats_with.push((short(&p), kinds));
            }
        }
    }
    // забытые файлы — из памяти вон
    seen.retain(|k, _| Path::new(k).exists());
    store_seen(&seen);
    if !chats_with.is_empty() {
        let kinds: Vec<String> = chats_with.iter().flat_map(|x| x.1.iter().cloned()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        out.push(
            Check::new(
                "sys",
                "След: ключи в чатах ИИ",
                WARN,
                &format!("{} · {}", crate::diag::plural(chats_with.len(), "чат", "чата", "чатов"), kinds.join(", ")),
            )
            .fixes(vec![fx("footprint_chats", "Вымарать ключи из чатов", None)])
            .hint(&format!(
                "Эти ключи были в переписке с ИИ: их видела модель, и они лежат открытым текстом в файлах сессий (например, {}). \
                Замени ключи новыми у поставщика; кнопка заменит их в чатах маской, сами чаты останутся читаемыми.{}",
                chats_with.first().map(|x| x.0.as_str()).unwrap_or(""),
                if partial { " Проверил не все чаты — продолжу в следующий раз." } else { "" }
            )),
        );
    }

    // 4. файлы входа программ: так и должно быть, но закрыто от других пользователей
    let mut creds: Vec<&str> = Vec::new();
    for (f, who) in CREDENTIALS {
        let p = h.join(f);
        if !p.is_file() {
            continue;
        }
        if f == ".env" || f == ".npmrc" {
            match scan_state(&p, until) {
                Ok(Some(k)) if !k.is_empty() => {}
                Err(()) => {
                    if !skipped.contains(&short(&p)) {
                        skipped.push(short(&p));
                    }
                    continue;
                }
                _ => continue,
            }
        }
        if !creds.contains(&who) {
            creds.push(who);
        }
        if open_to_others(&p) {
            exposed.push(p);
        }
    }
    if !creds.is_empty() {
        out.push(Check::new("sys", "След: входы программ", INFO, &creds.join(", ")).hint(
            "Токены входа этих программ лежат файлами в домашней папке — так они устроены. Программа под твоим именем может их прочитать; \
                после выхода из учётной записи на сайте «выйди» и в программе, токен перестанет работать.",
        ));
    }

    // 5. VPN: адреса серверов и ключи доступа
    let mut vpn: Vec<&str> = Vec::new();
    for (f, who) in VPN_FILES {
        let p = h.join(f);
        if p.exists() && !vpn.contains(&who) {
            vpn.push(who);
            if open_to_others(&p) {
                exposed.push(p);
            }
        }
    }
    #[cfg(unix)]
    {
        // WireGuard: закрытый ключ в /etc/wireguard обычно root-only; читаемый — плохо
        if let Ok(rd) = std::fs::read_dir("/etc/wireguard") {
            if rd.flatten().any(|e| crate::store::read_capped(&e.path(), 1 << 16).is_ok_and(|t| t.contains("PrivateKey"))) {
                out.push(Check::new("sys", "След: ключ WireGuard", BAD, "читается без прав администратора")
                    .hint("Закрытый ключ WireGuard в /etc/wireguard виден обычным программам — по нему можно подключиться к твоему VPN. Выполни: sudo chmod 600 /etc/wireguard/*.conf"));
            }
        }
    }
    if !vpn.is_empty() {
        out.push(Check::new("sys", "След: доступы VPN", INFO, &vpn.join(", "))
            .hint("В настройках этих клиентов — адреса серверов и ключи доступа. Любая программа под твоим именем может их прочитать и подключиться к твоему VPN. \
                Если ключ мог утечь — перевыпусти подписку/ключ у поставщика."));
    }

    // 6. SSH-ключи без пароля
    let mut naked: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(h.join(".ssh")) {
        for e in rd.flatten() {
            let p = e.path();
            let Ok(t) = crate::store::read_capped(&p, 1 << 16) else { continue };
            if t.contains("PRIVATE KEY-----") && !ssh_key_encrypted(&t) {
                naked.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                if open_to_others(&p) {
                    exposed.push(p);
                }
            }
        }
    }
    if !naked.is_empty() {
        out.push(Check::new("sys", "След: ключи SSH без пароля", WARN, &naked.join(", "))
            .hint("Скопировав этот файл, можно войти на твои серверы и в GitHub. Поставь пароль на ключ: ssh-keygen -p -f ~/.ssh/<ключ> (ключ останется тем же)."));
    }

    // 7. домашняя папка для других людей на этом компьютере
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&h).map(|m| m.permissions().mode() & 0o777).unwrap_or(0);
        let others = other_humans();
        if others > 0 && mode & 0o005 != 0 {
            out.push(
                Check::new(
                    "sys",
                    "След: домашняя папка",
                    WARN,
                    &format!("открыта ещё {} на компьютере", crate::diag::plural(others, "пользователю", "пользователям", "пользователям")),
                )
                .hint(&format!("Права {mode:o}: другие учётные записи могут читать твои файлы. Закрыть: chmod 750 ~")),
            );
        }
    }

    // 8. шифрование диска: при краже ноутбука всё это читается без пароля
    if let Some(enc) = disk_encrypted() {
        out.push(if enc {
            Check::new("sys", "След: шифрование диска", OK, "включено")
        } else {
            let hint = if cfg!(target_os = "macos") {
                "Без шифрования украденный ноутбук читается целиком: ключи, чаты, пароли браузера. Шифрование включается в FileVault."
            } else if cfg!(windows) {
                "Без шифрования украденный ноутбук читается целиком: ключи, чаты, пароли браузера. Шифрование включается в BitLocker."
            } else {
                "Без шифрования украденный ноутбук читается целиком: ключи, чаты, пароли браузера. Шифрование включается при установке системы."
            };
            Check::new("sys", "След: шифрование диска", INFO, "выключено").hint(hint)
        });
    }

    // одно правило со кнопкой «Закрыть от других»
    exposed = exposed_files(until);
    if !exposed.is_empty() {
        // реально опасно, только если на компьютере есть другие люди и домашняя папка пускает их внутрь
        #[cfg(unix)]
        let reachable =
            other_humans() > 0 && std::fs::metadata(&h).is_ok_and(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o001 != 0);
        #[cfg(not(unix))]
        let reachable = false;
        let list = exposed.iter().take(4).map(|p| short(p)).collect::<Vec<_>>().join(", ");
        out.push(Check::new("sys", "След: файлы открыты другим", if reachable { WARN } else { INFO }, &crate::diag::plural(exposed.len(), "файл", "файла", "файлов"))
            .hint(&if reachable {
                format!("Их могут читать другие пользователи компьютера: {list}.")
            } else {
                format!("По правам их могли бы читать другие учётные записи ({list}), но сейчас других людей на компьютере нет или домашняя папка закрыта. Закрыть на будущее — кнопкой.")
            })
            .fixes(vec![fx("footprint_lock", "Закрыть от других", None)]));
    }
    // «чисто» — только если просмотрено всё; иначе честно «проверила не всё» (отсутствие находок в части файлов
    // ничего не доказывает про остальные)
    if partial || capped || !skipped.is_empty() || !unsupported.is_empty() {
        let mut what = Vec::new();
        if partial {
            what.push("часть чатов не успел просмотреть (продолжу в следующий раз)".to_string());
        }
        if capped {
            what.push("в папках чатов файлов больше, чем я просматриваю (самые свежие, до 3000 в каждой папке)".to_string());
        }
        if !skipped.is_empty() {
            what.push(format!("не смог прочитать до конца: {}", skipped.join(", ")));
        }
        if !unsupported.is_empty() {
            what.push(format!("не умею читать чаты: {}", unsupported.join(", ")));
        }
        out.push(
            Check::new("sys", "Цифровой след: охват", INFO, "проверил не всё")
                .unknown()
                .hint(&format!("{}. Если ключей не нашёл — это не значит, что их там нет.", what.join("; "))),
        );
    } else if out.iter().all(|c| c.status == INFO || c.status == OK) && hist.is_empty() && chats_with.is_empty() {
        out.insert(
            0,
            Check::new("sys", "Цифровой след", OK, "в просмотренном ключей на виду нет")
                .hint("Просмотрел историю команд, файлы оболочки и чаты ИИ-агентов. Это не доказательство, что ключей нет нигде."),
        );
    }
    out
}

/// Ключ SSH зашифрован паролем: старый формат — «ENCRYPTED» в заголовке, новый (openssh-key-v1) —
/// имя шифра в начале данных не «none».
fn ssh_key_encrypted(t: &str) -> bool {
    if t.contains("ENCRYPTED") {
        return true;
    }
    if !t.contains("BEGIN OPENSSH PRIVATE KEY") {
        return false;
    }
    let b64: String = t.lines().filter(|l| !l.starts_with("-----")).collect();
    let Some(raw) = b64_decode(&b64) else { return true };
    // «openssh-key-v1\0», затем длина (4 байта) и имя шифра
    let rest = raw.strip_prefix(b"openssh-key-v1\0").unwrap_or(&[]);
    if rest.len() < 8 {
        return true;
    }
    let n = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    rest.get(4..4 + n).is_some_and(|c| c != b"none")
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()).take(400).collect();
    let mut out = Vec::new();
    for ch in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, c) in ch.iter().enumerate() {
            n |= val(*c)? << (18 - 6 * i);
        }
        for i in 0..ch.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// Зашифрован ли диск с домашней папкой. None — не узнать.
fn disk_encrypted() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        // устройство домашней папки в цепочке lsblk — тип «crypt» (LUKS)
        let r = crate::cmd::exec(&["lsblk", "-rno", "TYPE,MOUNTPOINTS"], Duration::from_secs(5), crate::cmd::CHECK);
        if r.code() != 0 {
            return None;
        }
        let tree = crate::cmd::exec(&["lsblk", "-sJo", "TYPE,MOUNTPOINTS"], Duration::from_secs(5), crate::cmd::CHECK);
        let j: serde_json::Value = serde_json::from_str(tree.text()).ok()?;
        let home = crate::store::home_str();
        // ищем раздел с самой длинной точкой подключения, с которой начинается домашняя папка, и смотрим его предков
        fn walk(v: &serde_json::Value, crypt_above: bool, home: &str, best: &mut (usize, bool)) {
            let ty = v["type"].as_str().unwrap_or("");
            let crypt = crypt_above || ty == "crypt";
            for m in v["mountpoints"].as_array().into_iter().flatten().filter_map(|m| m.as_str()) {
                if std::path::Path::new(home).starts_with(m) && m.len() >= best.0 {
                    *best = (m.len(), crypt);
                }
            }
            for c in v["children"].as_array().into_iter().flatten() {
                walk(c, crypt, home, best);
            }
        }
        let mut best = (0, false);
        for d in j["blockdevices"].as_array()? {
            // -s: дерево «вверх» — от раздела к дискам; crypt может быть ниже точки подключения
            fn has_crypt(v: &serde_json::Value) -> bool {
                v["type"].as_str() == Some("crypt") || v["children"].as_array().into_iter().flatten().any(has_crypt)
            }
            let c = has_crypt(d);
            walk(d, c, &home, &mut best);
        }
        (best.0 > 0).then_some(best.1)
    }
    #[cfg(target_os = "macos")]
    {
        // fdesetup isactive: код 0 — FileVault включён, 1 — выключен (прав администратора не нужно)
        let r = crate::cmd::exec(&["/usr/bin/fdesetup", "isactive"], Duration::from_secs(5), crate::cmd::CHECK);
        r.answered().then(|| r.code() == 0)
    }
    #[cfg(windows)]
    {
        None
    }
}

/// Кнопка «Закрыть от других»: убрать права группы и остальных у найденных открытых файлов
/// (только у тех, что нашла сама проверка, — список пересчитывается, аргументов нет).
pub fn fix_lock() -> (bool, String) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        // ровно те файлы, что показала проверка (а не все похожие): файл без секретов с намеренно общими правами
        // не трогаем
        let list = exposed_files(Instant::now() + Duration::from_secs(10));
        extern "C" {
            fn getuid() -> u32;
        }
        let me = unsafe { getuid() };
        let mut n = 0;
        let mut skipped: Vec<String> = Vec::new(); // файлы из проверки, которые не удалось закрыть
        let mut changed: Vec<String> = Vec::new();
        for p in list {
            // открываем без перехода по ссылке и меняем права у открытого объекта (fchmod): подменить файл между
            // проверкой и сменой прав нельзя
            let flags = libc::O_NONBLOCK | libc::O_NOFOLLOW;
            let Ok(f) = std::fs::OpenOptions::new().read(true).custom_flags(flags).open(&p) else {
                skipped.push(p.display().to_string());
                continue;
            };
            let Ok(m) = f.metadata() else {
                skipped.push(p.display().to_string());
                continue;
            };
            if m.permissions().mode() & 0o077 == 0 {
                continue;
            }
            if m.uid() != me {
                skipped.push(p.display().to_string());
                continue;
            }
            let (old, new) = (m.permissions().mode() & 0o777, m.permissions().mode() & 0o700);
            if crate::fixes::dry_run() {
                crate::fixes::log_dry(&format!("chmod {new:o} {}", p.display()));
                n += 1;
            } else if f.set_permissions(std::fs::Permissions::from_mode(new)).is_ok() {
                n += 1;
                changed.push(format!("{} {old:o}→{new:o}", p.display()));
            } else {
                skipped.push(p.display().to_string());
            }
        }
        if !changed.is_empty() {
            // что поменяли — в журнал: права можно вернуть вручную (chmod старое значение)
            crate::fixes::log(&format!("права изменены: {}", changed.join("; ")));
        }
        let done = format!("Закрыл от других пользователей: {}.", crate::diag::plural(n, "файл", "файла", "файлов"));
        if skipped.is_empty() {
            (true, done)
        } else {
            // файл из списка проверки остался открытым — не «всё хорошо»
            let rest = format!("Не смог закрыть ({}): {} — это ссылка, чужой файл или нет прав.", skipped.len(), skipped.join(", "));
            (false, format!("{done} {rest}"))
        }
    }
    #[cfg(not(unix))]
    {
        (false, "На этой системе права устроены иначе.".into())
    }
}

/// Файлы с секретами, открытые другим пользователям: файлы оболочки с ключами, файлы входа, настройки VPN,
/// ключи SSH без пароля. Одно правило для строки проверки и для кнопки «Закрыть от других».
fn exposed_files(until: Instant) -> Vec<PathBuf> {
    let Some(h) = crate::store::home_dir() else { return Vec::new() };
    let mut out = Vec::new();
    for f in [".bashrc", ".profile", ".bash_profile", ".zshrc", ".zprofile", ".zshenv", ".config/fish/config.fish", ".env"] {
        let p = h.join(f);
        if open_to_others(&p) && scan_file(&p, until).is_some_and(|k| !k.is_empty()) {
            out.push(p);
        }
    }
    for (f, _) in CREDENTIALS.iter().chain(VPN_FILES.iter()) {
        let p = h.join(f);
        if p.exists() && open_to_others(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    if let Ok(rd) = std::fs::read_dir(h.join(".ssh")) {
        for e in rd.flatten() {
            let p = e.path();
            if open_to_others(&p) && crate::store::read_capped(&p, 1 << 16).is_ok_and(|t| t.contains("PRIVATE KEY-----") && !ssh_key_encrypted(&t)) {
                out.push(p);
            }
        }
    }
    out
}

/// Кнопка «Убрать ключи из истории»: строки с ключами удаляются из файлов истории команд.
/// Копию не делаем — она хранила бы те же ключи.
pub fn fix_history() -> (bool, String) {
    let Some(h) = crate::store::home_dir() else { return (false, "Не нашёл домашнюю папку.".into()) };
    let mut removed = 0;
    let mut failed: Vec<String> = Vec::new(); // файлы истории, которые не удалось обработать
    for f in HISTORIES {
        let link = h.join(f);
        let p = match history_target(&link, &h) {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(why) => {
                failed.push(format!("{} ({why})", link.display()));
                continue;
            }
        };
        // до двух попыток: если оболочка дописала историю, пока мы её разбирали, — перечитать, а не затереть
        for attempt in 0..2 {
            let before = stamp(&p);
            // байтами: в истории бывают cp1251/koi8 и «метафицированные» байты zsh — строгий UTF-8 их не читает
            let t = match crate::store::read_bytes(&p, 64 << 20) {
                Ok(t) => t,
                Err(e) => {
                    failed.push(format!("{} ({e})", link.display()));
                    break;
                }
            };
            let (keep, n) = drop_secret_records(&t, f.contains("fish"));
            if n == 0 {
                break;
            }
            if crate::fixes::dry_run() {
                crate::fixes::log_dry(&format!("убрать {n} записей из {}", p.display()));
                removed += n;
                break;
            }
            if stamp(&p) != before {
                if attempt == 1 {
                    return (false, format!("{} всё время меняется (открыт терминал?) — закрой терминалы и нажми ещё раз.", p.display()));
                }
                continue;
            }
            #[cfg(unix)]
            let mode = std::fs::metadata(&p).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o700).unwrap_or(0o600);
            #[cfg(not(unix))]
            let mode = 0o600;
            if let Err(e) = crate::store::write_mode(&p, &keep, mode) {
                return (false, format!("Не удалось переписать {}: {e}", p.display()));
            }
            removed += n;
            break;
        }
    }
    if !failed.is_empty() {
        // файл из проверки не обработан: «ключей нет» сказать нельзя
        return (
            false,
            format!(
                "Не смог обработать: {} — записи с ключами там могли остаться. Убрано из остальных: {removed}. Проверь эти файлы вручную.",
                failed.join(", ")
            ),
        );
    }
    if removed == 0 {
        return (true, "В истории команд не нашёл записей с ключами.".into());
    }
    (
        true,
        format!(
            "Убрал из истории {}. Открытые терминалы помнят историю и могут записать её снова — закрой их. \
        Удаление строки не отзывает ключ: замени его у поставщика.",
            crate::diag::plural(removed, "запись", "записи", "записей")
        ),
    )
}

/// Закрыть ключи в файле чата на месте: каждый ключ — на маску той же длины («sk-proj-» и звёздочки).
/// Размер файла и все смещения не меняются, поэтому строки, JSON и агент, дописывающий этот чат
/// прямо сейчас, ничего не замечают. Через временный файл и переименование нельзя: открытая сессия
/// осталась бы с удалённым файлом. Копию не делаем — она хранила бы ключи.
/// Ok((сколько ключей закрыто, остался ли ключ после закрытия)).
fn mask_file(p: &Path, dry: bool) -> Result<(usize, bool), String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    const CH: usize = 1 << 20; // сколько байт «своих» в одном проходе
    const OV: usize = 4096; // запас справа: ключ на стыке проходов целиком виден в одном из них
    const LEFT: usize = 8; // слева — знак перед ключом (начало слова)
    let why = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::PermissionDenied => "нет прав на запись".to_string(),
        _ => e.to_string(),
    };
    if std::fs::symlink_metadata(p).map_err(why)?.file_type().is_symlink() {
        return Err("это ссылка".into());
    }
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(!dry);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut o, libc::O_NONBLOCK | libc::O_NOFOLLOW);
    let mut f = match o.open(p) {
        Ok(f) => f,
        // писать нельзя — сначала посмотреть, есть ли что вымарывать: файл только для чтения без ключей — не сбой
        Err(e) if !dry && e.kind() == std::io::ErrorKind::PermissionDenied => {
            return match mask_file(p, true) {
                Ok((0, x)) => Ok((0, x)),
                _ => Err(why(e)),
            };
        }
        Err(e) => return Err(why(e)),
    };
    if !f.metadata().map_err(why)?.is_file() {
        return Err("не обычный файл".into());
    }
    let read_at = |f: &mut std::fs::File, at: usize, n: usize| -> std::io::Result<Vec<u8>> {
        f.seek(SeekFrom::Start(at as u64))?;
        let mut buf = Vec::with_capacity(n);
        Read::by_ref(f).take(n as u64).read_to_end(&mut buf)?;
        Ok(buf)
    };
    let mut keys = 0;
    let mut base = 0usize;
    loop {
        let from = base.saturating_sub(LEFT);
        let want = base - from + CH + OV;
        let buf = read_at(&mut f, from, want).map_err(why)?;
        let last = buf.len() < want;
        for (s, e, first) in raw_spans(&buf) {
            let (s, e) = (from + s, from + e);
            if s < base || (!last && s >= base + CH) {
                continue; // чужой проход: его найдёт соседний
            }
            if !last && e >= from + buf.len() {
                return Err("ключ длиннее запаса на стыке".into());
            }
            if first {
                keys += 1;
            }
            if dry {
                continue;
            }
            // перед записью — те же байты на месте (файл мог измениться), и только тогда маска
            let cur = read_at(&mut f, s, e - s).map_err(why)?;
            if cur != buf[s - from..e - from] {
                return Err("файл изменился во время работы".into());
            }
            f.seek(SeekFrom::Start(s as u64)).map_err(why)?;
            f.write_all(&vec![b'*'; e - s]).map_err(why)?;
        }
        if last {
            break;
        }
        base += CH;
    }
    if !dry {
        f.flush().map_err(why)?;
    }
    drop(f);
    // осталось ли что-то, что проверка всё ещё считает ключом (запись, которую не разобрали)
    let left = !dry && keys > 0 && scan_file(p, Instant::now() + Duration::from_secs(600)).is_some_and(|k| !k.is_empty());
    Ok((keys, left))
}

/// Байты [начало, конец) секретов в куске файла — что закрывать (`true` — начало нового ключа). Ищем и в
/// тексте как есть, и в раскодированном (\uXXXX, %XX): место из него переводится обратно в файл. Начало
/// ключа («sk-proj-») остаётся, закрывается остальное.
fn raw_spans(bytes: &[u8]) -> Vec<(usize, usize, bool)> {
    // текст как lossy, но с таблицей «байт текста → байт файла» (битые байты заменяются на 3-байтовый знак)
    let mut text = String::with_capacity(bytes.len());
    let mut tmap: Vec<usize> = Vec::with_capacity(bytes.len() + 1);
    let mut at = 0;
    for ch in bytes.utf8_chunks() {
        for c in ch.valid().chars() {
            tmap.extend(std::iter::repeat_n(at, c.len_utf8()));
            at += c.len_utf8();
            text.push(c);
        }
        if !ch.invalid().is_empty() {
            tmap.extend(std::iter::repeat_n(at, '\u{FFFD}'.len_utf8()));
            at += ch.invalid().len();
            text.push('\u{FFFD}');
        }
    }
    tmap.push(bytes.len());
    let mut out: Vec<(usize, usize, bool)> = secret_spans(&text).iter().map(|s| (tmap[s.start + s.keep], tmap[s.end], s.first)).collect();
    let mut dmap: Vec<usize> = Vec::new();
    if let Some(d) = decode_escapes_map(&text, Some(&mut dmap)) {
        out.extend(secret_spans(&d).iter().map(|s| (tmap[dmap[s.start + s.keep]], tmap[dmap[s.end]], s.first)));
    }
    out.retain(|x| x.0 < x.1);
    out.sort_unstable();
    // совпавшие и пересекающиеся куски (тот же ключ, найденный и так, и так) — в один
    let mut merged: Vec<(usize, usize, bool)> = Vec::new();
    for (s, e, f) in out {
        match merged.last_mut() {
            Some(m) if s < m.1 => {
                m.1 = m.1.max(e);
                m.2 |= f;
            }
            _ => merged.push((s, e, f)),
        }
    }
    merged
}

/// Забыть память проверенных файлов: после правки чатов строка проверки должна пересчитаться.
fn forget_seen() {
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Кнопка «Вымарать ключи из чатов»: во всех файлах чатов ИИ-агентов ключи заменяются маской той же длины.
pub fn fix_chats() -> (bool, String) {
    let Some(h) = crate::store::home_dir() else { return (false, "Не нашёл домашнюю папку.".into()) };
    let dry = crate::fixes::dry_run();
    let home_real = std::fs::canonicalize(&h).unwrap_or(h.clone());
    let (mut keys, mut changed) = (0usize, 0usize);
    // причина → сколько файлов
    let mut failed: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut seen_files = std::collections::HashSet::new();
    for (d, ext) in CHAT_DIRS {
        let dir = h.join(d);
        // папка чатов должна оставаться в домашней папке (ссылка наружу — не наши файлы)
        match std::fs::canonicalize(&dir) {
            Ok(r) if r.starts_with(&home_real) => {}
            Ok(_) => {
                *failed.entry("папка чатов ведёт за пределы домашней".into()).or_default() += 1;
                continue;
            }
            Err(_) => continue,
        }
        let (files, _) = files_in(&dir, &[ext], usize::MAX);
        for p in files {
            if !seen_files.insert(p.clone()) {
                continue;
            }
            match mask_file(&p, dry) {
                Ok((0, _)) => {}
                Ok((n, left)) => {
                    keys += n;
                    changed += 1;
                    if dry {
                        crate::fixes::log_dry(&format!("закрыть ключи ({n}) в {}", p.display()));
                    }
                    if left {
                        *failed.entry("ключ записан необычно и остался".into()).or_default() += 1;
                    }
                }
                Err(why) => *failed.entry(why).or_default() += 1,
            }
        }
    }
    if dry {
        return (
            true,
            format!(
                "Пробный запуск: закрыл бы {} в {}.",
                crate::diag::plural(keys, "ключ", "ключа", "ключей"),
                crate::diag::plural(changed, "чате", "чатах", "чатах")
            ),
        );
    }
    forget_seen();
    let bad: usize = failed.values().sum();
    let reasons = failed.iter().map(|(w, n)| format!("{w} — {n}")).collect::<Vec<_>>().join("; ");
    if keys == 0 && bad == 0 {
        return (true, "В чатах не нашёл ключей.".into());
    }
    let mut m =
        format!("Закрыл {} в {}.", crate::diag::plural(keys, "ключ", "ключа", "ключей"), crate::diag::plural(changed, "чате", "чатах", "чатах"));
    if bad > 0 {
        m += &format!(" Не удалось изменить файлов: {bad} ({reasons}) — ключи там могли остаться.");
    }
    m += " Модель ключи уже видела: замени их у поставщика.";
    (bad == 0, m)
}

/// Какой файл править для пути истории: сам файл; ссылка (~/.bash_history → ~/dotfiles/…) — только если её
/// настоящая цель внутри домашней папки и принадлежит пользователю (запись идёт в цель, ссылка остаётся).
/// Ok(None) — файла нет; Err — почему нельзя.
fn history_target(link: &Path, home: &Path) -> Result<Option<PathBuf>, String> {
    let m = match std::fs::symlink_metadata(link) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if !m.file_type().is_symlink() {
        return Ok(Some(link.to_path_buf()));
    }
    let real = match crate::shellcfg::real_in_home(link, home) {
        Some(r) => r,
        None if !link.exists() => return Ok(None), // висячая ссылка: читать нечего
        None => return Err("ссылка ведёт за пределы домашней папки".into()),
    };
    // ссылка должна вести в файл истории (в имени «hist»), а не в произвольный файл дома и не в каталоги
    // с ключами и настройками: иначе подложенная ссылка заставила бы вырезать «ключи» из чужого файла
    let in_secret_dir = [".ssh", ".gnupg", ".config", ".aws", ".kube"].iter().any(|d| real.starts_with(home.join(d)));
    let name_ok = real.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().contains("hist"));
    if in_secret_dir || !name_ok {
        return Err("ссылка ведёт не в файл истории".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        extern "C" {
            fn getuid() -> u32;
        }
        let me = unsafe { getuid() };
        if std::fs::metadata(&real).map(|m| m.uid()).ok() != Some(me) {
            return Err("цель ссылки принадлежит не тебе".into());
        }
    }
    Ok(Some(real))
}

/// Убрать записи истории с ключами: у fish запись — блок «- cmd: …» с полями «when:», «paths:» (удаляется
/// целиком), у bash/zsh — строка (продолжение «\» на следующей строке — вместе с ней). (текст, сколько убрано)
fn drop_secret_records(t: &[u8], fish: bool) -> (Vec<u8>, usize) {
    let mut records: Vec<Vec<u8>> = Vec::new();
    for line in t.split_inclusive(|b| *b == b'\n') {
        let starts = if fish {
            line.starts_with(b"- cmd:")
        } else {
            records.last().is_none_or(|r| {
                let end = r.iter().rposition(|b| *b != b'\n' && *b != b'\r').map_or(0, |i| i + 1);
                !r[..end].ends_with(b"\\")
            })
        };
        if starts || records.is_empty() {
            records.push(line.to_vec());
        } else if let Some(last) = records.last_mut() {
            last.extend_from_slice(line);
        }
    }
    let before = records.len();
    // сравниваем по тексту с заменой неверных байтов, а сохраняем исходные байты уцелевших записей
    records.retain(|r| {
        let s = String::from_utf8_lossy(r);
        secret_kinds(&s).is_empty() && decode_escapes(&s).as_deref().is_none_or(|d| secret_kinds(d).is_empty())
    });
    let n = before - records.len();
    (records.concat(), n)
}

#[cfg(test)]
mod tests {
    /// Вывод dscl: служебные записи и хозяин не считаются, обычный второй человек — да.
    #[test]
    fn dscl_other_humans() {
        let out = "_amavisd                -6\nGuest                   201\ndaemon                  1\nnobody                  -2\nroot                    0\nulyanaperova            501\nanna                    502\n_spotlight              89\n";
        assert_eq!(count_dscl_humans(out, "ulyanaperova"), 1);
        assert_eq!(count_dscl_humans(out, "anna"), 1);
        assert_eq!(count_dscl_humans("ulyanaperova 501\n", "ulyanaperova"), 0);
        assert_eq!(count_dscl_humans("", "x"), 0);
    }

    /// Длинная цепочка «sk-sk-sk-…» не должна вешать поиск ключей (время росло как квадрат длины).
    #[test]
    fn long_prefix_run_is_fast() {
        let t = "sk-".repeat(70_000);
        let t0 = std::time::Instant::now();
        let _ = secret_kinds(&t);
        let _ = mask_secrets(&t);
        assert!(t0.elapsed() < std::time::Duration::from_secs(3), "{:?}", t0.elapsed());
    }

    use super::*;

    #[test]
    fn finds_kinds_without_false_alarms() {
        let fake = |p: &str| format!("{p}{}", "Ab3dEf9hIj2kLm5nOp7qRs1tUv4wXy8zAb3dEf9hIj2k");
        // правдоподобные случайные (собираются здесь, чтобы в тексте кода не было готовых «ключей»)
        let rnd = |n: usize| -> String {
            let mut x: u64 = 0x9e3779b97f4a7c15;
            (0..n)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789"[(x % 57) as usize] as char
                })
                .collect()
        };
        assert_eq!(secret_kinds(&format!("export ANTHROPIC_API_KEY=sk-ant-api03-{}", rnd(60))), vec!["Anthropic"]);
        assert_eq!(secret_kinds(&format!("key: \"sk-proj-{}\"", rnd(48))), vec!["OpenAI"]);
        assert_eq!(secret_kinds(&format!("token=ghp_{}", rnd(36))), vec!["GitHub"]);
        assert_eq!(secret_kinds(&format!("AKIA{}", rnd(16).to_uppercase())), vec!["AWS"]);
        assert_eq!(secret_kinds(&format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}", rnd(70))), vec!["закрытый ключ"]);
        // примеры и выдумки — не ключи
        assert!(secret_kinds(&fake("sk-proj-")).is_empty());
        assert!(secret_kinds("AKIAIOSFODNN7EXAMPLE").is_empty());
        assert!(secret_kinds("-----BEGIN OPENSSH PRIVATE KEY-----\nabc").is_empty());
        // кусок base64 картинки, где случайно встретилось «AIza»
        assert!(secret_kinds(&format!("iVBORw0KGgo/{}/AIza{}+Qw", rnd(20), rnd(50))).is_empty());
        assert_eq!(secret_kinds(&format!("key=AIza{}\"", rnd(35))), vec!["Google"]);
        // не ключи
        assert!(secret_kinds("pip install scikit-learn sk-learn-tutorial-for-beginners-and-everyone").is_empty());
        assert!(secret_kinds("task-ant-something sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_empty());
        assert!(secret_kinds("-----BEGIN ENCRYPTED PRIVATE KEY-----").is_empty());
        assert!(secret_kinds("https://example.com/sk-1/askhjhd").is_empty());
    }

    #[test]
    fn history_records() {
        let key = format!("sk-proj-{}", "Qw7Er9Ty2Ui4Op6As8Df1Gh3Jk5Lz0Xc9Vb7Nm5Qa3Ws1Ed");
        let fish = format!("- cmd: ls\n  when: 1\n- cmd: export OPENAI_API_KEY={key}\n  when: 2\n  paths:\n    - /tmp\n- cmd: pwd\n  when: 3\n");
        let (out, n) = drop_secret_records(fish.as_bytes(), true);
        assert_eq!(n, 1);
        assert_eq!(out, b"- cmd: ls\n  when: 1\n- cmd: pwd\n  when: 3\n");
        let bash = format!("ls\ncurl -H \\\n  'x: {key}'\npwd\n");
        let (out, n) = drop_secret_records(bash.as_bytes(), false);
        assert_eq!((out.as_slice(), n), (b"ls\npwd\n".as_slice(), 1));
    }

    fn fake_key() -> String {
        format!("sk-proj-{}", "Qw7Er9Ty2Ui4Op6As8Df1Gh3Jk5Lz0Xc9Vb7Nm5Qa3Ws1Ed")
    }

    /// Временный HOME и папка настроек; настоящие файлы не трогаются.
    #[cfg(unix)]
    fn with_home(name: &str, f: impl FnOnce(&Path)) {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("aptechka_fp_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("home")).unwrap();
        let (oh, ox) = (std::env::var_os("HOME"), std::env::var_os("XDG_CONFIG_HOME"));
        std::env::set_var("HOME", d.join("home"));
        std::env::set_var("XDG_CONFIG_HOME", d.join("cfg"));
        f(&d.join("home"));
        match oh {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match ox {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    fn tmp_file(name: &str, data: &[u8]) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aptechka_mask_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("chat.jsonl");
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn mask_keeps_size_and_other_bytes() {
        let (k1, k2) = (fake_key(), concat!("ghp_", "Ab3Cd5Ef7Gh9Ij1Kl3Mn5Op7Qr9St1Uv3Wx5"));
        // два ключа в одной строке, третий — на другой, битый UTF-8 и ключ в самом конце без перевода строки
        let mut data = format!("{{\"t\":\"a {k1} b {k2}\"}}\n{{\"t\":\"привет\"}}\n").into_bytes();
        data.extend_from_slice(b"\xff\xfe ok\n");
        data.extend_from_slice(format!("конец {k1}").as_bytes());
        let p = tmp_file("a", &data);
        let (n, left) = mask_file(&p, true).unwrap();
        assert_eq!((n, left), (3, false));
        assert_eq!(std::fs::read(&p).unwrap(), data, "пробный запуск ничего не меняет");
        let (n, left) = mask_file(&p, false).unwrap();
        assert_eq!((n, left), (3, false));
        let after = std::fs::read(&p).unwrap();
        assert_eq!(after.len(), data.len());
        let text = String::from_utf8_lossy(&after);
        assert!(text.contains("sk-proj-****") && text.contains("ghp_****"), "{text}");
        assert!(!text.contains("Qw7Er9") && !text.contains("Ab3Cd5"));
        assert!(secret_kinds(&text).is_empty());
        // все байты вне ключей — как были
        let diff: Vec<usize> = (0..data.len()).filter(|&i| data[i] != after[i]).collect();
        assert!(diff.iter().all(|&i| after[i] == b'*'));
        assert_eq!(diff.len(), (k1.len() - 8) * 2 + k2.len() - 4);
        assert_eq!(mask_file(&p, false).unwrap().0, 0);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn mask_escaped_and_pem_and_boundary() {
        // ключ с экранированием JSON (\u002d вместо «-»), закрытый ключ с «\n» между строками
        let esc = fake_key().replacen('-', "\\u002d", 1);
        let body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7VJTUt9Us8cKj";
        let pem = format!("-----BEGIN PRIVATE KEY-----\\n{body}\\n{body}\\n-----END PRIVATE KEY-----");
        let mut data = format!("{{\"a\":\"{esc}\",\"b\":\"{pem}\"}}\n").into_bytes();
        // ключ ровно на стыке проходов (1 МБ)
        let pad = (1 << 20) - data.len() - 20; // ключ начинается до границы, кончается после
        data.extend(std::iter::repeat_n(b' ', pad));
        data.extend_from_slice(format!(" {}", fake_key()).as_bytes());
        let p = tmp_file("b", &data);
        let (n, left) = mask_file(&p, false).unwrap();
        assert_eq!((n, left), (3, false));
        let after = String::from_utf8(std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(after.len(), data.len());
        assert!(secret_kinds(&after).is_empty(), "{}", &after[..300]);
        assert!(after.contains("-----BEGIN PRIVATE KEY-----\\n****") && after.contains("-----END PRIVATE KEY-----"));
        assert!(!after.contains("MIIEvQ") && !after.contains("Qw7Er9"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn mask_refuses_symlink() {
        let p = tmp_file("c", fake_key().as_bytes());
        let l = p.with_file_name("link.jsonl");
        std::os::unix::fs::symlink(&p, &l).unwrap();
        assert!(mask_file(&l, false).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), fake_key().as_bytes());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn fix_chats_on_fake_home() {
        with_home("chats", |h| {
            let d = h.join(".claude/projects/x");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("a.jsonl"), format!("{{\"m\":\"{}\"}}\n", fake_key())).unwrap();
            std::fs::write(d.join("b.jsonl"), "без ключей\n").unwrap();
            crate::fixes::set_dry_run(true);
            let (ok, msg) = fix_chats();
            crate::fixes::set_dry_run(false);
            assert!(ok && msg.contains("закрыл бы 1 ключ в 1 чате"), "{msg}");
            assert!(std::fs::read_to_string(d.join("a.jsonl")).unwrap().contains("Qw7Er9"));
            let (ok, msg) = fix_chats();
            assert!(ok && msg.starts_with("Закрыл 1 ключ в 1 чате"), "{msg}");
            assert!(!std::fs::read_to_string(d.join("a.jsonl")).unwrap().contains("Qw7Er9"));
            assert_eq!(std::fs::read_to_string(d.join("b.jsonl")).unwrap(), "без ключей\n");
        });
    }

    #[cfg(unix)]
    #[test]
    fn fix_history_symlink_and_non_utf8() {
        with_home("hist", |h| {
            std::fs::create_dir_all(h.join("dot")).unwrap();
            let mut data = b"ls \xef\xf0\xe8\xe2\xe5\xf2\n".to_vec(); // cp1251
            data.extend_from_slice(format!("export OPENAI_API_KEY={}\n", fake_key()).as_bytes());
            data.extend_from_slice(b"pwd\n");
            std::fs::write(h.join("dot/hist"), &data).unwrap();
            std::os::unix::fs::symlink(h.join("dot/hist"), h.join(".bash_history")).unwrap();
            let (ok, msg) = fix_history();
            assert!(ok, "{msg}");
            assert!(msg.starts_with("Убрал"), "{msg}");
            let after = std::fs::read(h.join("dot/hist")).unwrap();
            assert_eq!(after, b"ls \xef\xf0\xe8\xe2\xe5\xf2\npwd\n");
            assert!(std::fs::symlink_metadata(h.join(".bash_history")).unwrap().file_type().is_symlink());
        });
    }

    #[cfg(unix)]
    #[test]
    fn fix_history_refuses_symlink_to_non_history_file() {
        with_home("hist_key", |h| {
            std::fs::create_dir_all(h.join(".ssh")).unwrap();
            let key = format!("-----BEGIN OPENSSH PRIVATE KEY-----\nexport K={}\n", fake_key());
            std::fs::write(h.join(".ssh/id_ed25519"), &key).unwrap();
            std::os::unix::fs::symlink(h.join(".ssh/id_ed25519"), h.join(".bash_history")).unwrap();
            let (ok, _) = fix_history();
            assert!(!ok);
            assert_eq!(std::fs::read_to_string(h.join(".ssh/id_ed25519")).unwrap(), key);
            // файл в доме без «hist» в имени — тоже нет
            std::fs::write(h.join("notes.txt"), &key).unwrap();
            std::fs::remove_file(h.join(".bash_history")).unwrap();
            std::os::unix::fs::symlink(h.join("notes.txt"), h.join(".bash_history")).unwrap();
            let (ok, _) = fix_history();
            assert!(!ok);
            assert_eq!(std::fs::read_to_string(h.join("notes.txt")).unwrap(), key);
        });
    }

    #[cfg(unix)]
    #[test]
    fn fix_history_refuses_unreadable() {
        with_home("hist_bad", |h| {
            // ссылка за пределы домашней папки — не «ключей нет», а отказ
            let out = h.parent().unwrap().join("outside_hist");
            std::fs::write(&out, format!("export K={}\n", fake_key())).unwrap();
            std::os::unix::fs::symlink(&out, h.join(".bash_history")).unwrap();
            let (ok, msg) = fix_history();
            assert!(!ok, "{msg}");
            assert!(msg.contains("Не смог обработать"), "{msg}");
            // каталог вместо файла — тоже не «чисто»
            std::fs::remove_file(h.join(".bash_history")).unwrap();
            std::fs::create_dir(h.join(".bash_history")).unwrap();
            let (ok, _) = fix_history();
            assert!(!ok);
        });
    }

    #[cfg(unix)]
    #[test]
    fn scan_state_distinguishes_missing_and_unreadable() {
        let d = std::env::temp_dir().join(format!("aptechka_fp_scan_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        assert_eq!(scan_state(&d.join("нет"), until), Ok(None));
        assert_eq!(scan_state(&d, until), Err(())); // каталог
        std::fs::write(d.join("a"), "x").unwrap();
        assert_eq!(scan_state(&d.join("a"), until), Ok(Some(vec![])));
        assert_eq!(scan_state(&d.join("a"), Instant::now() - Duration::from_secs(1)), Err(()));
        // обрезка списка файлов сообщается
        std::fs::write(d.join("b"), "x").unwrap();
        assert!(files_in(&d, &[""], 1).1);
        assert!(!files_in(&d, &[""], 5).1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn backup_redaction() {
        let t = format!(
            "a=1\r\nexport OPENAI_API_KEY={}\r\nexport http_proxy=http://x:9\n-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nb=2\n",
            fake_key()
        );
        let r = redact_for_backup(&t);
        assert!(!r.contains("sk-proj") && !r.contains("AAAA"), "{r}");
        assert!(r.contains("a=1\r\n") && r.contains("export http_proxy=http://x:9\n") && r.ends_with("b=2\n"), "{r}");
    }

    #[test]
    fn ssh_cipher() {
        // заголовок openssh-key-v1 с шифром «none» (без пароля) и «aes256-ctr» (с паролем)
        let mk = |cipher: &[u8]| {
            let mut raw = b"openssh-key-v1\0".to_vec();
            raw.extend_from_slice(&(cipher.len() as u32).to_be_bytes());
            raw.extend_from_slice(cipher);
            raw.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            format!("-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----", crate::elevate::b64_for_test(&raw))
        };
        assert!(!ssh_key_encrypted(&mk(b"none")));
        assert!(ssh_key_encrypted(&mk(b"aes256-ctr")));
    }
}

/// Независимые регрессионные тесты (REGRESSION_TESTS.md).
#[cfg(test)]
#[path = "regress/footprint.rs"]
mod regress;
