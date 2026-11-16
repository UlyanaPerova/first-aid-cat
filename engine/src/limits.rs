//! Лимиты подписки и расход токенов ИИ-агентов: Claude Code и Codex.
//!
//! Откуда что берётся (только с этого компьютера, ничего не запрашиваем у серверов):
//! - Codex сам пишет в журнал сессии (`~/.codex/sessions/…jsonl`, событие `token_count`) проценты
//!   5-часового и недельного лимита с временем сброса, и токены каждого ответа;
//! - Claude Code отдаёт проценты лимитов только своей строке состояния (документированное поле
//!   `rate_limits`): `aptechka --statusline` сохраняет их в папку сеанса (см. `statusline_main`);
//!   токены — из журналов сессий `~/.claude/projects/…jsonl` (поле `usage` у ответов).
//!
//! Журналы Claude большие (сотни МБ за месяц), поэтому они читаются по кусочку: запоминается, докуда
//! прочитан каждый файл, и в следующий раз разбираются только новые строки.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::diag::{Check, BAD, INFO, OK, WARN};

/// За сколько дней собирается расход (для аналитики).
pub const DAYS: i64 = 30;

/// Одно окно лимита: занято, %; когда сбросится (Unix-время); длина окна, минут (0 — неизвестна).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub pct: f64,
    pub resets_at: i64,
    pub minutes: u32,
}

/// Лимиты агента: 5-часовое окно, недельное и когда данные получены.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
    pub five: Option<Window>,
    pub week: Option<Window>,
    pub at: i64,
}

impl Window {
    /// Окно уже сбросилось — занятое в нём не считается.
    pub fn now_pct(&self, now: i64) -> f64 {
        if self.resets_at > 0 && self.resets_at <= now {
            0.0
        } else {
            self.pct
        }
    }
}

/// Правдоподобно ли окно, полученное в миг `at`: сброс не может быть дальше длины самого окна (плюс 10 минут на
/// расхождение часов; длина неизвестна — не дальше 8 суток) и не может быть отрицательным. Иначе это мусор в файле:
/// так в плашку попал «сброс через 26709 дн» из пробных данных. Время сброса 0 — «неизвестно», это допустимо.
fn plausible(w: &Window, at: i64) -> bool {
    let len = if w.minutes > 0 { i64::from(w.minutes) * 60 } else { 8 * 86_400 };
    w.resets_at >= 0 && w.resets_at <= at.saturating_add(len).saturating_add(600)
}

impl Limits {
    /// Убрать неправдоподобные окна (см. `plausible`); None — не осталось ни одного. `at` 0 — миг неизвестен, меряем от «сейчас».
    fn checked(self, now: i64) -> Option<Limits> {
        let at = if self.at > 0 { self.at } else { now };
        let l = Limits { five: self.five.filter(|w| plausible(w, at)), week: self.week.filter(|w| plausible(w, at)), at: self.at };
        (l.five.is_some() || l.week.is_some()).then_some(l)
    }

    /// Самый заполненный лимит сейчас (для значка в панели; на macOS значка пока нет).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub fn top(&self, now: i64) -> Option<f64> {
        [self.five, self.week].iter().flatten().map(|w| w.now_pct(now)).reduce(f64::max)
    }
}

/// Токены: новые на входе, ответ, чтение из кэша, запись в кэш.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tok {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tok {
    pub fn total(&self) -> u64 {
        // с насыщением: испорченная строка журнала не должна «обернуть» сумму в маленькое число
        self.input.saturating_add(self.output).saturating_add(self.cache_read).saturating_add(self.cache_write)
    }
    pub fn add(&mut self, o: &Tok) {
        self.input = self.input.saturating_add(o.input);
        self.output = self.output.saturating_add(o.output);
        self.cache_read = self.cache_read.saturating_add(o.cache_read);
        self.cache_write = self.cache_write.saturating_add(o.cache_write);
    }
}

/// Расход по (день ГГГГММДД по местному времени, модель).
pub type Usage = BTreeMap<(i32, String), Tok>;

/// Токены по (день, проект). Проект — имя папки, в которой работал агент (сам путь не храним).
pub type Projects = BTreeMap<(i32, String), u64>;

/// Имя проекта по папке: последняя часть пути («/home/u/first_aid_cat» → «first_aid_cat»). Рабочая копия
/// агента внутри проекта («…/proj/.claude/worktrees/agent-…») — это тот же проект.
fn project_name(cwd: &str) -> String {
    let unix = cwd.replace('\\', "/");
    let cwd = unix.split("/.claude/worktrees/").next().unwrap_or(&unix);
    let t = cwd.trim_end_matches(['/', '\\']);
    let n = t.rsplit(['/', '\\']).next().unwrap_or("");
    if n.is_empty() {
        "—".into()
    } else {
        n.to_string()
    }
}

fn sum_projects(state: &Mutex<Option<Files>>) -> Projects {
    let g = state.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Projects::new();
    for st in g.iter().flat_map(|m| m.values()) {
        for (k, n) in &st.projects {
            *out.entry(k.clone()).or_default() += n;
        }
    }
    out
}

/// Расход Claude Code по проектам (журналы читаются тем же проходом, что и `claude_usage`).
pub fn claude_projects() -> Projects {
    let _ = claude_usage();
    sum_projects(&CLAUDE)
}

/// Расход Codex по проектам.
pub fn codex_projects() -> Projects {
    codex_scan();
    sum_projects(&CODEX)
}

// ───────────────────────────── время ─────────────────────────────

/// «2026-11-04T23:03:29.405Z» → Unix-время (только UTC с «Z», как пишут оба агента).
fn iso_secs(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |a: usize, l: usize| s.get(a..a + l)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0, 4)?, n(5, 2)?, n(8, 2)?, n(11, 2)?, n(14, 2)?, n(17, 2)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    // невозможные поля (час 25, 31 февраля) — не время: иначе строка журнала попала бы не в тот день
    if !(1..=12).contains(&m) || !(1..=days).contains(&d) || !(0..=23).contains(&hh) || !(0..=59).contains(&mm) || !(0..=60).contains(&ss) {
        return None;
    }
    // дни от 1970-01-01 (алгоритм Хиннанта «days_from_civil»)
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// День по местному времени: 20261104.
pub fn local_day(secs: i64) -> i32 {
    let t = crate::ui::localtime(secs);
    (t.year as i32) * 10_000 + t.mon * 100 + t.mday
}

