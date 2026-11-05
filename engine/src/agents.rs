//! ИИ-агенты в терминале, кроме Claude (он в diag.rs): Codex, Qwen Code, Kimi Code, Gemini CLI, Antigravity, Cursor, OpenCode.
//!
//! Для каждого: установлен ли, связь с его серверами, сессии (папка проекта, первая реплика, когда,
//! жива ли, занят ли) и команда «продолжить сессию».
//! Форматы Codex, Qwen и Kimi проверены на настоящих сессиях (образцы — tests/fixtures/agents,
//! записаны в ВМ 2026-10-31: codex-cli 0.166.0, qwen 0.26.4, kimi-code 2.4.1).
//! Gemini CLI и Antigravity — по документации, без живой проверки: читаем бережно, что найдём.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::diag::{Check, BAD, INFO, OK, WARN};

#[derive(Clone, Debug, Default)]
pub struct AgentSession {
    /// Код агента: "codex", "qwen", "kimi", "gemini", "agy".
    pub agent: &'static str,
    pub id: String,
    pub cwd: String,
    /// Первая реплика человека.
    pub title: String,
    /// Последнее изменение, секунды Unix.
    pub mtime: f64,
    /// Запущен ли сейчас процесс агента с этой сессией.
    pub live: bool,
    /// Занят ответом (по последним событиям файла); None — не понять.
    pub busy: Option<bool>,
    /// Файл (или папка) сессии.
    pub path: PathBuf,
}

pub struct Agent {
    pub id: &'static str,
    pub name: &'static str,
    /// Имена программы (для поиска в PATH и среди процессов).
    pub bins: &'static [&'static str],
    /// Где ещё бывает программа, кроме PATH (относительно домашней папки).
    pub extra_bins: &'static [&'static str],
    /// Папки данных (относительно домашней; первая найденная — его), переменная окружения важнее.
    pub dirs: &'static [&'static str],
    pub env_home: &'static str,
    /// Серверы для проверки связи: (подпись, адрес).
    pub hosts: &'static [(&'static str, &'static str)],
    /// statuspage-совместимый status.json, если есть.
    pub status: Option<&'static str>,
    pub sessions: fn(&Path) -> Vec<AgentSession>,
}

pub static AGENTS: &[Agent] = &[
    Agent {
        id: "codex",
        name: "Codex",
        bins: &["codex"],
        extra_bins: &[".local/bin/codex", ".codex/bin/codex"],
        dirs: &[".codex"],
        env_home: "CODEX_HOME",
        hosts: &[
            ("API OpenAI", "https://api.openai.com/v1/models"),
            ("ChatGPT", "https://chatgpt.com/backend-api/codex/responses"),
            ("сервером входа OpenAI", "https://auth.openai.com/favicon.ico"),
        ],
        status: Some("https://status.openai.com/api/v2/status.json"),
        sessions: codex_sessions,
    },
    Agent {
        id: "qwen",
        name: "Qwen Code",
        bins: &["qwen"],
        extra_bins: &[".local/bin/qwen"],
        dirs: &[".qwen"],
        env_home: "QWEN_HOME",
        hosts: &[("DashScope", "https://dashscope-intl.aliyuncs.com/")],
        status: None,
        sessions: qwen_sessions,
    },
    Agent {
        id: "kimi",
        name: "Kimi",
        bins: &["kimi"],
        extra_bins: &[".kimi-code/bin/kimi", ".local/bin/kimi"],
        dirs: &[".kimi-code", ".kimi"],
        env_home: "KIMI_CODE_HOME",
        hosts: &[("API Kimi", "https://api.kimi.com/"), ("Moonshot", "https://api.moonshot.ai/")],
        status: None,
        sessions: kimi_sessions,
    },
    Agent {
        id: "gemini",
        name: "Gemini CLI",
        bins: &["gemini"],
        extra_bins: &[".local/bin/gemini"],
        dirs: &[".gemini/tmp"],
        env_home: "",
        hosts: &[("Gemini API", "https://generativelanguage.googleapis.com/")],
        status: None,
        sessions: gemini_sessions,
    },
    Agent {
        id: "agy",
        name: "Antigravity",
        bins: &["agy"],
        extra_bins: &[".local/bin/agy"],
        dirs: &[".gemini/antigravity-cli", ".gemini/antigravity"],
        env_home: "",
        hosts: &[("Google Cloud Code", "https://cloudcode-pa.googleapis.com/"), ("сервером входа Google", "https://accounts.google.com/favicon.ico")],
        status: None,
        sessions: agy_sessions,
    },
    Agent {
        id: "cursor",
        name: "Cursor",
        bins: &["cursor-agent"],
        extra_bins: &[".local/bin/cursor-agent"],
        dirs: &[".cursor/chats"],
        env_home: "",
        hosts: &[("Cursor API", "https://api2.cursor.sh/"), ("cursor.com", "https://cursor.com/favicon.ico")],
        status: Some("https://status.cursor.com/api/v2/status.json"),
        sessions: cursor_sessions,
    },
    Agent {
        id: "opencode",
        name: "OpenCode",
        bins: &["opencode"],
        extra_bins: &[".opencode/bin/opencode", ".local/bin/opencode"],
        dirs: &[".local/share/opencode"],
        env_home: "",
        hosts: &[("opencode.ai", "https://opencode.ai/"), ("models.dev", "https://models.dev/api.json")],
        status: None,
        sessions: opencode_sessions,
    },
];

pub fn find(id: &str) -> Option<&'static Agent> {
    AGENTS.iter().find(|a| a.id == id)
}

fn home() -> PathBuf {
    crate::store::home_dir().unwrap_or_default()
}

impl Agent {
    /// Папка данных агента (None — не найдена).
    pub fn data_dir(&self) -> Option<PathBuf> {
        if !self.env_home.is_empty() {
            if let Some(p) = std::env::var_os(self.env_home).map(PathBuf::from).filter(|p| p.is_dir()) {
                return Some(p);
            }
        }
        self.dirs.iter().map(|d| home().join(d)).find(|p| p.is_dir())
    }

