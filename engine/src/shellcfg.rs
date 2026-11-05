//! Файлы настроек оболочки (~/.bashrc, ~/.profile, ~/.zshrc, /etc/environment, environment.d):
//! где в них заданы переменные прокси и можно ли безопасно закомментировать такую строку.
//!
//! Одни и те же правила для проверки (что показать) и для починки (что менять):
//! - считается только строка из одних присваиваний (`export http_proxy=…`, `http_proxy=… https_proxy=…`);
//!   `http_proxy=… curl …` задаёт прокси одной команде и в настройку не превращается;
//! - менять можно только строку верхнего уровня: внутри `if … fi`, функции или цикла закомментированная
//!   строка может оставить пустой блок, и оболочка перестанет читать файл (`if …; then fi` — ошибка);
//! - ссылку (~/.bashrc → ~/dotfiles/bashrc) не заменяем файлом, а правим настоящий файл;
//! - окончания строк (\r\n) и права файла сохраняются, запись атомарная, рядом — копия.

use std::path::{Path, PathBuf};

/// Переменные, которыми программы берут прокси.
pub const PROXY_VARS: [&str; 5] = ["http_proxy", "https_proxy", "all_proxy", "ftp_proxy", "socks_proxy"];

/// Найденная строка с прокси.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyLine {
    /// номер строки, с 1
    pub n: usize,
    /// (переменная, значение без кавычек); у `export http_proxy` без «=» значение пустое
    pub vars: Vec<(String, String)>,
    /// строка на верхнем уровне файла — её можно закомментировать
    pub top_level: bool,
    /// в строке нет ничего, кроме прокси (иначе закомментировать целиком нельзя)
    pub only_proxy: bool,
}

/// Разбить строку на слова с учётом кавычек и `\`. None — в строке есть то, что мы не разбираем
/// (подстановка `$(…)`, `;`, `|`, `&` вне кавычек): такую строку считаем «не настройкой».
fn words(line: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut q: Option<char> = None;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match (q, c) {
            (Some(qc), c) if c == qc => q = None,
            (Some('"'), '\\') => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                q = Some(c);
                has = true;
            }
            (None, '\\') => {
                cur.push(it.next()?);
                has = true;
            }
            (None, '#') if !has => break, // комментарий
            (None, c) if c.is_whitespace() => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, ';' | '|' | '&' | '`' | '(' | ')' | '<' | '>') => return None,
            (None, '$') if it.peek() == Some(&'(') => return None,
            (None, c) => {
                cur.push(c);
                has = true;
            }
        }
    }
    if q.is_some() {
        return None; // кавычка не закрыта — строка продолжается дальше
    }
    if has {
        out.push(cur);
    }
    Some(out)
}

fn is_name(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_alphabetic() || f == '_') && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// Прокси-переменные, которые строка задаёт насовсем. Пусто — строка не про прокси (или не настройка).
pub fn proxy_vars(line: &str) -> Vec<(String, String)> {
    assignments(line).into_iter().filter(|(k, _)| PROXY_VARS.contains(&k.to_ascii_lowercase().as_str())).collect()
}

/// В строке — только прокси-переменные (её можно закомментировать целиком, ничего чужого не погасив).
/// «export PATH=… http_proxy=…» — нет: вместе с прокси пропал бы PATH.
pub fn only_proxy(line: &str) -> bool {
    let all = assignments(line);
    !all.is_empty() && all.iter().all(|(k, _)| PROXY_VARS.contains(&k.to_ascii_lowercase().as_str()))
}