// ───────────────────────────── чтение журналов по кусочку ─────────────────────────────

/// Что уже прочитано из одного журнала.
#[derive(Default)]
struct FileState {
    pos: u64,
    /// отпечатки уже учтённых ответов (Claude пишет один ответ несколькими строками с одинаковым usage)
    ids: HashSet<u64>,
    /// модель и папка последнего хода (Codex пишет их отдельной строкой turn_context)
    model: String,
    cwd: String,
    usage: Usage,
    /// токены по (день, проект — имя папки, где работал агент)
    projects: Projects,
    /// последние лимиты из этого файла (Codex) и их время
    limits: Option<Limits>,
    /// отпечаток начала файла: сколько первых байт взято и их хеш. По нему видно, что файл заменили другим,
    /// даже если новый длиннее прежнего (по одной длине такую замену не заметить)
    head: (u64, u64),
    /// итог сессии Codex (total_token_usage) из последнего учтённого события: повтор события с тем же итогом
    /// (обновились только лимиты) расход не прибавляет
    codex_total: Option<u64>,
}

/// Сколько первых байт файла идёт в отпечаток.
const HEAD: u64 = 512;

/// Хеш первых `n` байт файла (None — не прочитать).
fn head_print(f: &mut std::fs::File, n: u64) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut buf = vec![0u8; n as usize];
    f.seek(SeekFrom::Start(0)).ok()?;
    f.read_exact(&mut buf).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    buf.hash(&mut h);
    Some(h.finish())
}

/// Файл уже не тот, что читали: стал короче или вырос, но начало у него другое (переписан целиком).
/// Начало сверяется только у выросшего файла — у неизменившихся лишнего чтения нет.
fn replaced(f: &mut std::fs::File, len: u64, st: &FileState) -> bool {
    if len < st.pos {
        return true;
    }
    len > st.pos && st.head.0 > 0 && head_print(f, st.head.0) != Some(st.head.1)
}

/// Состояние чтения одного агента: файл → прочитанное.
type Files = HashMap<PathBuf, FileState>;

static CLAUDE: Mutex<Option<Files>> = Mutex::new(None);
static CODEX: Mutex<Option<Files>> = Mutex::new(None);

/// Журналы (*.jsonl), изменённые за последние DAYS дней: обход без перехода по ссылкам, не глубже depth.
fn recent_logs(dir: &Path, depth: usize, since: std::time::SystemTime, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if out.len() >= 20_000 {
            return;
        }
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            if depth > 0 {
                recent_logs(&p, depth - 1, since, out);
            }
        } else if ft.is_file() && p.extension().is_some_and(|x| x == "jsonl") && e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since) {
            out.push(p);
        }
    }
}

/// Строка длиннее этого пропускается (ответы с большими вставками кода бывают в несколько МБ).
const LINE_CAP: usize = 8 << 20;

/// Дочитать новые строки файла: каждая строка с `needle` разбирается как JSON и отдаётся в `eat`.
/// Файл стал короче (его переписали) — читаем заново.
fn read_new(p: &Path, st: &mut FileState, needles: &[&[u8]], seen: &mut HashSet<u64>, eat: &mut Eat) {
    let Ok(mut f) = crate::store::open_regular(p) else { return };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if replaced(&mut f, len, st) {
        *st = FileState::default();
    }
    if len == st.pos {
        return;
    }
    // отпечаток начала — пока он короче HEAD, берём столько, сколько в файле уже есть
    if st.head.0 < HEAD {
        let n = len.min(HEAD);
        if let Some(h) = head_print(&mut f, n) {
            st.head = (n, h);
        }
    }
    if f.seek(SeekFrom::Start(st.pos)).is_err() {
        return;
    }
    let _drop = crate::footprint::CacheDrop::new(&f);
    let mut rd = std::io::BufReader::with_capacity(1 << 16, f);
    let mut buf = Vec::new();
    loop {
        let got = crate::agents::read_line_capped(&mut rd, &mut buf, LINE_CAP);
        if got == 0 {
            break;
        }
        // последняя строка без «\n» ещё дописывается — дочитаем её в следующий раз
        if !buf.ends_with(b"\n") && got <= LINE_CAP {
            break;
        }
        st.pos += got as u64;
        if got > LINE_CAP || !needles.iter().any(|n| buf.windows(n.len()).any(|w| w == *n)) {
            continue;
        }
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf) {
            eat(&v, st, seen);
        }
    }
}

/// Разбор одной строки журнала: строка, состояние её файла и отпечатки ответов, уже учтённых во ВСЕХ файлах агента.
type Eat<'a> = dyn FnMut(&serde_json::Value, &mut FileState, &mut HashSet<u64>) + 'a;

/// Прочитать все свежие журналы агента; файлы старше DAYS дней забываются.
fn scan(state: &Mutex<Option<Files>>, roots: &[PathBuf], depth: usize, needles: &[&[u8]], eat: &mut Eat) {
    let since = std::time::SystemTime::now() - std::time::Duration::from_secs((DAYS as u64 + 1) * 86_400);
    let mut logs = Vec::new();
    for root in roots {
        recent_logs(root, depth, since, &mut logs);
    }
    let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
    let files = g.get_or_insert_with(HashMap::new);
    let keep: HashSet<&PathBuf> = logs.iter().collect();
    files.retain(|p, _| keep.contains(p));
    // файл стал короче (его переписали) — он будет прочитан заново; забыть его ответы ДО сбора общего списка,
    // иначе его же строки сочлись бы повторами
    for (p, st) in files.iter_mut() {
        let Ok(mut f) = crate::store::open_regular(p) else { continue };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if replaced(&mut f, len, st) {
            *st = FileState::default();
        }
    }
    // ответы, уже учтённые в любом файле: один ответ считается один раз, даже если историю сессии
    // скопировали в новый файл (раньше повторы отсеивались только внутри одного файла)
    let mut seen: HashSet<u64> = files.values().flat_map(|st| st.ids.iter().copied()).collect();
    for p in &logs {
        let st = files.entry(p.clone()).or_default();
        read_new(p, st, needles, &mut seen, eat);
    }
}