    /// Путь к программе, если она установлена.
    pub fn binary(&self) -> Option<PathBuf> {
        self.bins.iter().find_map(|b| crate::procs::which(b)).or_else(|| self.extra_bins.iter().map(|p| home().join(p)).find(|p| p.is_file()))
    }

    pub fn installed(&self) -> bool {
        self.binary().is_some() || self.data_dir().is_some()
    }

    /// Сессии, новые сверху; живые отмечены по запущенным процессам.
    pub fn list_sessions(&self, limit: usize) -> Vec<AgentSession> {
        let Some(dir) = self.data_dir() else { return Vec::new() };
        let mut v = (self.sessions)(&dir);
        v.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
        mark_live(self, &mut v);
        v.truncate(limit);
        v
    }
}

/// Строка для PowerShell в одинарных кавычках. PowerShell считает кавычкой не только «'», но и
/// типографские ‘ ’ ‚ ‛ — их тоже удваиваем, иначе папка с именем «a’; calc; ’» выполнила бы команду.
pub fn ps_quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// Команда, чтобы продолжить сессию в терминале (с переходом в папку проекта).
/// Номер сессии годится для команды: только буквы, цифры, «-», «_», «.» (номер берётся из файлов агента;
/// с «;» или «$(…)» он выполнился бы в терминале как команда).
pub fn safe_id(id: &str) -> Option<&str> {
    (!id.is_empty() && id.len() <= 128 && !id.starts_with('-') && id.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))).then_some(id)
}

pub fn resume_command(s: &AgentSession) -> String {
    if safe_id(&s.id).is_none() {
        return String::new();
    }
    let run = match s.agent {
        "codex" => format!("codex resume {}", s.id),
        "qwen" => format!("qwen -r {}", s.id),
        "kimi" => format!("kimi -r {}", s.id),
        "gemini" => format!("gemini --resume {}", s.id),
        "agy" => format!("agy --conversation {}", s.id),
        "cursor" => format!("cursor-agent --resume {}", s.id),
        "opencode" => format!("opencode -s {}", s.id),
        _ => return String::new(),
    };
    // папки уже нет — без cd (иначе команда упала бы, не дойдя до агента)
    if s.cwd.is_empty() || !std::path::Path::new(&s.cwd).is_dir() {
        run
    } else if cfg!(windows) {
        format!("Set-Location -LiteralPath {}; {run}", ps_quote(&s.cwd))
    } else {
        format!("cd '{}' && {run}", s.cwd.replace('\'', r"'\''"))
    }
}

// ───────────────────────────── Чтение файлов ─────────────────────────────

fn mtime(p: &Path) -> f64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Строка до '\n', но в buf — не больше cap байт: остаток длинной строки пропускается, не попадая в память
/// (журнал сессии в гигабайт без переводов строки раньше читался в память целиком). 0 — конец файла.
pub(crate) fn read_line_capped(rd: &mut impl std::io::BufRead, buf: &mut Vec<u8>, cap: usize) -> usize {
    buf.clear();
    let mut total = 0;
    loop {
        let chunk = match rd.fill_buf() {
            Ok(c) if !c.is_empty() => c,
            _ => return total,
        };
        let (take, done) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        let room = cap.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..take.min(room)]);
        rd.consume(take);
        total += take;
        if done {
            return total;
        }
    }
}

/// Строк JSON подряд из файла (не больше n, строки длиннее 1 МБ — пропуск), битые пропускаем.
fn json_lines(f: std::fs::File, n: usize) -> Vec<serde_json::Value> {
    const CAP: usize = 1 << 20;
    let mut rd = std::io::BufReader::new(f);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    for _ in 0..n {
        let got = read_line_capped(&mut rd, &mut buf, CAP);
        if got == 0 {
            break;
        }
        if got > CAP {
            continue;
        }
        if let Ok(v) = serde_json::from_slice(&buf) {
            out.push(v);
        }
    }
    out
}

/// Первые n строк файла как JSON (битые пропускаем).
fn head_json(p: &Path, n: usize) -> Vec<serde_json::Value> {
    let Ok(f) = crate::store::open_regular(p) else { return Vec::new() };
    json_lines(f, n)
}

/// Последние ~bytes байт файла построчно как JSON (первую, обрезанную, строку пропускаем).
fn tail_json(p: &Path, bytes: u64) -> Vec<serde_json::Value> {
    let Ok(mut f) = crate::store::open_regular(p) else { return Vec::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = String::new();
    let _ = f.take(bytes).read_to_string(&mut buf);
    let mut lines: Vec<&str> = buf.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines.iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn s<'a>(v: &'a serde_json::Value, path: &[&str]) -> &'a str {
    let mut cur = v;
    for k in path {
        cur = match cur.get(k) {
            Some(x) => x,
            None => return "",
        };
    }
    cur.as_str().unwrap_or("")
}

/// Однострочный кусок первой реплики.
fn clip(t: &str) -> String {
    let one: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut c: String = one.chars().take(80).collect();
    if one.chars().count() > 80 {
        c.push('…');
    }
    c
}

fn files_with(dir: &Path, ext: &str, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // тип — без перехода по ссылке: ссылка на «/» в папке сессий обошла бы весь диск; и не больше 5000 файлов
        let Ok(ft) = e.file_type() else { continue };
        // предел обхода — 50 000 файлов (сортировка по свежести — потом, по всем найденным)
        if out.len() >= 50_000 {
            return;
        }
        if ft.is_dir() {
            if depth > 0 {
                files_with(&p, ext, depth - 1, out);
            }
        } else if ft.is_file() && name.ends_with(ext) {
            out.push(p);
        }
    }
}