/// Все присваивания строки-настройки (export/declare -x или голые «X=…»); пусто — строка не настройка.
fn assignments(line: &str) -> Vec<(String, String)> {
    let Some(w) = words(line) else { return Vec::new() };
    let mut w = w.as_slice();
    // export / declare -x / typeset -x
    let exported = match w {
        [e, rest @ ..] if e == "export" => {
            w = rest;
            true
        }
        [d, x, rest @ ..] if (d == "declare" || d == "typeset") && x == "-x" => {
            w = rest;
            true
        }
        _ => false,
    };
    let mut out = Vec::new();
    for t in w {
        let (k, v) = match t.split_once('=') {
            Some((k, v)) if is_name(k) => (k, v),
            // «export http_proxy» — без значения; без export голое слово — уже команда, не настройка
            None if exported && is_name(t) => (t.as_str(), ""),
            _ => return Vec::new(),
        };
        out.push((k.to_string(), v.to_string()));
    }
    out
}

/// Для каждой строки файла: Some(true) — верхний уровень (не внутри if/case/цикла/функции/{…}, не продолжение
/// предыдущей строки); Some(false) — команда, но вложенная; None — не команда, а текст «here-document».
/// Разбор простой: при неясности — «вложенная», тогда строку не трогаем, а подсказываем поправить вручную.
pub fn top_level(text: &str) -> Vec<Option<bool>> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut cont = false; // предыдущая строка кончилась на «\»
    let mut heredoc: Option<String> = None;
    // открытая на прошлых строках кавычка: строки внутри неё — текст значения, а не команды
    // (message="первая\nexport https_proxy=…\nпоследняя")
    let mut quote: Option<char> = None;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(end) = &heredoc {
            out.push(None);
            if line.trim() == end {
                heredoc = None;
            }
            continue;
        }
        let started_in_quote = quote.is_some();
        quote = quote_state_after(line, quote);
        if started_in_quote {
            out.push(None);
            continue;
        }
        out.push(Some(depth == 0 && !cont));
        cont = line.ends_with('\\');
        if let Some(end) = heredoc_start(line) {
            heredoc = Some(end);
        }
        depth = (depth + block_delta(line)).max(0);
    }
    out
}

/// Какая кавычка открыта после строки (с учётом открытой до неё): одинарная до одинарной, двойная — до
/// двойной без «\» перед ней; «#» вне кавычек — комментарий до конца строки.
fn quote_state_after(line: &str, mut q: Option<char>) -> Option<char> {
    let mut prev = ' ';
    let mut esc = false;
    for c in line.chars() {
        match q {
            Some('\'') => {
                if c == '\'' {
                    q = None;
                }
            }
            Some(_) => {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    q = None;
                }
            }
            None => {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '#' && (prev.is_whitespace() || prev == ';') {
                    break;
                } else if c == '"' || c == '\'' {
                    q = Some(c);
                }
            }
        }
        prev = c;
    }
    q
}