fn sum_usage(state: &Mutex<Option<Files>>) -> Usage {
    let g = state.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = Usage::new();
    for st in g.iter().flat_map(|m| m.values()) {
        for (k, t) in &st.usage {
            out.entry(k.clone()).or_default().add(t);
        }
    }
    out
}

fn u(v: &serde_json::Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

fn hash(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

// ───────────────────────────── Claude Code ─────────────────────────────

/// Папка Claude Code (CLAUDE_CONFIG_DIR или ~/.claude).
pub fn claude_dir() -> Option<PathBuf> {
    claude_dirs().into_iter().next()
}

/// Все папки, где Claude Code держит данные на этой машине. Задана CLAUDE_CONFIG_DIR — только она;
/// иначе обычная ~/.claude и папка по правилам XDG (~/.config/claude или $XDG_CONFIG_HOME/claude):
/// на части машин журналы лежат там. Журналы читаются из всех, что есть.
fn claude_dirs() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
    claude_dirs_from(env("CLAUDE_CONFIG_DIR"), crate::store::home_dir(), env("XDG_CONFIG_HOME"))
}

fn claude_dirs_from(config_dir: Option<PathBuf>, home: Option<PathBuf>, xdg: Option<PathBuf>) -> Vec<PathBuf> {
    let mut v = match (config_dir, home) {
        (Some(p), _) => vec![p],
        (None, Some(h)) => vec![h.join(".claude"), xdg.unwrap_or_else(|| h.join(".config")).join("claude")],
        (None, None) => Vec::new(),
    };
    v.retain(|p| p.is_dir());
    v
}

/// Строка журнала Claude: ответ модели с usage.
fn claude_line(v: &serde_json::Value, st: &mut FileState, seen: &mut HashSet<u64>) {
    if v.get("type").and_then(|x| x.as_str()) != Some("assistant") {
        return;
    }
    let Some(m) = v.get("message") else { return };
    let Some(us) = m.get("usage") else { return };
    let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("");
    if !id.is_empty() {
        // уже учтён — в этом файле или в другом (в `seen` — ответы всех файлов)
        let h = hash(id);
        if !seen.insert(h) {
            return;
        }
        st.ids.insert(h);
    }
    let Some(t) = v.get("timestamp").and_then(|x| x.as_str()).and_then(iso_secs) else { return };
    let model = m.get("model").and_then(|x| x.as_str()).unwrap_or("?");
    if model == "<synthetic>" {
        return;
    }
    let tok = Tok {
        input: u(us, "input_tokens"),
        output: u(us, "output_tokens"),
        cache_read: u(us, "cache_read_input_tokens"),
        cache_write: u(us, "cache_creation_input_tokens"),
    };
    st.usage.entry((local_day(t), model.to_string())).or_default().add(&tok);
    // проект — папка, где сессия началась (агент потом ходит по подпапкам: engine, src… — это тот же проект)
    if st.cwd.is_empty() {
        st.cwd = v.get("cwd").and_then(|x| x.as_str()).unwrap_or("").to_string();
    }
    let e = st.projects.entry((local_day(t), project_name(&st.cwd))).or_default();
    *e = e.saturating_add(tok.total());
}

/// Расход Claude Code за DAYS дней.
pub fn claude_usage() -> Usage {
    let roots: Vec<PathBuf> = claude_dirs().iter().map(|d| d.join("projects")).collect();
    if !roots.is_empty() {
        scan(&CLAUDE, &roots, 4, &[b"\"usage\""], &mut claude_line);
    }
    sum_usage(&CLAUDE)
}

/// Файл, куда строка состояния кладёт лимиты Claude (папка сеанса).
fn claude_limits_path() -> PathBuf {
    crate::store::runtime_dir().join("claude_limits.json")
}

fn window(v: Option<&serde_json::Value>, pct_key: &str, minutes: u32) -> Option<Window> {
    let v = v?;
    let pct = v.get(pct_key)?.as_f64()?;
    if !pct.is_finite() {
        return None;
    }
    Some(Window { pct: pct.clamp(0.0, 100.0), resets_at: v.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0), minutes })
}

/// Копия тех же лимитов в папке настроек: папка сеанса стирается при перезагрузке, и до первого ответа
/// Claude Code плашка оставалась без колец. По копии показываем последние известные цифры
/// (окно, которое с тех пор сбросилось, само станет «0 %, уже сброшен» — см. `Window::now_pct`).
fn claude_limits_saved_path() -> PathBuf {
    crate::store::private_dir().join("claude_limits.json")
}

/// Лимиты Claude, которые записала строка состояния: свежие из папки сеанса, а после перезагрузки —
/// последние сохранённые (None — Claude Code с нашей строкой состояния ещё ни разу не отвечал).
pub fn claude_limits() -> Option<Limits> {
    let read = |p: PathBuf| crate::store::read_capped(&p, 4096).ok().and_then(|t| parse_claude_limits(&t));
    let now = crate::ui::now_secs();
    let read = |p| read(p).and_then(|l: Limits| l.checked(now));
    read(claude_limits_path()).or_else(|| read(claude_limits_saved_path()))
}

fn parse_claude_limits(t: &str) -> Option<Limits> {
    let v: serde_json::Value = serde_json::from_str(t).ok()?;
    let rl = v.get("rate_limits")?;
    let l = Limits {
        five: window(rl.get("five_hour"), "used_percentage", 300),
        week: window(rl.get("seven_day"), "used_percentage", 7 * 24 * 60),
        at: v.get("at").and_then(|x| x.as_i64()).unwrap_or(0),
    };
    (l.five.is_some() || l.week.is_some()).then_some(l)
}

// ───────────────────────────── Codex ─────────────────────────────

fn codex_dir() -> Option<PathBuf> {
    crate::agents::find("codex").and_then(|a| a.data_dir())
}