/// Самые свежие файлы (чтобы не разбирать тысячи старых сессий).
fn newest(mut files: Vec<PathBuf>, n: usize) -> Vec<(PathBuf, f64)> {
    let mut v: Vec<(PathBuf, f64)> = files
        .drain(..)
        .map(|p| {
            let t = mtime(&p);
            (p, t)
        })
        .collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(n);
    v
}

const SCAN: usize = 40;

// ───────────────────────────── Codex ─────────────────────────────

/// ~/.codex/sessions/YYYY/MM/DD/rollout-<время>-<id>.jsonl. Первая строка — session_meta (id, cwd);
/// первая реплика — response_item/message с role=user, не служебная (<environment_context> и т. п.);
/// занят — последнее из событий task_started / task_complete|turn_aborted|error — task_started.
fn codex_sessions(dir: &Path) -> Vec<AgentSession> {
    let mut files = Vec::new();
    files_with(&dir.join("sessions"), ".jsonl", 4, &mut files);
    newest(files, SCAN)
        .into_iter()
        .map(|(p, t)| {
            let head = head_json(&p, 200);
            let meta = head.iter().find(|j| s(j, &["type"]) == "session_meta");
            let mut id = meta.map(|m| s(m, &["payload", "id"]).to_string()).unwrap_or_default();
            if id.is_empty() {
                // запасной путь: последние 36 символов имени файла
                let stem = p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                id = stem.chars().rev().take(36).collect::<Vec<_>>().into_iter().rev().collect();
            }
            let cwd = meta.map(|m| s(m, &["payload", "cwd"]).to_string()).unwrap_or_default();
            let title = head
                .iter()
                .filter(|j| s(j, &["type"]) == "response_item" && s(j, &["payload", "role"]) == "user")
                .flat_map(|j| j["payload"]["content"].as_array().cloned().unwrap_or_default())
                .map(|c| s(&c, &["text"]).to_string())
                .find(|t| !t.trim_start().starts_with('<') && !t.trim().is_empty())
                .map(|t| clip(&t))
                .unwrap_or_default();
            let busy =
                tail_json(&p, 64 * 1024).iter().rev().filter(|j| s(j, &["type"]) == "event_msg").find_map(|j| match s(j, &["payload", "type"]) {
                    "task_started" => Some(true),
                    "task_complete" | "turn_complete" | "turn_aborted" | "error" => Some(false),
                    _ => None,
                });
            AgentSession { agent: "codex", id, cwd, title, mtime: t, live: false, busy, path: p }
        })
        .collect()
}

// ───────────────────────────── Qwen Code ─────────────────────────────

/// ~/.qwen/projects/<папка>/chats/<id>.jsonl (раньше ~/.qwen/tmp/<хэш>/chats/). Записи:
/// type user|assistant|system|tool_result, cwd, sessionId, message.parts[0].text.
/// Занят — последняя несистемная запись не от ассистента.
fn qwen_sessions(dir: &Path) -> Vec<AgentSession> {
    let mut files = Vec::new();
    for sub in ["projects", "tmp"] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        for e in rd.flatten() {
            files_with(&e.path().join("chats"), ".jsonl", 0, &mut files);
        }
    }
    newest(files, SCAN)
        .into_iter()
        .map(|(p, t)| {
            let head = head_json(&p, 60);
            let id = head
                .iter()
                .map(|j| s(j, &["sessionId"]))
                .find(|x| !x.is_empty())
                .map(String::from)
                .unwrap_or_else(|| p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default());
            let cwd = head.iter().map(|j| s(j, &["cwd"])).find(|x| !x.is_empty()).unwrap_or("").to_string();
            let title = head
                .iter()
                .filter(|j| s(j, &["type"]) == "user")
                .filter_map(|j| j["message"]["parts"].as_array().and_then(|a| a.first()).map(|x| s(x, &["text"]).to_string()))
                .find(|t| !t.trim().is_empty())
                .map(|t| clip(&t))
                .unwrap_or_default();
            let busy =
                tail_json(&p, 64 * 1024).iter().rev().map(|j| s(j, &["type"]).to_string()).find(|ty| ty != "system").map(|ty| ty != "assistant");
            AgentSession { agent: "qwen", id, cwd, title, mtime: t, live: false, busy, path: p }
        })
        .collect()
}

// ───────────────────────────── Kimi Code ─────────────────────────────