/// Начало here-document («cat <<EOF», «<<-'END'»): маркер конца. Не here-document — арифметика «$((1<<3))»,
/// «<<<» (here-string), «<<» внутри кавычек, «<<» без слова после него.
fn heredoc_start(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let mut q: Option<u8> = None;
    let mut paren = 0i32; // глубина «$(( … ))» и «(( … ))»
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match q {
            Some(qc) => {
                if c == qc {
                    q = None;
                }
            }
            None => match c {
                b'\'' | b'"' => q = Some(c),
                b'#' if i == 0 || b[i - 1].is_ascii_whitespace() => return None,
                b'(' if b.get(i + 1) == Some(&b'(') => {
                    paren += 1;
                    i += 1;
                }
                b')' if b.get(i + 1) == Some(&b')') && paren > 0 => {
                    paren -= 1;
                    i += 1;
                }
                // «<<<» — here-string: пропускаем все три знака
                b'<' if b.get(i + 1) == Some(&b'<') && b.get(i + 2) == Some(&b'<') => i += 2,
                b'<' if paren == 0 && b.get(i + 1) == Some(&b'<') => {
                    let rest = line[i + 2..].trim_start_matches('-').trim_start();
                    let word = rest.trim_start_matches(['\'', '"', '\\']);
                    let end: String = word.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                    // маркер — слово, начинающееся с буквы или «_» (не «3))»)
                    return end.chars().next().filter(|c| c.is_ascii_alphabetic() || *c == '_').map(|_| end);
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Сколько блоков строка открывает минус сколько закрывает (по первым словам команд).
fn block_delta(line: &str) -> i32 {
    // без комментария (грубо: «#» после пробела или в начале) и содержимого кавычек
    let mut s = String::new();
    let mut q: Option<char> = None;
    let mut prev = ' ';
    for c in line.chars() {
        match q {
            Some(qc) if c == qc => q = None,
            Some(_) => {}
            None if c == '\'' || c == '"' => q = Some(c),
            None if c == '#' && prev.is_whitespace() => break,
            None => s.push(c),
        }
        prev = c;
    }
    let mut d = 0;
    for part in s.split([';', '&', '|']) {
        let mut ws = part.split_whitespace();
        let first = ws.next().unwrap_or("");
        d += match first {
            "if" | "case" | "for" | "while" | "until" | "select" | "{" => 1,
            "fi" | "esac" | "done" | "}" => -1,
            "function" => 1, // «function f {» — тело ниже; «{» на той же строке посчитается ниже
            _ => 0,
        };
        // «f() {», «function f {»: открывающая скобка в конце
        if first != "{" && part.trim_end().ends_with('{') {
            d += if first == "function" { 0 } else { 1 };
        }
        // «}» в конце однострочника: «f() { …; }» — уже учтено как первое слово части после «;»
    }
    d
}

/// Все строки с прокси в тексте файла.
pub fn find_proxy_lines(text: &str) -> Vec<ProxyLine> {
    let top = top_level(text);
    text.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let top_level = top.get(i).copied().flatten()?;
            let line = l.trim_end_matches('\r');
            let vars = proxy_vars(line);
            (!vars.is_empty()).then(|| ProxyLine { n: i + 1, vars, top_level, only_proxy: only_proxy(line) })
        })
        .collect()
}

/// Настоящий путь файла (по ссылкам), если он внутри домашней папки (тоже настоящей). Иначе None:
/// `~/../../etc/x` или ссылка на системный файл — не наше.
pub fn real_in_home(path: &Path, home: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    let home = std::fs::canonicalize(home).ok()?;
    (real.starts_with(&home) && real != home).then_some(real)
}

/// Короткий отпечаток текста строки: кнопка помнит не только номер, но и саму строку — если выше вставили
/// строку и номера сдвинулись, закомментирована не будет другая.
pub fn line_tag(line: &str) -> String {
    let mut h: u32 = 0x811c9dc5;
    for b in line.trim_end_matches('\r').bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    format!("{h:08x}")
}

/// Закомментировать строку `n` (с 1), если это по-прежнему строка с прокси верхнего уровня.
/// Пишет в настоящий файл (по ссылке), с копией `<файл>.aptechka.bak`. Ok — путь копии.
#[cfg(test)]
pub fn comment_out(path: &Path, n: usize, dry: bool) -> Result<PathBuf, String> {
    comment_out_tagged(path, n, None, dry)
}

/// То же, с отпечатком строки (`tag`): строка №n должна быть той самой.
pub fn comment_out_tagged(path: &Path, n: usize, tag: Option<&str>, dry: bool) -> Result<PathBuf, String> {
    // каким файл был при чтении: перед записью сверим — если за это время его поправили (редактор, другая
    // программа) или подменили, не затираем
    let before = identity(path);
    #[cfg(unix)]
    if std::fs::metadata(path).is_ok_and(|m| std::os::unix::fs::MetadataExt::nlink(&m) > 1) {
        return Err("У файла несколько имён (жёсткая ссылка) — правка разорвала бы связь, поправь его вручную.".into());
    }
    let text = crate::store::read_capped(path, 1 << 20).map_err(|e| format!("Не прочитать {}: {e}", path.display()))?;
    if let Some(t) = tag {
        if text.lines().nth(n.saturating_sub(1)).map(line_tag).as_deref() != Some(t) {
            return Err(format!(
                "Файл {} изменился с прошлой проверки (строка {n} уже другая) — проверь ещё раз, я ничего не трогала.",
                path.display()
            ));
        }
    }
    let line = find_proxy_lines(&text).into_iter().find(|l| l.n == n);
    match line {
        Some(l) if l.top_level && l.only_proxy => {}
        Some(l) if !l.only_proxy => return Err(format!("В строке {n} кроме прокси есть другие настройки (например, PATH): если закомментировать её, пропадут и они. Убери из неё только прокси вручную.")),
        Some(_) => return Err("Строка внутри условия или функции — закомментировать её одну небезопасно, поправь файл вручную.".into()),
        None => return Err(format!("Файл {} изменился с прошлой проверки — проверь ещё раз, я ничего не трогала.", path.display())),
    }
    // копия — в папке Аптечки, с датой и временем: вторая правка не затирает первую копию; только для владельца
    // (в строке прокси бывает пароль)
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let bak = crate::store::private_dir().join("backups").join(format!("{name}.{stamp}.bak"));
    if dry {
        return Ok(bak);
    }
    // строки вместе с их окончаниями: \r\n остаётся \r\n
    let new: String = text
        .split_inclusive('\n')
        .enumerate()
        .map(|(i, l)| {
            if i + 1 != n {
                return l.to_string();
            }
            let body = l.trim_end_matches(['\n', '\r']);
            format!("# {body}   # выключено Аптечкой{}", &l[body.len()..])
        })
        .collect();
    let _ = std::fs::create_dir_all(bak.parent().unwrap_or(Path::new(".")));
    if bak.exists() {
        return Err("Копия с таким временем уже есть — попробуй через секунду.".into());
    }
    crate::store::write_private(&bak, crate::footprint::redact_for_backup(&text).as_bytes())
        .map_err(|_| "Не удалось сделать копию файла — ничего не меняю.".to_string())?;
    if identity(path) != before {
        return Err("Файл изменился, пока я его правила — проверь ещё раз, я ничего не трогала.".into());
    }
    write_atomic(path, &new)?;
    Ok(bak)
}

/// Записать через временный файл рядом и подмену (сбой посреди записи не оставит пустой файл); права — как были.
pub fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    // права — как у прежнего файла; временный файл — с непредсказуемым именем, без перехода по ссылке
    #[cfg(unix)]
    let mode = std::fs::metadata(path).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o777).unwrap_or(0o644);
    #[cfg(not(unix))]
    let mode = 0o644;
    crate::store::write_mode(path, content.as_bytes(), mode).map_err(|e| e.to_string())
}