/// Строка журнала Codex: модель хода, токены ответа и лимиты.
fn codex_line(v: &serde_json::Value, st: &mut FileState, _seen: &mut HashSet<u64>) {
    let Some(p) = v.get("payload") else { return };
    let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if kind == "turn_context" {
        if let Some(m) = p.get("model").and_then(|x| x.as_str()) {
            st.model = m.to_string();
        }
        // проект — папка первого хода сессии
        if let Some(c) = p.get("cwd").and_then(|x| x.as_str()).filter(|_| st.cwd.is_empty()) {
            st.cwd = c.to_string();
        }
        return;
    }
    if p.get("type").and_then(|x| x.as_str()) != Some("token_count") {
        return;
    }
    let Some(t) = v.get("timestamp").and_then(|x| x.as_str()).and_then(iso_secs) else { return };
    // итог сессии не изменился — это повтор события (пришли только новые лимиты): расход уже учтён
    let total = p
        .get("info")
        .and_then(|i| i.get("total_token_usage"))
        .map(|t| hash(&format!("{} {} {} {}", u(t, "input_tokens"), u(t, "cached_input_tokens"), u(t, "output_tokens"), u(t, "total_tokens"))));
    let repeat = total.is_some() && total == st.codex_total;
    if total.is_some() {
        st.codex_total = total;
    }
    if let Some(last) = p.get("info").and_then(|i| i.get("last_token_usage")).filter(|_| !repeat) {
        let (inp, cached) = (u(last, "input_tokens"), u(last, "cached_input_tokens"));
        let tok = Tok {
            input: inp.saturating_sub(cached),
            output: u(last, "output_tokens"),
            cache_read: cached,
            cache_write: u(last, "cache_write_input_tokens"),
        };
        let model = if st.model.is_empty() { "codex".to_string() } else { st.model.clone() };
        st.usage.entry((local_day(t), model)).or_default().add(&tok);
        let e = st.projects.entry((local_day(t), project_name(&st.cwd))).or_default();
        *e = e.saturating_add(tok.total());
    }
    if let Some(rl) = p.get("rate_limits") {
        let mins = |w: Option<&serde_json::Value>| w.and_then(|w| w.get("window_minutes")).and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        let l = Limits {
            five: window(rl.get("primary"), "used_percent", mins(rl.get("primary"))),
            week: window(rl.get("secondary"), "used_percent", mins(rl.get("secondary"))),
            at: t,
        };
        if let Some(l) = l.checked(t).filter(|_| st.limits.is_none_or(|old| old.at <= t)) {
            st.limits = Some(l);
        }
    }
}

fn codex_scan() {
    if let Some(d) = codex_dir() {
        scan(&CODEX, &[d.join("sessions")], 4, &[b"token_count", b"turn_context"], &mut codex_line);
    }
}

/// Расход Codex за DAYS дней.
pub fn codex_usage() -> Usage {
    codex_scan();
    sum_usage(&CODEX)
}

/// Самые свежие лимиты Codex из всех журналов.
pub fn codex_limits() -> Option<Limits> {
    codex_scan();
    let g = CODEX.lock().unwrap_or_else(|e| e.into_inner());
    g.iter().flat_map(|m| m.values()).filter_map(|s| s.limits).max_by_key(|l| l.at)
}

// ───────────────────────────── сводка для значка в панели ─────────────────────────────

/// Один агент в сводке: лимиты (если известны) и токены за сегодня и за 7 дней.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Agent {
    pub name: &'static str,
    pub limits: Option<Limits>,
    pub today: u64,
    pub week: u64,
    /// токены по дням: последние BAR_DAYS дней, от старого к сегодняшнему
    pub days: Vec<u64>,
}

/// Сколько дней в столбиках на плашке значка.
pub const BAR_DAYS: i64 = 14;

fn sums(us: &Usage, now: i64) -> (u64, u64) {
    let (today, week) = (local_day(now), local_day(now - 6 * 86_400));
    us.iter().fold((0, 0), |(t, w), ((d, _), x)| (t + if *d == today { x.total() } else { 0 }, w + if *d >= week { x.total() } else { 0 }))
}

/// Сводка по агентам, о которых что-то известно (лимиты или расход).
pub fn snapshot() -> Vec<Agent> {
    let now = crate::ui::now_secs();
    [("Claude Code", claude_limits(), claude_usage()), ("Codex", codex_limits(), codex_usage())]
        .into_iter()
        .filter_map(|(name, limits, us)| {
            let (today, week) = sums(&us, now);
            let mut per: HashMap<i32, u64> = HashMap::new();
            for ((d, _), t) in &us {
                *per.entry(*d).or_default() += t.total();
            }
            let days = (0..BAR_DAYS).rev().map(|i| per.get(&local_day(now - i * 86_400)).copied().unwrap_or(0)).collect();
            (limits.is_some() || week > 0).then_some(Agent { name, limits, today, week, days })
        })
        .collect()
}

// ───────────────────────────── строки на вкладке «ИИ» ─────────────────────────────

/// «3,4 млн», «52 тыс.», «812».
pub fn fmt_tokens(n: u64) -> String {
    // десятичная запятая — только по-русски (подпись «млн» переводится при показе: «3.4M»)
    let dec = |x: f64| {
        let s = format!("{x:.1}");
        if crate::i18n::lang() == crate::i18n::Lang::Ru {
            s.replace('.', ",")
        } else {
            s
        }
    };
    // граница разряда — по округлённому до 0,1 значению: 999 950 000 — это уже «1,0 млрд», а не «1000,0 млн»
    if n >= 999_950_000 {
        format!("{} млрд", dec(n as f64 / 1e9))
    } else if n >= 1_000_000 {
        format!("{} млн", dec(n as f64 / 1e6))
    } else if n >= 1_000 {
        format!("{} тыс.", n / 1000)
    } else {
        n.to_string()
    }
}

/// «в 21:40», «завтра в 09:15», «пн в 14:00» — когда сбросится.
pub(crate) fn when(at: i64, now: i64) -> String {
    let (t, today) = (crate::ui::localtime(at), local_day(now));
    let day = local_day(at);
    let hm = format!("{:02}:{:02}", t.hour, t.min);
    if day == today {
        format!("в {hm}")
    } else if day == local_day(now + 86_400) {
        format!("завтра в {hm}")
    } else {
        format!("{:02}.{:02} в {hm}", t.mday, t.mon)
    }
}