/// ~/.kimi-code/session_index.jsonl: {sessionId, sessionDir, workDir}. В папке сессии state.json
/// (updatedAt мс, lastTurnReason) и agents/main/wire.jsonl (turn.prompt — реплика человека;
/// agent.turn.started / agent.turn.ended — начало и конец хода).
/// Старый kimi-cli (~/.kimi/sessions/<md5>/<id>/context.jsonl) — только id и время.
fn kimi_sessions(dir: &Path) -> Vec<AgentSession> {
    let mut out = Vec::new();
    let index = dir.join("session_index.jsonl");
    if index.is_file() {
        let mut seen = std::collections::HashSet::new();
        // свежие записи — в конце файла: читаем хвост (последние ~2 МБ), а не первые строки
        let mut rows: Vec<serde_json::Value> = tail_json(&index, 2 << 20);
        rows.reverse();
        for j in rows {
            let id = s(&j, &["sessionId"]).to_string();
            if id.is_empty() || !seen.insert(id.clone()) {
                continue;
            }
            let sdir = PathBuf::from(s(&j, &["sessionDir"]));
            if !sdir.is_dir() {
                continue;
            }
            let wire = sdir.join("agents").join("main").join("wire.jsonl");
            let state: serde_json::Value =
                crate::store::read_text(sdir.join("state.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let t = state.get("updatedAt").and_then(|x| x.as_f64()).map(|ms| ms / 1000.0).unwrap_or_else(|| mtime(&wire));
            let title = head_json(&wire, 80)
                .iter()
                .filter(|j| s(j, &["type"]) == "turn.prompt")
                .filter_map(|j| j["input"].as_array().and_then(|a| a.iter().find_map(|x| x.get("text").and_then(|t| t.as_str()).map(String::from))))
                .find(|t| !t.trim().is_empty())
                .map(|t| clip(&t))
                .unwrap_or_default();
            let busy = tail_json(&wire, 64 * 1024).iter().rev().find_map(|j| match s(j, &["type"]) {
                "agent.turn.started" => Some(true),
                "agent.turn.ended" | "turn.ended" => Some(false),
                _ => None,
            });
            let cwd = [s(&j, &["workDir"]), s(&state, &["cwd"])].into_iter().find(|x| !x.is_empty()).unwrap_or("").to_string();
            out.push(AgentSession { agent: "kimi", id, cwd, title, mtime: t, live: false, busy, path: wire });
            if out.len() >= SCAN {
                break;
            }
        }
    } else if let Ok(rd) = std::fs::read_dir(dir.join("sessions")) {
        // старый kimi-cli
        for e in rd.flatten() {
            let Ok(rd2) = std::fs::read_dir(e.path()) else { continue };
            for s2 in rd2.flatten() {
                let ctx = s2.path().join("context.jsonl");
                if ctx.is_file() {
                    let id = s2.file_name().to_string_lossy().to_string();
                    out.push(AgentSession { agent: "kimi", id, mtime: mtime(&ctx), path: ctx, ..Default::default() });
                }
            }
        }
    }
    out
}

// ───────────────────────────── Gemini CLI, Antigravity ─────────────────────────────

/// ~/.gemini/tmp/<хэш проекта>/chats/*.json(l): sessionId внутри; папку проекта по хэшу не узнать.
fn gemini_sessions(dir: &Path) -> Vec<AgentSession> {
    let mut files = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            files_with(&e.path().join("chats"), ".json", 0, &mut files);
            files_with(&e.path().join("chats"), ".jsonl", 0, &mut files);
        }
    }
    newest(files, SCAN)
        .into_iter()
        .map(|(p, t)| {
            // файл сессии больше 32 МБ целиком не читаем (память)
            let small = std::fs::metadata(&p).map(|m| m.len() < 32 << 20).unwrap_or(false);
            let txt = if small { crate::store::read_text(&p).unwrap_or_default() } else { String::new() };
            let first: serde_json::Value =
                serde_json::from_str(&txt).or_else(|_| serde_json::from_str(txt.lines().next().unwrap_or(""))).unwrap_or_default();
            let id = [s(&first, &["sessionId"]), s(&first, &["id"])]
                .into_iter()
                .find(|x| !x.is_empty())
                .map(String::from)
                .unwrap_or_else(|| p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default());
            AgentSession { agent: "gemini", id, mtime: t, path: p, ..Default::default() }
        })
        .collect()
}

/// ~/.gemini/antigravity-cli/conversations/<id>.* — только код и время (формат закрыт).
fn agy_sessions(dir: &Path) -> Vec<AgentSession> {
    let mut files = Vec::new();
    for ext in [".jsonl", ".json", ".pb", ".db", ".sqlite"] {
        files_with(&dir.join("conversations"), ext, 1, &mut files);
    }
    newest(files, SCAN)
        .into_iter()
        .map(|(p, t)| {
            let id = p.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
            AgentSession { agent: "agy", id, mtime: t, path: p, ..Default::default() }
        })
        .collect()
}

// ───────────────────────────── Cursor ─────────────────────────────

/// md5 полного пути папки — так Cursor называет папку проекта в ~/.cursor/chats.
fn md5_hex(path: &str) -> String {
    format!("{:x}", md5::compute(path.as_bytes()))
}

/// Папки, среди которых ищем проект по md5: домашняя, её подпапки на два уровня и папки
/// запущенных процессов cursor-agent.
fn cursor_candidates() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    let h = home();
    let mut add = |p: &Path| {
        let s = p.to_string_lossy().to_string();
        m.insert(md5_hex(&s), s);
    };
    add(&h);
    if let Ok(rd) = std::fs::read_dir(&h) {
        for e in rd.flatten().filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.')) {
            add(&e.path());
            if let Ok(rd2) = std::fs::read_dir(e.path()) {
                for e2 in rd2.flatten().take(400).filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.')) {
                    add(&e2.path());
                }
            }
        }
    }
    for p in crate::procs::list().into_iter().filter(|p| p.is("cursor-agent")) {
        if !p.cwd.is_empty() {
            add(Path::new(&p.cwd));
        }
    }
    m
}

/// ~/.cursor/chats/<md5 папки проекта>/<id чата>/store.db (SQLite — первую реплику не читаем).
fn cursor_sessions(dir: &Path) -> Vec<AgentSession> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return v };
    let cands = cursor_candidates();
    for proj in rd.flatten() {
        let hash = proj.file_name().to_string_lossy().to_string();
        let cwd = cands.get(&hash).cloned().unwrap_or_default();
        let Ok(chats) = std::fs::read_dir(proj.path()) else { continue };
        for c in chats.flatten() {
            let db = c.path().join("store.db");
            if !db.is_file() {
                continue;
            }
            let wal = c.path().join("store.db-wal");
            let t = mtime(&db).max(mtime(&wal));
            v.push(AgentSession {
                agent: "cursor",
                id: c.file_name().to_string_lossy().to_string(),
                cwd: cwd.clone(),
                mtime: t,
                path: db,
                ..Default::default()
            });
        }
    }
    v
}

// ───────────────────────────── OpenCode ─────────────────────────────

/// Самая большая база (и журнал -wal), которую согласна скопировать.
const OC_MAX_COPY: u64 = 256 << 20;
/// Всего на чтение базы: копирование и запрос.
const OC_BUDGET: std::time::Duration = std::time::Duration::from_secs(8);