/// (устройство, номер файла, размер, время изменения) — «тот же ли это файл и не менялся ли он».
fn identity(p: &Path) -> Option<(u64, u64, u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((m.dev(), m.ino(), m.len(), m.mtime_nsec() + m.mtime() * 1_000_000_000))
    }
    #[cfg(not(unix))]
    Some((0, 0, m.len(), m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vars() {
        assert_eq!(proxy_vars("export https_proxy=http://127.0.0.1:10809"), vec![("https_proxy".into(), "http://127.0.0.1:10809".into())]);
        assert_eq!(proxy_vars("  HTTP_PROXY='socks5://h:1' ALL_PROXY=\"x\"  # комм").len(), 2);
        assert_eq!(proxy_vars("declare -x http_proxy=a").len(), 1);
        assert_eq!(proxy_vars("export http_proxy").len(), 1);
        // не настройка: прокси одной команде, команды, подстановки, комментарий
        assert!(proxy_vars("http_proxy=x curl ya.ru").is_empty());
        assert!(proxy_vars("export http_proxy=$(cat ~/.p)").is_empty());
        assert!(proxy_vars("[ -n x ] && export http_proxy=a").is_empty());
        assert!(proxy_vars("# export http_proxy=a").is_empty());
        assert!(proxy_vars("export no_proxy=localhost").is_empty());
        // строка с PATH и прокси — прокси найден, но строка не «только прокси»
        assert_eq!(proxy_vars("export PATH=\"$HOME/bin:$PATH\" http_proxy=http://h:1").len(), 1);
        assert!(!only_proxy("export PATH=\"$HOME/bin:$PATH\" http_proxy=http://h:1"));
        assert!(only_proxy("export http_proxy=a https_proxy=b"));
        assert!(proxy_vars("echo http_proxy=a").is_empty());
    }

    #[test]
    fn levels() {
        let t = "export http_proxy=a\nif true; then\n  export https_proxy=b\nfi\nf() {\n  export all_proxy=c\n}\nexport x=1 \\\n  http_proxy=d\ncat <<EOF\nexport http_proxy=e\nEOF\nexport ftp_proxy=f\n";
        let found: Vec<(usize, bool)> = find_proxy_lines(t).iter().map(|l| (l.n, l.top_level)).collect();
        assert_eq!(found, vec![(1, true), (3, false), (6, false), (9, false), (13, true)]);
        // «<<» в арифметике и here-string — не начало here-document: строки ниже видны
        let t2 = "N=$((1<<3))\nexport https_proxy=http://h:1\ncat <<< \"x\"\nexport http_proxy=a\n";
        assert_eq!(find_proxy_lines(t2).iter().map(|l| l.n).collect::<Vec<_>>(), vec![2, 4]);
        assert_eq!(heredoc_start("cat <<-'END'"), Some("END".into()));
        assert_eq!(heredoc_start("echo 'a << b'"), None);
        // строка внутри многострочного значения в кавычках — не настройка
        let t3 = "message=\"first\nexport https_proxy=http://127.0.0.1:9\nlast\"\nexport http_proxy=a\n";
        assert_eq!(find_proxy_lines(t3).iter().map(|l| l.n).collect::<Vec<_>>(), vec![4]);
        // однострочник с if — не настройка вовсе
        assert!(find_proxy_lines("if x; then export http_proxy=a; fi\n").is_empty());
        assert!(find_proxy_lines("case $x in a) ;; esac\nexport http_proxy=a\n")[0].top_level);
    }

    // про ссылки на файлы оболочки — только там, где они есть (на Windows ссылку пришлось бы заменить копией)
    #[cfg(unix)]
    #[test]
    fn comment_keeps_crlf_and_symlink() {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("aptechka_shellcfg_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("dot")).unwrap();
        // копии правок — во временную папку настроек, не в настоящую
        let old_cfg = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", d.join("cfg"));
        let real = d.join("dot/bashrc");
        std::fs::write(&real, "a=1\r\nexport http_proxy=x\r\nb=2\r\n").unwrap();
        let link = d.join(".bashrc");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(not(unix))]
        std::fs::copy(&real, &link).unwrap();
        let target = real_in_home(&link, &d).unwrap();
        assert_eq!(target, std::fs::canonicalize(&real).unwrap());
        // номер не той строки — отказ
        assert!(comment_out(&target, 1, false).is_err());
        comment_out(&target, 2, false).unwrap();
        assert_eq!(crate::store::read_text(&real).unwrap(), "a=1\r\n# export http_proxy=x   # выключено Аптечкой\r\nb=2\r\n");
        #[cfg(unix)]
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        // вне домашней папки — нет
        assert!(real_in_home(Path::new("/etc/hostname"), &d).is_none());
        // копия — в папке настроек Аптечки, только для владельца
        let baks: Vec<_> = std::fs::read_dir(d.join("cfg/aptechka/backups")).unwrap().flatten().collect();
        assert_eq!(baks.len(), 1);
        match old_cfg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