/// То же для длинного окна (неделя): день назван всегда — «сегодня в 14:00», «завтра в 14:00», «вт 17.11 в 14:00».
/// Просто «в 14:00» у недельного лимита непонятно: сегодня, завтра или в понедельник.
pub(crate) fn when_day(at: i64, now: i64) -> String {
    let (t, day) = (crate::ui::localtime(at), local_day(at));
    let hm = format!("{:02}:{:02}", t.hour, t.min);
    if day == local_day(now) {
        format!("сегодня в {hm}")
    } else if day == local_day(now + 86_400) {
        format!("завтра в {hm}")
    } else {
        format!("{} {:02}.{:02} в {hm}", weekday(t.year, t.mon, t.mday), t.mday, t.mon)
    }
}

/// День недели по дате (способ Сакамото): «пн» … «вс».
fn weekday(year: i64, mon: i32, mday: i32) -> &'static str {
    const SHIFT: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if mon < 3 { year - 1 } else { year };
    let d = (y + y / 4 - y / 100 + y / 400 + SHIFT[(mon.clamp(1, 12) - 1) as usize] + i64::from(mday)).rem_euclid(7);
    ["вс", "пн", "вт", "ср", "чт", "пт", "сб"][d as usize]
}

/// Строка одного лимита: «42% · сброс в 21:40».
fn limit_row(agent: &str, what: &str, w: &Window, at: i64, now: i64) -> Check {
    let pct = w.now_pct(now);
    let status = if pct >= 100.0 {
        BAD
    } else if pct >= 90.0 {
        WARN
    } else {
        OK
    };
    let mut value = if w.resets_at > 0 && w.resets_at <= now {
        "0% · уже сброшен".to_string()
    } else if w.resets_at > 0 {
        // недельный лимит — всегда с днём («сегодня в 14:00»), как на плашке
        format!("{pct:.0}% · сброс {}", if w.minutes >= 24 * 60 { when_day(w.resets_at, now) } else { when(w.resets_at, now) })
    } else {
        format!("{pct:.0}%")
    };
    // данные старые (агент давно не отвечал): сказать, насколько
    let age = now - at;
    if at > 0 && age > 15 * 60 && !(w.resets_at > 0 && w.resets_at <= now) {
        value.push_str(&format!(" · данные {} назад", ago(age)));
    }
    let hint = match status {
        BAD => "Лимит исчерпан: агент ответит после сброса.",
        WARN => "Лимит почти исчерпан.",
        _ => "",
    };
    Check::new("ai", &format!("{agent}: лимит {what}"), status, &value).hint(hint)
}

/// Через сколько: «через 1 ч 56 мин», «через 4 дн 23 ч», «через 12 мин».
pub(crate) fn until(at: i64, now: i64) -> String {
    let s = (at - now).max(0);
    let (d, h, m) = (s / 86_400, s % 86_400 / 3600, s % 3600 / 60);
    if d > 0 {
        format!("через {d} дн {h} ч")
    } else if h > 0 {
        format!("через {h} ч {m} мин")
    } else {
        format!("через {} мин", m.max(1))
    }
}

pub(crate) fn ago(s: i64) -> String {
    if s < 3600 {
        format!("{} мин", s / 60)
    } else if s < 86_400 {
        format!("{} ч", s / 3600)
    } else {
        format!("{} дн.", s / 86_400)
    }
}

/// Подпись окна: по его длине, а когда длина неизвестна — по месту (первичное окно — пятичасовое, второе — недельное).
fn what_of(w: &Window, primary: bool) -> &'static str {
    if w.minutes >= 24 * 60 || w.minutes == 0 && !primary {
        "на неделю"
    } else {
        "на 5 ч"
    }
}

/// Расход: «сегодня 3,4 млн · за 7 дней 41 млн».
fn usage_row(agent: &str, us: &Usage, now: i64) -> Option<Check> {
    let today = local_day(now);
    let week = local_day(now - 6 * 86_400);
    let (mut t, mut w) = (Tok::default(), Tok::default());
    for ((d, _), x) in us {
        if *d == today {
            t.add(x);
        }
        if *d >= week {
            w.add(x);
        }
    }
    if w.total() == 0 {
        return None;
    }
    let value = format!("сегодня {} · за 7 дней {}", fmt_tokens(t.total()), fmt_tokens(w.total()));
    let hint = format!(
        "За 7 дней: новые на входе {}, ответы {}, чтение из кэша {}, запись в кэш {}.",
        fmt_tokens(w.input),
        fmt_tokens(w.output),
        fmt_tokens(w.cache_read),
        fmt_tokens(w.cache_write)
    );
    Some(Check::new("ai", &format!("{agent}: расход токенов"), INFO, &value).hint(&hint))
}

/// Проверка «Лимиты и расход ИИ»: строки только для агентов, о которых что-то известно.
pub fn check() -> Vec<Check> {
    crate::footprint::background_priority();
    let now = crate::ui::now_secs();
    let mut out = Vec::new();
    for (agent, lim, us) in [("Claude Code", claude_limits(), claude_usage()), ("Codex", codex_limits(), codex_usage())] {
        if let Some(l) = lim {
            for (w, primary) in [(l.five, true), (l.week, false)] {
                if let Some(w) = w {
                    out.push(limit_row(agent, what_of(&w, primary), &w, l.at, now));
                }
            }
        }
        out.extend(usage_row(agent, &us, now));
    }
    out.extend(no_limits_yet_rows());
    out.extend(missing_agents_row(&missing_agent_names()));
    out
}

fn claude_installed() -> bool {
    claude_dir().is_some() || crate::procs::which("claude").is_some()
}

/// Claude Code или Codex установлены, а лимитов от них ещё не было: без строки непонятно, почему колец и процентов нет.
fn no_limits_yet_rows() -> Vec<Check> {
    let mut out = Vec::new();
    let codex = crate::agents::find("codex").is_some_and(|a| a.installed());
    for (name, installed, has) in [("Claude Code", claude_installed(), claude_limits().is_some()), ("Codex", codex, codex_limits().is_some())] {
        if installed && !has {
            out.push(
                Check::new("ai", &format!("{name}: лимиты"), INFO, "пока нет данных")
                    .hint("Лимиты появятся после первого ответа агента: программа берёт их из его ответов."),
            );
        }
    }
    out
}