/// OpenCode (1.19+) хранит сессии в SQLite (~/.local/share/opencode/opencode.db, режим WAL). Чужую программу
/// не запускаем (обычная проверка этого не делает), базу читаем сами и только на чтение.
/// Саму базу не открываем: копируем opencode.db и opencode.db-wal (обычные файлы, без ссылок, не больше
/// 256 МБ) в закрытую временную папку и читаем копию. Так мы не берём блокировок у работающего OpenCode,
/// не создаём -wal/-shm рядом с его базой и видим свежие записи из журнала (с `immutable=1` журнал
/// игнорируется, и список отстал бы). Копия удаляется сразу после чтения.
/// Кэш — по «личности» файлов (время изменения + размер базы и журнала).
fn opencode_sessions(dir: &Path) -> Vec<AgentSession> {
    opencode_read(dir).unwrap_or_default()
}

type OcCache = Option<(PathBuf, Vec<(u64, u64)>, Vec<AgentSession>)>;
static OC_CACHE: std::sync::Mutex<OcCache> = std::sync::Mutex::new(None);
static OC_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Последнее чтение базы OpenCode не удалось (повреждена, занята, схема незнакомая).
pub(crate) fn opencode_unreadable() -> bool {
    OC_FAILED.load(std::sync::atomic::Ordering::Relaxed)
}

fn oc_stamp(db: &Path) -> Vec<(u64, u64)> {
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    [db.to_path_buf(), wal]
        .iter()
        .map(|p| {
            let m = std::fs::metadata(p).ok();
            let t = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64);
            (t.unwrap_or(0), m.map_or(0, |m| m.len()))
        })
        .collect()
}

fn opencode_read(dir: &Path) -> Result<Vec<AgentSession>, String> {
    let db = dir.join("opencode.db");
    if !db.is_file() {
        OC_FAILED.store(false, std::sync::atomic::Ordering::Relaxed);
        return Ok(Vec::new());
    }
    let stamp = oc_stamp(&db);
    if let Ok(c) = OC_CACHE.lock() {
        if let Some((p, st, v)) = c.as_ref() {
            if *p == db && *st == stamp {
                return Ok(v.clone());
            }
        }
    }
    let r = opencode_read_db(&db);
    OC_FAILED.store(r.is_err(), std::sync::atomic::Ordering::Relaxed);
    if let Ok(v) = &r {
        if let Ok(mut c) = OC_CACHE.lock() {
            *c = Some((db.clone(), stamp, v.clone()));
        }
    }
    r
}

/// Копия на время чтения; Drop стирает её, что бы ни случилось.
struct TempCopy(PathBuf);
impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Скопировать обычный файл (не ссылку), не больше max байт и до срока.
fn oc_copy(src: &Path, dst: &Path, max: u64, deadline: std::time::Instant) -> Result<(), String> {
    if std::fs::symlink_metadata(src).map_err(|e| e.to_string())?.file_type().is_symlink() {
        return Err("ссылка вместо файла".into());
    }
    let mut f = crate::store::open_regular(src).map_err(|e| e.to_string())?;
    if f.metadata().map_err(|e| e.to_string())?.len() > max {
        return Err("файл слишком большой".into());
    }
    let mut out = std::fs::File::create(dst).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        if std::time::Instant::now() > deadline {
            return Err("слишком долго".into());
        }
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        total += n as u64;
        if total > max {
            return Err("файл слишком большой".into());
        }
        std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| e.to_string())?;
    }
}

fn opencode_read_db(db: &Path) -> Result<Vec<AgentSession>, String> {
    let deadline = std::time::Instant::now() + OC_BUDGET;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = crate::store::private_dir().join(format!("oc_{}_{nanos}_{n}", std::process::id()));
    #[allow(unused_mut)]
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&tmp).map_err(|e| e.to_string())?;
    let guard = TempCopy(tmp);
    let copy = guard.0.join("opencode.db");
    oc_copy(db, &copy, OC_MAX_COPY, deadline)?;
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    if std::fs::symlink_metadata(&wal).is_ok() {
        // журнала может не быть (OpenCode закрыт) — тогда читаем одну базу
        oc_copy(&wal, &guard.0.join("opencode.db-wal"), OC_MAX_COPY, deadline)?;
    }
    opencode_query(&copy, db, deadline)
}

/// Запрос к копии. Таблица `session_v2` (новые версии) или `session` (старые); нужные столбцы находим по
/// PRAGMA table_info — если их нет, это незнакомая схема: ошибка, а не паника.
fn opencode_query(copy: &Path, orig: &Path, deadline: std::time::Instant) -> Result<Vec<AgentSession>, String> {
    use rusqlite::{types::Value, Connection, OpenFlags};
    let mut conn =
        Connection::open_with_flags(copy, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| e.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(2)).map_err(|e| e.to_string())?;
    let _ = conn.progress_handler(10_000, Some(move || std::time::Instant::now() > deadline));
    let mut out: Vec<AgentSession> = Vec::new();
    let mut found = false;
    for table in ["session_v2", "session"] {
        let cols: Vec<String> = {
            let mut st = conn.prepare(&format!("PRAGMA table_info({table})")).map_err(|e| e.to_string())?;
            let rows = st.query_map([], |r| r.get::<_, String>(1)).map_err(|e| e.to_string())?;
            rows.flatten().collect()
        };
        if !["id", "title", "directory", "time_updated"].iter().all(|c| cols.iter().any(|x| x == c)) {
            continue;
        }
        found = true;
        let mut cond = Vec::new();
        if cols.iter().any(|c| c == "parent_id") {
            cond.push("parent_id IS NULL"); // подсессии (помощники) не показываем
        }
        if cols.iter().any(|c| c == "time_archived") {
            cond.push("time_archived IS NULL");
        }
        let wh = if cond.is_empty() { String::new() } else { format!(" WHERE {}", cond.join(" AND ")) };
        let sql = format!("SELECT id, title, directory, time_updated FROM {table}{wh} ORDER BY time_updated DESC LIMIT {SCAN}");
        let mut st = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut rows = st.query([]).map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            // значения читаем как есть (Value): строка с неожиданным типом не роняет весь список
            let text = |i: usize| match r.get::<_, Value>(i) {
                Ok(Value::Text(t)) => t,
                _ => String::new(),
            };
            let id = text(0);
            if id.is_empty() {
                continue;
            }
            let t = match r.get::<_, Value>(3) {
                Ok(Value::Integer(n)) => n as f64,
                Ok(Value::Real(x)) => x,
                _ => 0.0,
            };
            out.push(AgentSession {
                agent: "opencode",
                id,
                cwd: text(2),
                title: crate::footprint::mask_secrets(&clip(&text(1))),
                // время в миллисекундах; на случай секунд — как есть
                mtime: if t > 1e11 { t / 1000.0 } else { t },
                path: orig.to_path_buf(),
                ..Default::default()
            });
        }
    }
    if !found {
        return Err("незнакомая схема базы".into());
    }
    out.sort_by(|a, b| b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(SCAN);
    Ok(out)
}