/// Агенты, которых нет на этом компьютере, но чьи проверки включены в «Показателях».
fn missing_agent_names() -> Vec<&'static str> {
    let mut v = Vec::new();
    if crate::prefs::unit_enabled("claude", true) && !claude_installed() {
        v.push("Claude Code");
    }
    v.extend(crate::agents::AGENTS.iter().filter(|a| crate::prefs::unit_enabled(a.id, true) && !a.installed()).map(|a| a.name));
    v
}

/// Одна сжатая строка вместо молчания: почему на вкладке нет строк про этих агентов.
fn missing_agents_row(names: &[&str]) -> Option<Check> {
    if names.is_empty() {
        return None;
    }
    Some(Check::new("ai", "Агенты не найдены", INFO, &names.join(", ")).hint("Их проверки появятся здесь, когда агент будет установлен."))
}

// ───────────────────────────── строка состояния Claude Code ─────────────────────────────

/// `aptechka --statusline`: Claude Code передаёт на вход JSON сеанса; из него сохраняем только
/// `rate_limits` (проценты и время сброса) в папку сеанса. Если у человека была своя строка
/// состояния — запускаем её с тем же входом и печатаем её вывод (её Аптечка не заменяет, а продолжает).
pub fn statusline_main() {
    use std::io::{Read, Write};
    let mut input = Vec::new();
    let _ = std::io::stdin().take(1 << 20).read_to_end(&mut input);
    if let Some(rl) = serde_json::from_slice::<serde_json::Value>(&input).ok().and_then(|v| v.get("rate_limits").cloned()) {
        let body = serde_json::json!({ "rate_limits": rl, "at": crate::ui::now_secs() }).to_string();
        // писать, только если цифры поменялись (строка состояния обновляется часто)
        let p = claude_limits_path();
        let old = crate::store::read_capped(&p, 4096).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let stale = old.as_ref().and_then(|o| o.get("at")).and_then(|x| x.as_i64()).is_none_or(|at| crate::ui::now_secs() - at > 60);
        if old.as_ref().and_then(|o| o.get("rate_limits")) != Some(&rl) || stale {
            let _ = crate::store::write_private_quick(&p, body.as_bytes());
        }
        // копия на диск (переживает перезагрузку) — реже: когда цифры поменялись или копии больше 10 минут
        let sp = claude_limits_saved_path();
        let saved = crate::store::read_capped(&sp, 4096).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let old_copy = saved.as_ref().and_then(|o| o.get("at")).and_then(|x| x.as_i64()).is_none_or(|at| crate::ui::now_secs() - at > 600);
        if saved.as_ref().and_then(|o| o.get("rate_limits")) != Some(&rl) || old_copy {
            let _ = crate::store::write_private_quick(&sp, body.as_bytes());
        }
    }
    if let Some(prev) = crate::prefs::load().get("claude_statusline_prev").and_then(|x| x.get("command")).and_then(|x| x.as_str()) {
        let out = run_prev(prev, &input);
        let _ = std::io::stdout().write_all(&out);
    }
}

/// Прежняя строка состояния человека — тем же способом, что Claude Code (через оболочку), с тем же входом.
fn run_prev(cmd: &str, input: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    let mut c = {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(cmd);
        c
    };
    #[cfg(windows)]
    let mut c = {
        // Claude Code на Windows запускает команды в Git Bash; нет его — cmd
        match crate::procs::which("bash") {
            Some(b) => {
                let mut c = Command::new(b);
                c.arg("-c").arg(cmd);
                c
            }
            None => {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(cmd);
                c
            }
        }
    };
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    let Ok(mut ch) = c.spawn() else { return Vec::new() };
    if let Some(mut si) = ch.stdin.take() {
        let _ = si.write_all(input);
    }
    ch.wait_with_output().map(|o| o.stdout).unwrap_or_default()
}

// ───────────────────────────── подключение строки состояния ─────────────────────────────

/// Наша команда для строки состояния: путь к этой программе в кавычках оболочки.
/// None — сборка из исходников (папка target/): её путь временный, в настройки Claude его не пишем.
fn our_command() -> Option<String> {
    let exe = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok())?;
    let s = exe.to_string_lossy().into_owned();
    let unix = s.replace('\\', "/");
    if unix.contains("/target/release/") || unix.contains("/target/debug/") || unix.contains("/target/x86_64-") {
        return None;
    }
    // Claude Code запускает команду через оболочку (на Windows — Git Bash): одинарные кавычки, косые вперёд
    let path = if cfg!(windows) { unix } else { s };
    Some(format!("'{}' --statusline", path.replace('\'', r"'\''")))
}

fn is_ours(cmd: &str) -> bool {
    cmd.contains("--statusline") && cmd.to_lowercase().contains("aptechka")
}

/// Что сделать с настройками Claude Code (чистая функция, проверяется тестами):
/// None — ничего не менять; Some((новые настройки, прежняя строка состояния человека для цепочки)).
fn plan(
    settings: &serde_json::Value,
    ours: &str,
    want: bool,
    prev: Option<&serde_json::Value>,
) -> Option<(serde_json::Value, Option<serde_json::Value>)> {
    let obj = settings.as_object()?;
    let cur = obj.get("statusLine");
    let cur_cmd = cur.and_then(|c| c.get("command")).and_then(|x| x.as_str());
    let mut new = obj.clone();
    if want {
        match cur_cmd {
            Some(c) if c == ours => return None,
            // наша, но программа переехала — поправить путь, прежняя строка человека остаётся как была
            Some(c) if is_ours(c) => {
                let mut sl = cur.cloned().unwrap_or_default();
                sl["command"] = ours.into();
                new.insert("statusLine".into(), sl);
                return Some((new.into(), prev.cloned()));
            }
            _ => {
                // своя у человека — запоминаем её и продолжаем (её отступы и прочее оставляем)
                let mut sl = match cur {
                    Some(c) if c.is_object() => c.clone(),
                    _ => serde_json::json!({ "type": "command" }),
                };
                sl["type"] = "command".into();
                sl["command"] = ours.into();
                new.insert("statusLine".into(), sl);
                return Some((new.into(), cur.cloned()));
            }
        }
    }
    // выключили: вернуть прежнюю строку человека (или убрать нашу)
    if !cur_cmd.is_some_and(is_ours) {
        return None;
    }
    match prev {
        Some(p) => {
            new.insert("statusLine".into(), p.clone());
        }
        None => {
            new.remove("statusLine");
        }
    }
    Some((new.into(), None))
}

/// Подключить (или отключить, если проверка «Лимиты и расход ИИ» выключена) строку состояния Claude Code.
/// Зовётся при запуске окна или виджета. Файл настроек — ссылка или не разбирается — не трогаем.
pub fn sync_claude_statusline() {
    sync_with(None);
}

/// `aptechka --claude-limits on|off` (off — для удаления программы: вернуть строку состояния человека).
pub fn sync_with(force: Option<bool>) {
    let Some(dir) = claude_dir() else { return };
    let Some(ours) = our_command() else { return };
    let want = force.unwrap_or_else(|| crate::prefs::unit_enabled("ai_limits", true));
    let p = dir.join("settings.json");
    if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
        crate::diag::log("лимиты Claude: settings.json — ссылка, не трогаю");
        return;
    }
    let text = match crate::store::read_capped(&p, 4 << 20) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "{}".into(),
        Err(_) => return,
    };
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else {
        crate::diag::log("лимиты Claude: settings.json не разбирается, не трогаю");
        return;
    };
    let prefs = crate::prefs::load();
    let prev = prefs.get("claude_statusline_prev").filter(|v| !v.is_null());
    let Some((new, keep_prev)) = plan(&settings, &ours, want, prev) else { return };
    if crate::fixes::dry_run() {
        crate::diag::log("DRY_RUN: подключил бы лимиты Claude (строка состояния)");
        return;
    }
    // копия до правки — в backups/ Аптечки; строки с ключами (в settings бывает env с API-ключом) — замаскированы
    if p.exists() {
        let stamp = crate::ui::now_secs();
        let bak = crate::store::private_dir().join("backups").join(format!("claude-settings.json.{stamp}.bak"));
        let _ = std::fs::create_dir_all(crate::store::private_dir().join("backups"));
        if crate::store::write_private(&bak, crate::footprint::redact_for_backup(&text).as_bytes()).is_err() {
            return;
        }
    }
    let Ok(body) = serde_json::to_string_pretty(&new) else { return };
    // сначала запомнить прежнюю строку человека (её зовёт statusline_main), потом править файл Claude:
    // при сбое между шагами строка человека не теряется
    if want && keep_prev.is_some() {
        crate::prefs::update(|v| v["claude_statusline_prev"] = keep_prev.clone().unwrap_or_default());
    }
    let mode = std::fs::metadata(&p).map(|m| permissions_mode(&m)).unwrap_or(0o600);
    if crate::store::write_mode(&p, format!("{body}\n").as_bytes(), mode).is_ok() {
        if !want {
            crate::prefs::update(|v| v["claude_statusline_prev"] = serde_json::Value::Null);
        }
        crate::diag::log(if want {
            "лимиты Claude: строка состояния подключена"
        } else {
            "лимиты Claude: строка состояния отключена"
        });
    }
}