// ───────────────────────────── Живые сессии ─────────────────────────────

/// Отметить живые: процесс агента держит файл сессии открытым (Linux, /proc/<pid>/fd),
/// а если так не понять — самая свежая сессия в папке, где запущен процесс (macOS — папку даёт lsof),
/// а если и папка неизвестна (Windows) — самая свежая сессия, начатая после запуска процесса.
fn mark_live(a: &Agent, v: &mut Vec<AgentSession>) {
    // «в терминале» — только процессы с терминалом: фоновые серверы и служебные помощники агента
    // (opencode serve, codex app-server) — не сессия человека. И отбор «обёрток» — уже среди них: иначе
    // настоящий Codex, запустивший своих помощников с тем же именем, сам считался «обёрткой» и пропадал
    let procs =
        crate::procs::without_wrappers(crate::procs::list().into_iter().filter(|p| a.bins.iter().any(|b| p.is(b)) && p.tty != Some(false)).collect());
    for p in &procs {
        let open = open_files(p.pid);
        let mut hit = false;
        for sess in v.iter_mut() {
            if open.iter().any(|f| f == &sess.path || f.starts_with(sess.path.parent().unwrap_or(&sess.path)) && sess.agent == "kimi") {
                sess.live = true;
                hit = true;
            }
        }
        if hit {
            continue;
        }
        // папка процесса известна — сессия из неё; неизвестна (Windows) — самая свежая с его запуска
        let found = if !p.cwd.is_empty() {
            v.iter_mut().find(|x| x.cwd == p.cwd && !x.live)
        } else {
            v.iter_mut().find(|x| !x.live && (p.start <= 0.0 || x.mtime >= p.start - 120.0))
        };
        match found {
            Some(sess) => sess.live = true,
            // агент запущен, а файла его сессии ещё нет (только открыли, ни одного сообщения) — он всё равно
            // работает в терминале: показываем как новую сессию (без номера — продолжать пока нечего)
            None => v.insert(
                0,
                AgentSession {
                    agent: a.id,
                    id: String::new(),
                    cwd: p.cwd.clone(),
                    title: "новая сессия".into(),
                    mtime: p.start,
                    live: true,
                    busy: None,
                    path: PathBuf::new(),
                },
            ),
        }
    }
}