#[cfg(unix)]
fn permissions_mode(m: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o777
}
#[cfg(not(unix))]
fn permissions_mode(_: &std::fs::Metadata) -> u32 {
    0o600
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_dirs_default_xdg_and_override() {
        let base = std::env::temp_dir().join(format!("aptechka-claude-dirs-{}", std::process::id()));
        let (home, xdg) = (base.join("home"), base.join("xdg"));
        for d in [home.join(".claude"), home.join(".config/claude"), xdg.join("claude"), base.join("custom")] {
            std::fs::create_dir_all(d).unwrap();
        }
        // обе папки есть — читаем обе; первая (для настроек) — обычная ~/.claude
        assert_eq!(claude_dirs_from(None, Some(home.clone()), None), vec![home.join(".claude"), home.join(".config/claude")]);
        // XDG_CONFIG_HOME задана — вторая папка там
        assert_eq!(claude_dirs_from(None, Some(home.clone()), Some(xdg.clone())), vec![home.join(".claude"), xdg.join("claude")]);
        // CLAUDE_CONFIG_DIR — только она
        assert_eq!(claude_dirs_from(Some(base.join("custom")), Some(home.clone()), None), vec![base.join("custom")]);
        // папки нет — пусто
        assert!(claude_dirs_from(Some(base.join("нет")), Some(home), None).is_empty());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn missing_agents_row_lists_names() {
        assert!(missing_agents_row(&[]).is_none());
        let c = missing_agents_row(&["Gemini CLI", "Qwen Code"]).unwrap();
        assert_eq!(c.value, "Gemini CLI, Qwen Code");
        assert_eq!(c.status, INFO);
    }

    #[test]
    fn weekday_by_date() {
        // 15 ноября 2026 — воскресенье, 17-е — вторник; 29 февраля 2028 — вторник; 1 января 1970 — четверг
        assert_eq!(weekday(2026, 11, 15), "вс");
        assert_eq!(weekday(2026, 11, 17), "вт");
        assert_eq!(weekday(2028, 2, 29), "вт");
        assert_eq!(weekday(1970, 1, 1), "чт");
    }

    #[test]
    fn when_day_always_names_the_day() {
        let now = 1_794_699_310; // ночь на 15.11.2026
        assert!(when_day(now + 3600, now).starts_with("сегодня в ") || when_day(now + 3600, now).starts_with("завтра в "));
        // через 4 суток — день недели и число
        let far = when_day(now + 4 * 86_400, now);
        assert!(far.split(' ').next().is_some_and(|w| ["пн", "вт", "ср", "чт", "пт", "сб", "вс"].contains(&w)), "{far}");
        assert!(far.contains(" в "), "{far}");
    }

    #[test]
    fn claude_limits_parse_and_reset() {
        // так лимиты лежат в файле строки состояния (и в его копии в папке настроек)
        let l = parse_claude_limits(
            r#"{"rate_limits":{"five_hour":{"used_percentage":5,"resets_at":1000},"seven_day":{"used_percentage":21,"resets_at":9000}},"at":500}"#,
        )
        .unwrap();
        // после перезагрузки показываем сохранённое: 5-часовое окно уже сбросилось — 0, недельное ещё идёт — 21
        assert_eq!(l.five.unwrap().now_pct(2000), 0.0);
        assert_eq!(l.week.unwrap().now_pct(2000), 21.0);
        assert_eq!(parse_claude_limits("{}"), None);
        assert_eq!(parse_claude_limits("не json"), None);
    }

    #[test]
    fn iso_time() {
        assert_eq!(iso_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_secs("2026-11-04T23:03:29.405Z"), Some(1_793_833_409));
        assert_eq!(iso_secs("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(iso_secs("вчера"), None);
    }

    #[test]
    fn claude_counts_each_answer_once() {
        let mut st = FileState::default();
        let line = |id: &str| {
            serde_json::json!({"type": "assistant", "timestamp": "2026-11-04T12:00:00Z",
                "message": {"id": id, "model": "claude-x", "usage": {"input_tokens": 10, "output_tokens": 5,
                "cache_read_input_tokens": 100, "cache_creation_input_tokens": 1}}})
        };
        let mut seen = HashSet::new();
        claude_line(&line("a"), &mut st, &mut seen);
        claude_line(&line("a"), &mut st, &mut seen);
        claude_line(&line("b"), &mut st, &mut seen);
        let t: Tok = st.usage.values().copied().fold(Tok::default(), |mut a, b| {
            a.add(&b);
            a
        });
        assert_eq!(t, Tok { input: 20, output: 10, cache_read: 200, cache_write: 2 });
        // тот же ответ «a» в ДРУГОМ файле (историю сессии скопировали в новый журнал) — второй раз не считается;
        // новый ответ «c» в том файле — считается
        let mut other = FileState::default();
        claude_line(&line("a"), &mut other, &mut seen);
        assert!(other.usage.is_empty());
        claude_line(&line("c"), &mut other, &mut seen);
        assert_eq!(other.usage.values().map(|t| t.output).sum::<u64>(), 5);
    }

    #[test]
    fn codex_limits_and_tokens() {
        let mut st = FileState::default();
        codex_line(&serde_json::json!({"type": "turn_context", "payload": {"model": "gpt-x"}}), &mut st, &mut HashSet::new());
        codex_line(
            &serde_json::json!({"timestamp": "2026-11-04T23:03:29.405Z", "type": "event_msg", "payload": {"type": "token_count",
                "info": {"last_token_usage": {"input_tokens": 100, "cached_input_tokens": 60, "output_tokens": 7}},
                "rate_limits": {"primary": {"used_percent": 85.0, "window_minutes": 300, "resets_at": 1_793_847_719_i64},
                    "secondary": {"used_percent": 13.0, "window_minutes": 10080, "resets_at": 1_794_434_519_i64}}}}),
            &mut st,
            &mut HashSet::new(),
        );
        let l = st.limits.unwrap();
        assert_eq!(l.five.unwrap().pct, 85.0);
        assert_eq!(l.week.unwrap().minutes, 10080);
        let (k, t) = st.usage.iter().next().unwrap();
        assert_eq!(k.1, "gpt-x");
        assert_eq!(*t, Tok { input: 40, output: 7, cache_read: 60, cache_write: 0 });
        // сброс прошёл — занятое не считается
        assert_eq!(l.five.unwrap().now_pct(1_793_847_720), 0.0);
        assert_eq!(l.top(1_793_847_720), Some(13.0));
    }

    #[test]
    fn statusline_plan() {
        let ours = "'/usr/bin/aptechka' --statusline";
        let mine = serde_json::json!({"type": "command", "command": "~/.claude/line.sh", "padding": 0});
        // пусто — ставим свою
        let (n, prev) = plan(&serde_json::json!({"model": "x"}), ours, true, None).unwrap();
        assert_eq!(n["statusLine"]["command"], ours);
        assert_eq!(n["model"], "x");
        assert!(prev.is_none());
        // уже наша — ничего
        assert!(plan(&n, ours, true, None).is_none());
        // у человека своя — продолжаем её, отступ сохраняем
        let s = serde_json::json!({"statusLine": mine});
        let (n, prev) = plan(&s, ours, true, None).unwrap();
        assert_eq!(n["statusLine"]["command"], ours);
        assert_eq!(n["statusLine"]["padding"], 0);
        assert_eq!(prev.as_ref(), Some(&mine));
        // программа переехала — путь новый, прежняя строка та же
        let (n2, prev2) = plan(&n, "'/opt/aptechka' --statusline", true, prev.as_ref()).unwrap();
        assert_eq!(n2["statusLine"]["command"], "'/opt/aptechka' --statusline");
        assert_eq!(prev2.as_ref(), Some(&mine));
        // выключили — вернуть строку человека
        let (n3, _) = plan(&n2, ours, false, prev.as_ref()).unwrap();
        assert_eq!(n3["statusLine"], mine);
        // выключили, а строка не наша — не трогаем
        assert!(plan(&s, ours, false, None).is_none());
        // не объект — не трогаем
        assert!(plan(&serde_json::json!([1]), ours, true, None).is_none());
    }

    #[test]
    fn project_names() {
        assert_eq!(project_name("/home/u/first_aid_cat"), "first_aid_cat");
        assert_eq!(project_name("/home/u/proj/"), "proj");
        assert_eq!(project_name(r"C:\Users\u\site"), "site");
        assert_eq!(project_name(""), "—");
        assert_eq!(project_name("/home/u/proj/.claude/worktrees/agent-a63c"), "proj");
    }

    #[test]
    fn token_format() {
        assert_eq!(fmt_tokens(812), "812");
        assert_eq!(fmt_tokens(52_300), "52 тыс.");
        assert_eq!(fmt_tokens(3_400_000), "3,4 млн");
    }
}

/// Замер на настоящих журналах этого компьютера (только чтение): cargo test --release real_logs -- --ignored --nocapture
#[cfg(test)]
#[test]
#[ignore]
fn real_logs_timing() {
    for pass in 1..=2 {
        let t = std::time::Instant::now();
        let n = claude_usage().len() + codex_usage().len();
        println!("проход {pass}: {n} записей, {:?}", t.elapsed());
    }
}

/// Независимые регрессионные тесты (REGRESSION_TESTS.md); подключены отдельным файлом, на сборку без `cfg(test)` не влияют.
#[cfg(test)]
#[path = "regress/limits.rs"]
mod regress;