#[cfg(target_os = "linux")]
fn open_files(pid: i64) -> Vec<PathBuf> {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .map(|rd| rd.flatten().filter_map(|e| std::fs::read_link(e.path()).ok()).collect())
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn open_files(_pid: i64) -> Vec<PathBuf> {
    Vec::new()
}

// ───────────────────────────── Проверки ─────────────────────────────

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn ago(t: f64) -> String {
    let d = (now() - t).max(0.0);
    if d < 90.0 {
        "только что".into()
    } else if d < 3600.0 * 1.5 {
        format!("{:.0} мин назад", d / 60.0)
    } else if d < 86400.0 * 1.5 {
        format!("{:.0} ч назад", d / 3600.0)
    } else {
        format!("{:.0} дн назад", d / 86400.0)
    }
}

/// Связь с сервером агента — общая проверка (diag::site_row).
pub fn reach(title: &str, url: &str) -> Check {
    crate::diag::site_row("ai", title, url)
}

/// Строки проверки одного агента; пусто, если он не установлен.
pub fn check(a: &Agent) -> Vec<Check> {
    if !a.installed() {
        return Vec::new();
    }
    let mut out: Vec<Check> = a.hosts.iter().map(|(t, u)| reach(&format!("{}: связь с {t}", a.name), u)).collect();
    if let Some(url) = a.status {
        let (_, body, _) = crate::diag::http_get(url, 6, 200_000, None);
        if let Some(st) = serde_json::from_str::<serde_json::Value>(&body).ok().and_then(|j| j.get("status").cloned()) {
            let ind = st.get("indicator").and_then(|x| x.as_str()).unwrap_or("none");
            let status = match ind {
                "none" => OK,
                "minor" => WARN,
                _ => BAD,
            };
            let value = if ind == "none" {
                "всё работает".to_string()
            } else {
                st.get("description").and_then(|x| x.as_str()).unwrap_or(ind).to_string()
            };
            out.push(Check::new("ai", &format!("{}: серверы", a.name), status, &value).hint(if ind == "none" {
                ""
            } else {
                "Это проблема на их стороне, не у тебя."
            }));
        }
    }
    let sess = a.list_sessions(SCAN);
    let live: Vec<&AgentSession> = sess.iter().filter(|x| x.live).collect();
    let value = if !live.is_empty() {
        let busy = live.iter().filter(|x| x.busy == Some(true)).count();
        if busy > 0 {
            format!("{} в терминале · {} думает", live.len(), busy)
        } else {
            format!("{} в терминале · ждёт", live.len())
        }
    } else if let Some(last) = sess.first() {
        format!("закрыт · последняя {}", ago(last.mtime))
    } else if a.id == "opencode" && opencode_unreadable() {
        // база есть, но прочесть её не вышло (повреждена, занята, схема незнакомая) — не «сессий нет»
        "не смогла прочитать".into()
    } else {
        "сессий нет".into()
    };
    let hint = match sess.first() {
        Some(x) => format!("Продолжить последнюю: {}", resume_command(x)),
        None => String::new(),
    };
    out.push(Check::new("ai", &format!("{}: сессии", a.name), INFO, &value).hint(&hint));
    out
}

pub(crate) fn check_codex() -> Vec<Check> {
    find("codex").map(check).unwrap_or_default()
}
pub(crate) fn check_qwen() -> Vec<Check> {
    find("qwen").map(check).unwrap_or_default()
}
pub(crate) fn check_kimi() -> Vec<Check> {
    find("kimi").map(check).unwrap_or_default()
}
pub(crate) fn check_gemini() -> Vec<Check> {
    find("gemini").map(check).unwrap_or_default()
}
pub(crate) fn check_cursor() -> Vec<Check> {
    find("cursor").map(check).unwrap_or_default()
}
pub(crate) fn check_opencode() -> Vec<Check> {
    find("opencode").map(check).unwrap_or_default()
}
pub(crate) fn check_agy() -> Vec<Check> {
    find("agy").map(check).unwrap_or_default()
}

/// Все сессии всех установленных агентов, свежие сверху (для списка «Сессии»).
pub fn all_sessions(limit: usize) -> Vec<AgentSession> {
    let mut v: Vec<AgentSession> = AGENTS.iter().filter(|a| a.data_dir().is_some()).flat_map(|a| a.list_sessions(limit)).collect();
    v.sort_by(|a, b| b.live.cmp(&a.live).then(b.mtime.partial_cmp(&a.mtime).unwrap_or(std::cmp::Ordering::Equal)));
    v.truncate(limit);
    v
}

#[cfg(test)]
mod tests {
    #[test]
    fn ps_quotes() {
        assert_eq!(super::ps_quote("C:\\a b"), "'C:\\a b'");
        assert_eq!(super::ps_quote("a'b"), "'a''b'");
        assert_eq!(super::ps_quote("a\u{2019}; calc; \u{2019}"), "'a\u{2019}\u{2019}; calc; \u{2019}\u{2019}'");
    }

    #[test]
    fn ids_are_safe() {
        use super::safe_id;
        assert!(safe_id("01a191ba-8ebb-7c30-94ba-31e001dbb110").is_some());
        assert!(safe_id("session_439c.1").is_some());
        for bad in ["x; rm -rf ~", "$(id)", "a`b`", "", "-rf", "a b", "x'y"] {
            assert!(safe_id(bad).is_none(), "{bad}");
        }
    }

    use super::*;

    fn fx(p: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agents").join(p)
    }

    #[test]
    fn codex_reads_real_session() {
        let v = codex_sessions(&fx(".codex"));
        assert_eq!(v.len(), 1);
        let x = &v[0];
        assert_eq!(x.id, "01a191ba-8ebb-7c30-94ba-31e001dbb110");
        assert_eq!(x.cwd, "/home/ubuntu/proj-a");
        assert_eq!(x.title, "Reply with exactly: hi");
        assert_eq!(x.busy, Some(false));
        // папки proj-a нет — команда без cd
        assert_eq!(resume_command(x), "codex resume 01a191ba-8ebb-7c30-94ba-31e001dbb110");
    }

    #[test]
    fn qwen_reads_real_sessions() {
        let v = qwen_sessions(&fx(".qwen"));
        assert_eq!(v.len(), 3);
        assert!(v.iter().all(|x| x.cwd == "/home/ubuntu/proj-b" && x.title == "Reply with exactly: hi"));
        // две удачные сессии закончились ответом; первая упала с ошибкой API
        assert!(v.iter().filter(|x| x.busy == Some(false)).count() >= 2);
    }

    #[test]
    fn capped_lines() {
        // строка в 5 МБ без перевода строки: в буфере не больше лимита, следующая строка читается целой
        let mut data = vec![b'x'; 5 << 20];
        data.extend_from_slice(b"\n{\"a\":1}\n");
        let mut rd = std::io::BufReader::new(std::io::Cursor::new(data));
        let mut buf = Vec::new();
        assert_eq!(super::read_line_capped(&mut rd, &mut buf, 1024), (5 << 20) + 1);
        assert_eq!(buf.len(), 1024);
        assert_eq!(super::read_line_capped(&mut rd, &mut buf, 1024), 8);
        assert_eq!(buf, b"{\"a\":1}\n");
        assert_eq!(super::read_line_capped(&mut rd, &mut buf, 1024), 0);
    }

    #[test]
    fn cursor_layout() {
        // HOME общий на процесс: другой тест (diag::snapshot_roundtrip) может подменять его параллельно
        let _env = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // как у cursor-agent: chats/<md5(папка)>/<id>/store.db
        let tmp = std::env::temp_dir().join(format!("aptechka-cursor-{}", std::process::id()));
        let h = home().to_string_lossy().to_string();
        let chat = tmp.join(md5_hex(&h)).join("chat-123");
        std::fs::create_dir_all(&chat).unwrap();
        std::fs::write(chat.join("store.db"), b"x").unwrap();
        let v = cursor_sessions(&tmp);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "chat-123");
        assert_eq!(v[0].cwd, h);
        assert!(resume_command(&v[0]).ends_with("cursor-agent --resume chat-123"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn kimi_reads_real_session() {
        // в индексе абсолютные пути из ВМ — подменим на папку с образцами
        let tmp = std::env::temp_dir().join(format!("aptechka-kimi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let src = fx(".kimi-code");
        let idx = crate::store::read_text(src.join("session_index.jsonl")).unwrap();
        std::fs::write(tmp.join("session_index.jsonl"), idx.replace("/home/ubuntu/.kimi-code", &src.to_str().unwrap().replace('\\', "\\\\")))
            .unwrap();
        let v = kimi_sessions(&tmp);
        assert_eq!(v.len(), 1);
        let x = &v[0];
        assert_eq!(x.id, "session_439c1b35-399f-491c-9b95-91bf6d56b68b");
        assert_eq!(x.cwd, "/home/ubuntu/proj-c");
        assert_eq!(x.title, "Reply with exactly: hi");
        assert_eq!(x.busy, Some(false));
        assert_eq!(resume_command(x), "kimi -r session_439c1b35-399f-491c-9b95-91bf6d56b68b");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    static OC_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn oc_tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aptechka-oc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn opencode_reads_db() {
        let _g = OC_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let d = oc_tmp("ok");
        // ключ собираем на лету: в исходниках его нет
        let key = format!("sk-ant-api03-{}", "aB3dE5fG7hJ9kL1mN3pQ5rS7tU9vW1xY3zA5bC7d");
        {
            let c = rusqlite::Connection::open(d.join("opencode.db")).unwrap();
            c.execute_batch(
                "CREATE TABLE session_v2 (id text PRIMARY KEY, parent_id text, directory text NOT NULL, title text, time_updated integer NOT NULL, time_archived integer);",
            )
            .unwrap();
            let mut ins = c.prepare("INSERT INTO session_v2 VALUES (?1,?2,?3,?4,?5,?6)").unwrap();
            ins.execute(rusqlite::params!["ses_old", None::<String>, "/tmp/a", "старая", 1_700_000_000_000i64, None::<i64>]).unwrap();
            ins.execute(rusqlite::params!["ses_new", None::<String>, "/tmp/b", format!("ключ {key} тут"), 1_800_000_000_000i64, None::<i64>])
                .unwrap();
            ins.execute(rusqlite::params!["ses_sub", Some("ses_new"), "/tmp/b", "помощник", 1_900_000_000_000i64, None::<i64>]).unwrap();
            ins.execute(rusqlite::params!["ses_arch", None::<String>, "/tmp/b", "в архиве", 1_900_000_000_000i64, Some(5i64)]).unwrap();
            // кривые строки: нет номера, время текстом, заголовок числом
            ins.execute(rusqlite::params!["", None::<String>, "/x", "без номера", 1i64, None::<i64>]).unwrap();
            ins.execute(rusqlite::params!["ses_bad", None::<String>, "/x", 42i64, "вчера", None::<i64>]).unwrap();
            ins.execute(rusqlite::params!["x; rm -rf ~", None::<String>, "/x", "злой номер", 1_600_000_000_000i64, None::<i64>]).unwrap();
        }
        let v = opencode_read(&d).unwrap();
        let ids: Vec<&str> = v.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["ses_new", "ses_old", "x; rm -rf ~", "ses_bad"]);
        assert_eq!(v[0].cwd, "/tmp/b");
        assert_eq!(v[0].mtime, 1_800_000_000.0);
        assert!(!v[0].title.contains(&key), "{}", v[0].title);
        assert!(v[0].title.starts_with("ключ "));
        assert_eq!(v[3].title, "42"); // число в текстовом столбце SQLite хранит строкой
        assert_eq!(v[3].mtime, 0.0);
        assert!(resume_command(&v[0]).ends_with("opencode -s ses_new"));
        assert_eq!(resume_command(&v[2]), "", "небезопасный номер не даёт команду");
        assert!(!opencode_unreadable());
        // кэш по времени и размеру: тот же ответ без чтения
        assert_eq!(opencode_read(&d).unwrap().len(), 4);
        // временных копий не осталось
        let left = std::fs::read_dir(crate::store::private_dir())
            .map(|r| r.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&format!("oc_{}_", std::process::id()))).count())
            .unwrap_or(0);
        assert_eq!(left, 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn opencode_old_table_and_unknown_schema() {
        let _g = OC_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let d = oc_tmp("old");
        {
            let c = rusqlite::Connection::open(d.join("opencode.db")).unwrap();
            c.execute_batch("CREATE TABLE session (id text, title text, directory text, time_updated integer); INSERT INTO session VALUES ('s1','t','/a',1700000000000);")
                .unwrap();
        }
        let v = opencode_read(&d).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "s1");
        let _ = std::fs::remove_dir_all(&d);

        let d = oc_tmp("schema");
        {
            let c = rusqlite::Connection::open(d.join("opencode.db")).unwrap();
            c.execute_batch("CREATE TABLE session (foo text);").unwrap();
        }
        assert!(opencode_read_db(&d.join("opencode.db")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn opencode_garbage_or_locked_is_error_not_panic() {
        let _g = OC_TEST.lock().unwrap_or_else(|e| e.into_inner());
        let d = oc_tmp("garbage");
        std::fs::write(d.join("opencode.db"), vec![0xA5u8; 8192]).unwrap();
        assert!(opencode_read_db(&d.join("opencode.db")).is_err());
        assert!(opencode_read(&d).is_err());
        assert!(opencode_unreadable());
        assert!(opencode_sessions(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);

        // база занята: другой процесс держит исключительную блокировку — копия всё равно читается
        let d = oc_tmp("locked");
        let c = rusqlite::Connection::open(d.join("opencode.db")).unwrap();
        c.execute_batch("CREATE TABLE session (id text, title text, directory text, time_updated integer); INSERT INTO session VALUES ('s1','t','/a',1700000000000); BEGIN EXCLUSIVE;")
            .unwrap();
        let r = opencode_read_db(&d.join("opencode.db"));
        assert!(r.is_ok() || r.is_err()); // главное — без паники и зависания
        drop(c);
        // ссылка вместо базы — отказ
        #[cfg(unix)]
        {
            let t = oc_tmp("link");
            std::os::unix::fs::symlink(d.join("opencode.db"), t.join("opencode.db")).unwrap();
            assert!(opencode_read_db(&t.join("opencode.db")).is_err());
            let _ = std::fs::remove_dir_all(&t);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
