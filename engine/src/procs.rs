//! Список запущенных процессов текущего пользователя: pid, имя, командная строка, рабочая папка.
//! Linux — из /proc (без внешних команд), macOS — ps и lsof, Windows — Toolhelp32 и вызовы системы.

#[derive(Clone, Debug, Default)]
pub struct Proc {
    pub pid: i64,
    /// Имя программы (comm на Linux, имя файла программы на macOS и Windows).
    pub name: String,
    /// Аргументы командной строки.
    pub args: Vec<String>,
    /// Рабочая папка ("" — не удалось узнать; на Windows не узнаём).
    pub cwd: String,
    /// Когда запущен, секунды Unix (0 — не удалось узнать).
    pub start: f64,
    /// Родитель (0 — неизвестен).
    pub ppid: i64,
    /// Есть ли у процесса терминал (запущен в окне терминала, а не фоновой службой). None — не узнать (Windows).
    pub tty: Option<bool>,
}

impl Proc {
    /// Похоже ли на программу `bin`: по имени или по одному из первых аргументов
    /// (node-обёртки: `node /…/bin/qwen`, `node …/@anthropic-ai/claude-code/cli.js`; python: `python …/kimi`).
    /// Это запущенная копия Аптечки с таким аргументом (сама программа, а не `timeout aptechka …`,
    /// `gdb aptechka` или редактор с файлом «aptechka»): сравниваем имя программы процесса с нашим.
    pub fn is_our(&self, arg: &str) -> bool {
        let base = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or(s).trim_end_matches(".exe").to_lowercase();
        let ours = self_exe().map(|p| base(&p.to_string_lossy())).unwrap_or_else(|| "aptechka".into());
        // на Linux имя процесса обрезано до 15 символов
        let name = base(&self.name);
        (name == ours || ours.starts_with(&name) && name.len() == 15) && self.args.iter().skip(1).any(|a| a == arg)
    }

    pub fn is(&self, bin: &str) -> bool {
        let base = |s: &str| -> String {
            let b = s.rsplit(['/', '\\']).next().unwrap_or(s).to_string();
            let b = b.strip_suffix(".exe").or_else(|| b.strip_suffix(".cmd")).or_else(|| b.strip_suffix(".js")).unwrap_or(&b).to_string();
            b.to_lowercase()
        };
        // сама программа: имя процесса или файл argv[0] (а не любой аргумент: «vim /tmp/claude» — не Claude)
        if base(&self.name) == bin || self.args.first().is_some_and(|a| base(a) == bin) {
            return true;
        }
        // программа на node/bun/python: её скрипт — первый не-ключ после запускальщика; агент, если скрипт —
        // файл с именем агента (…/bin/qwen) или лежит в установленном пакете агента (node_modules/@…/claude-code)
        const RUNNERS: [&str; 5] = ["node", "bun", "deno", "python", "python3"];
        let runner = RUNNERS.contains(&base(&self.name).as_str()) || self.args.first().is_some_and(|a| RUNNERS.contains(&base(a).as_str()));
        if !runner {
            return false;
        }
        let Some(script) = self.args.iter().skip(1).find(|a| !a.starts_with('-')) else { return false };
        if base(script) == bin {
            return true;
        }
        let low = script.to_lowercase();
        let in_package = low.contains("node_modules") || low.contains("site-packages") || low.contains("/.npm/") || low.contains("\\npm\\");
        in_package && low.split(['/', '\\']).any(|part| part == bin || part.starts_with(&format!("{bin}-")))
    }
}

/// Тиков в секунду для времён процессов в /proc (почти везде 100, но спрашиваем у системы).
pub fn clk_tck() -> f64 {
    #[cfg(target_os = "linux")]
    {
        extern "C" {
            fn sysconf(name: i32) -> i64;
        }
        // _SC_CLK_TCK = 2
        let t = unsafe { sysconf(2) };
        if t > 0 {
            return t as f64;
        }
    }
    100.0
}

/// Все процессы прямо сейчас, без памяти и без исключений (в том числе запущенные самой Аптечкой).
pub fn list_fresh() -> Vec<Proc> {
    list_now()
}

/// Список процессов. Он нужен часто (сессии агентов — раз в 4 с, для каждого агента), а на macOS
/// и Windows это запуск внешних команд, поэтому свежий список держим недолго в памяти.
pub fn list() -> Vec<Proc> {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, Vec<Proc>)>> = Mutex::new(None);
    let ttl = Duration::from_millis(if cfg!(target_os = "linux") { 1_000 } else { 3_000 });
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((t, v)) = c.as_ref() {
        if t.elapsed() < ttl {
            return v.clone();
        }
    }
    // свои дочерние процессы (Аптечка сама запускает, например, `opencode session list`) — не программы пользователя
    let me = std::process::id() as i64;
    let v: Vec<Proc> = list_now().into_iter().filter(|p| p.ppid != me).collect();
    *c = Some((Instant::now(), v.clone()));
    v
}

#[cfg(target_os = "macos")]
fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(target_os = "linux")]
fn list_now() -> Vec<Proc> {
    let me = std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m)).ok();
    // время загрузки системы: от него считается время запуска процессов (в тиках, 100 в секунду)
    let btime: f64 = std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("btime ").and_then(|v| v.trim().parse().ok())))
        .unwrap_or(0.0);
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i64>().ok()) else { continue };
        let dir = e.path();
        // только свои процессы: чужие нам всё равно не прочитать и не трогать
        let uid = std::fs::metadata(&dir).map(|m| std::os::unix::fs::MetadataExt::uid(&m)).ok();
        if uid != me {
            continue;
        }
        let name = std::fs::read_to_string(dir.join("comm")).unwrap_or_default().trim().to_string();
        let args: Vec<String> = std::fs::read(dir.join("cmdline"))
            .unwrap_or_default()
            .split(|b| *b == 0)
            .filter(|a| !a.is_empty())
            .map(|a| String::from_utf8_lossy(a).to_string())
            .collect();
        let cwd = std::fs::read_link(dir.join("cwd")).map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
        // stat: после имени в скобках — поля с 3-го; starttime — 22-е
        let stat = std::fs::read_to_string(dir.join("stat")).unwrap_or_default();
        let fields: Vec<&str> = stat.rsplit_once(')').map(|(_, rest)| rest.split_whitespace().collect()).unwrap_or_default();
        let ppid = fields.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
        let start = fields.get(19).and_then(|v| v.parse::<f64>().ok()).map_or(0.0, |ticks| if btime > 0.0 { btime + ticks / clk_tck() } else { 0.0 });
        // управляющий терминал — 5-е поле после имени (tty_nr): 0 — терминала нет (служба, фоновый сервер)
        let tty = fields.get(4).and_then(|v| v.parse::<i64>().ok()).map(|t| t != 0);
        out.push(Proc { pid, name, args, cwd, start, ppid, tty });
    }
    out
}

/// Windows: снимок процессов (Toolhelp32) — имя, pid, родитель; командная строка и время запуска —
/// у самого процесса (NtQueryInformationProcess, GetProcessTimes). Без PowerShell: тот стоит ~1 с процессора.
/// Папку процесса Windows так не отдаёт.
#[cfg(windows)]
fn list_now() -> Vec<Proc> {
    use core::ffi::c_void;
    type Handle = *mut c_void;
    #[repr(C)]
    struct Entry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        ppid: u32,
        prio: i32,
        flags: u32,
        exe: [u16; 260],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Process32FirstW(snap: Handle, e: *mut Entry) -> i32;
        fn Process32NextW(snap: Handle, e: *mut Entry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetProcessTimes(h: Handle, c: *mut u64, e: *mut u64, k: *mut u64, u: *mut u64) -> i32;
        fn CloseHandle(h: Handle) -> i32;
        fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
        fn GetCurrentProcessId() -> u32;
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationProcess(h: Handle, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    }
    const SNAPPROCESS: u32 = 2;
    const QUERY_LIMITED: u32 = 0x1000;
    const CMDLINE_INFO: u32 = 60; // ProcessCommandLineInformation, Windows 8.1+
    let invalid = usize::MAX as Handle;
    let mut out = Vec::new();
    unsafe {
        let mut my_session = u32::MAX - 1;
        ProcessIdToSessionId(GetCurrentProcessId(), &mut my_session);
        let snap = CreateToolhelp32Snapshot(SNAPPROCESS, 0);
        if snap == invalid || snap.is_null() {
            return out;
        }
        let mut e: Entry = std::mem::zeroed();
        e.size = std::mem::size_of::<Entry>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let n = e.exe.iter().position(|&c| c == 0).unwrap_or(e.exe.len());
            let name = String::from_utf16_lossy(&e.exe[..n]);
            let (mut args, mut start) = (Vec::new(), 0.0);
            let h = OpenProcess(QUERY_LIMITED, 0, e.pid);
            if !h.is_null() {
                // командная строка: UNICODE_STRING {длина, макс., указатель} и сам текст следом
                let mut need = 0u32;
                NtQueryInformationProcess(h, CMDLINE_INFO, std::ptr::null_mut(), 0, &mut need);
                if need > 0 && need < 1 << 20 {
                    let mut buf = vec![0u64; (need as usize).div_ceil(8)];
                    if NtQueryInformationProcess(h, CMDLINE_INFO, buf.as_mut_ptr() as *mut c_void, need, &mut need) >= 0 {
                        // UNICODE_STRING описан типами этой платформы (не смещением «8», верным только для 64 бит)
                        #[repr(C)]
                        struct UnicodeString {
                            length: u16,
                            max: u16,
                            buffer: *const u16,
                        }
                        let us = &*(buf.as_ptr() as *const UnicodeString);
                        let len = us.length as usize / 2;
                        let ptr = us.buffer;
                        // текст должен лежать внутри полученного буфера — иначе не читаем
                        let lo = buf.as_ptr() as usize;
                        let hi = lo + buf.len() * 8;
                        let inside = (ptr as usize) >= lo && (ptr as usize).saturating_add(len * 2) <= hi;
                        if !ptr.is_null() && len > 0 && inside && us.max as usize >= us.length as usize {
                            let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                            args = split_cmdline(&text);
                        }
                    }
                }
                let (mut c, mut x, mut k, mut u) = (0u64, 0u64, 0u64, 0u64);
                if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u) != 0 && c > 0 {
                    start = c as f64 / 1e7 - 11_644_473_600.0; // FILETIME (с 1601 г., по 100 нс) → Unix
                }
                CloseHandle(h);
            }
            // только процессы своего сеанса входа (как на Linux и macOS — только свои), а не всех пользователей
            let mut sess = u32::MAX;
            if ProcessIdToSessionId(e.pid, &mut sess) != 0 && sess == my_session {
                out.push(Proc { pid: e.pid as i64, name, args, cwd: String::new(), start, ppid: e.ppid as i64, tty: None });
            }
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

/// Командная строка Windows → аргументы (кавычки "…" держат пробелы).
#[cfg(windows)]
fn split_cmdline(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut q) = (Vec::new(), String::new(), false);
    for ch in s.chars() {
        match ch {
            '"' => q = !q,
            c if c.is_whitespace() && !q => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// macOS: ps (pid, сколько работает, программа, аргументы) и lsof (рабочие папки) — оба быстрые.
#[cfg(target_os = "macos")]
fn list_now() -> Vec<Proc> {
    use std::collections::HashMap;
    let (_, uid) = crate::diag::run(&["id", "-u"], 5);
    let uid = uid.trim().to_string();
    let (_, comm) = crate::diag::run(&["ps", "-U", &uid, "-o", "pid=,ppid=,etime=,comm="], 10);
    let (_, args) = crate::diag::run(&["ps", "-U", &uid, "-o", "pid=,args="], 10);
    // терминал процесса: «??» — нет (фоновая служба)
    let (_, ttys) = crate::diag::run(&["ps", "-U", &uid, "-o", "pid=,tty="], 10);
    let ttymap: HashMap<i64, bool> = ttys
        .lines()
        .filter_map(|l| {
            let (pid, t) = l.trim().split_once(' ')?;
            Some((pid.parse().ok()?, !matches!(t.trim(), "??" | "-" | "")))
        })
        .collect();
    let mut argmap: HashMap<i64, Vec<String>> = HashMap::new();
    for l in args.lines() {
        let l = l.trim();
        if let Some((pid, rest)) = l.split_once(' ') {
            if let Ok(pid) = pid.parse() {
                argmap.insert(pid, rest.split_whitespace().map(str::to_string).collect());
            }
        }
    }
    // lsof -Fpn: строки «p<pid>» и «n<папка>»
    let (_, lsof) = crate::diag::run(&["lsof", "-a", "-d", "cwd", "-u", &uid, "-Fpn", "-w"], 10);
    let mut cwds: HashMap<i64, String> = HashMap::new();
    let mut cur = -1;
    for l in lsof.lines() {
        if let Some(p) = l.strip_prefix('p') {
            cur = p.parse().unwrap_or(-1);
        } else if let Some(n) = l.strip_prefix('n') {
            cwds.insert(cur, n.to_string());
        }
    }
    let now = now_secs();
    comm.lines()
        .filter_map(|l| {
            let (pid, rest) = l.trim().split_once(' ')?;
            let pid: i64 = pid.parse().ok()?;
            let (ppid, rest) = rest.trim_start().split_once(' ')?;
            let (etime, path) = rest.trim_start().split_once(' ')?;
            let name = path.trim().rsplit('/').next()?.to_string();
            Some(Proc {
                pid,
                name,
                args: argmap.remove(&pid).unwrap_or_else(|| vec![path.trim().to_string()]),
                cwd: cwds.remove(&pid).unwrap_or_default(),
                start: etime_secs(etime).map_or(0.0, |e| now - e),
                ppid: ppid.parse().unwrap_or(0),
                tty: ttymap.get(&pid).copied(),
            })
        })
        .collect()
}

/// ps etime: [[дд-]чч:]мм:сс → секунды.
#[cfg(any(target_os = "macos", test))]
fn etime_secs(s: &str) -> Option<f64> {
    let (days, hms) = match s.split_once('-') {
        Some((d, r)) => (d.parse::<f64>().ok()?, r),
        None => (0.0, s),
    };
    let mut secs = 0.0;
    for part in hms.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(days * 86400.0 + secs)
}

/// Файл самой Аптечки. На Linux, если программу обновили, пока она работала, путь к старому файлу
/// выглядит как «… (deleted)» — запускать нужно новый файл по тому же пути.
pub fn self_exe() -> Option<std::path::PathBuf> {
    let p = std::env::current_exe().ok()?;
    let s = p.to_string_lossy();
    Some(std::path::PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s)))
}

/// Запустить программу и не ждать её; когда она закончится, забрать её код в фоне,
/// чтобы не оставался «зомби»-процесс.
pub fn spawn_detached(cmd: &mut std::process::Command) -> bool {
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
    }
}

/// Время изменения файла самой Аптечки (обновили ли её, пока она работает).
pub fn self_stamp() -> Option<std::time::SystemTime> {
    std::fs::metadata(self_exe()?).and_then(|m| m.modified()).ok()
}

/// Из процессов одной программы оставить «настоящие»: обёртка (node-скрипт, script, npx),
/// запустившая такой же процесс, — это тот же агент, второй раз его не считаем.
pub fn without_wrappers(v: Vec<Proc>) -> Vec<Proc> {
    let parents: Vec<i64> = v.iter().map(|p| p.ppid).collect();
    v.into_iter().filter(|p| !parents.contains(&p.pid)).collect()
}

/// Кто грузит процессор — чтобы не пугать зря.
#[derive(Debug, PartialEq)]
pub enum Hog {
    /// виртуальная машина (её нагрузка — то, что запущено внутри)
    Vm,
    /// запущено ИИ-помощником (имя помощника)
    Agent(&'static str),
    /// сборка, скрипт, компилятор
    Build,
    Other,
}

const VMS: &[&str] = &[
    "qemu-system",
    "qemu-kvm",
    "VBoxHeadless",
    "VirtualBoxVM",
    "VirtualBox",
    "vmware-vmx",
    "vmware",
    "prl_vm_app",
    "UTM",
    "Virtualization",
    "com.apple.Virtualization",
    "multipassd",
    "hyperkit",
    "vmmem",
    "wsl",
    "WslService",
];
const AGENTS: &[(&str, &str)] = &[
    ("claude", "Claude"),
    ("codex", "Codex"),
    ("qwen", "Qwen Code"),
    ("kimi", "Kimi"),
    ("gemini", "Gemini CLI"),
    ("agy", "Antigravity"),
    ("cursor-agent", "Cursor"),
    ("opencode", "OpenCode"),
];
const BUILDERS: &[&str] = &[
    "python", "python3", "node", "bun", "deno", "cargo", "rustc", "gcc", "g++", "cc1", "cc1plus", "clang", "ld", "make", "cmake", "ninja", "java",
    "go", "dotnet", "msbuild", "gradle", "npm", "pip", "ffmpeg",
];

/// Разобрать, что за программа грузит процессор: сама она, её родители (до 20 поколений) — ИИ-помощник?
pub fn hog_kind(p: &Proc, all: &[Proc]) -> Hog {
    let base = p.name.trim_end_matches(".exe").to_string();
    if VMS.iter().any(|v| base.starts_with(v)) {
        return Hog::Vm;
    }
    let mut cur = p.ppid;
    for _ in 0..20 {
        let Some(par) = all.iter().find(|x| x.pid == cur) else { break };
        if let Some((_, name)) = AGENTS.iter().find(|(bin, _)| par.is(bin)) {
            return Hog::Agent(name);
        }
        if cur == par.ppid {
            break;
        }
        cur = par.ppid;
    }
    if BUILDERS.iter().any(|b| base == *b || base.starts_with(&format!("{b}3")) || base.starts_with("python3.")) {
        return Hog::Build;
    }
    Hog::Other
}

/// Жив ли процесс с таким номером.
pub fn alive(pid: i64) -> bool {
    if pid <= 0 || pid > i32::MAX as i64 {
        return false;
    }
    alive_os(pid)
}

#[cfg(target_os = "linux")]
fn alive_os(pid: i64) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn alive_os(pid: i64) -> bool {
    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // сигнал 0 ничего не делает, только проверяет; «нет прав» (EPERM) — значит, процесс есть
    (unsafe { kill(pid as i32, 0) == 0 }) || std::io::Error::last_os_error().raw_os_error() == Some(1)
}

#[cfg(windows)]
fn alive_os(pid: i64) -> bool {
    type Handle = *mut core::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn GetExitCodeProcess(h: Handle, code: *mut u32) -> i32;
        fn CloseHandle(h: Handle) -> i32;
    }
    const QUERY_LIMITED: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let h = OpenProcess(QUERY_LIMITED, 0, pid as u32);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
        CloseHandle(h);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, args: &[&str]) -> Proc {
        Proc { name: name.into(), args: args.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn recognizes_agents() {
        assert!(p("claude", &["claude", "--resume", "x"]).is("claude"));
        assert!(p("node", &["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"]).is("claude"));
        assert!(p(
            "node.exe",
            &["C:\\Program Files\\nodejs\\node.exe", "C:\\Users\\u\\AppData\\Roaming\\npm\\node_modules\\@google\\gemini-cli\\dist\\index.js"]
        )
        .is("gemini"));
        assert!(p("claude.exe", &["C:\\Users\\u\\.local\\bin\\claude.exe"]).is("claude"));
        assert!(p("node", &["node", "/home/u/.npm/bin/qwen"]).is("qwen"));
        // папка проекта с похожим именем у обычной программы — не агент
        assert!(!p("vim", &["vim", "/home/u/claude-notes/a.txt"]).is("claude"));
        assert!(!p("node", &["node", "/home/u/proj/server.js"]).is("claude"));
        // имя в аргументе обычной программы или в папке проекта — не агент
        assert!(!p("vim", &["vim", "/tmp/claude"]).is("claude"));
        assert!(!p("node", &["node", "/tmp/claude-notes/server.js"]).is("claude"));
    }

    #[test]
    fn hogs_explained() {
        let mk = |pid, ppid, name: &str, args: &[&str]| Proc {
            pid,
            ppid,
            name: name.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let all = vec![
            mk(1, 0, "systemd", &[]),
            mk(10, 1, "bash", &["bash"]),
            mk(11, 10, "claude", &["claude"]),
            mk(12, 11, "bash", &["bash", "-c", "python3 x.py"]),
            mk(13, 12, "python3", &["python3", "x.py"]),
            mk(20, 1, "qemu-system-x86", &["qemu-system-x86_64"]),
            mk(30, 10, "python3", &["python3", "y.py"]),
            mk(40, 1, "firefox", &["firefox"]),
        ];
        let f = |pid| hog_kind(all.iter().find(|p| p.pid == pid).unwrap(), &all);
        assert_eq!(f(13), Hog::Agent("Claude"));
        assert_eq!(f(20), Hog::Vm);
        assert_eq!(f(30), Hog::Build);
        assert_eq!(f(40), Hog::Other);
    }

    #[test]
    fn wrappers_counted_once() {
        let mk = |pid, ppid| Proc { pid, ppid, ..Default::default() };
        let v = without_wrappers(vec![mk(10, 1), mk(11, 10), mk(20, 1)]);
        assert_eq!(v.iter().map(|p| p.pid).collect::<Vec<_>>(), vec![11, 20]);
    }

    #[test]
    fn etime() {
        assert_eq!(etime_secs("05:07"), Some(307.0));
        assert_eq!(etime_secs("1:02:03"), Some(3723.0));
        assert_eq!(etime_secs("2-01:00:00"), Some(176400.0));
    }

    #[test]
    fn self_alive() {
        assert!(alive(std::process::id() as i64));
        assert!(!alive(-1));
    }
}

/// Найти программу в PATH (с расширениями Windows), вернуть полный путь.
pub fn which(bin: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat", ""] } else { &[""] };
    // только абсолютные папки: пустой элемент PATH или «.» — это текущая папка, программу оттуда не берём
    for dir in std::env::split_paths(&path).filter(|d| d.is_absolute()) {
        for ext in exts {
            let p = dir.join(format!("{bin}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Процессы, без которых рабочий стол или система ломаются: «Закрыть программу» им не предлагаем и не выполняем
/// (даже если они заняли процессор — тогда беда не в них, а в том, что их нагружает).
pub fn is_protected(name: &str) -> bool {
    const P: &[&str] = &[
        "gnome-shell",
        "Xorg",
        "Xwayland",
        "systemd",
        "init",
        "dbus-daemon",
        "dbus-broker",
        "pipewire",
        "wireplumber",
        "pulseaudio",
        "gdm",
        "gdm-session-worker",
        "sddm",
        "lightdm",
        "login",
        "sshd",
        "kwin_x11",
        "kwin_wayland",
        "plasmashell",
        "mutter",
        "gnome-session-binary",
        "NetworkManager",
        "polkitd",
        "xdg-desktop-portal",
        "explorer.exe",
        "dwm.exe",
        "csrss.exe",
        "winlogon.exe",
        "lsass.exe",
        "services.exe",
        "svchost.exe",
        "WindowServer",
        "loginwindow",
        "Dock",
        "Finder",
        "SystemUIServer",
        "launchd",
        "aptechka",
        "aptechka.exe",
        "systemd-logind",
        "gnome-keyring-daemon",
        "Xvfb",
        "wininit.exe",
        "smss.exe",
        "sihost.exe",
        "audiodg.exe",
        "MsMpEng.exe",
        "ctfmon.exe",
        "fontdrvhost.exe",
        "coreaudiod",
        "kernel_task",
    ];
    // путь → имя файла; на Linux ядро обрезает имя (comm) до 15 символов: «gnome-session-b» = «gnome-session-binary»
    let n = name.trim().rsplit(['/', '\\']).next().unwrap_or("");
    if n.is_empty() {
        return false;
    }
    let lower = n.to_lowercase();
    P.iter().any(|p| {
        let p = p.to_lowercase();
        p == lower || (n.len() == 15 && p.starts_with(&lower))
    })
}

#[cfg(test)]
#[test]
fn protected_names() {
    assert!(is_protected("gnome-session-b")); // обрезано ядром до 15 символов
    assert!(is_protected("xdg-desktop-por"));
    assert!(is_protected("/System/Library/CoreServices/Dock.app/Contents/MacOS/Dock"));
    assert!(is_protected("EXPLORER.EXE"));
    assert!(!is_protected("gnome-session")); // короче 15 — сравниваем целиком
    assert!(!is_protected("firefox"));
    assert!(!is_protected(""));
}

/// Время запуска процесса (Linux: поле 22 /proc/PID/stat, в тиках) — чтобы отличить его от нового процесса,
/// получившего тот же номер после закрытия старого.
pub fn start_ticks(pid: i64) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // имя в скобках может содержать пробелы — считаем поля после последней «)»
    s.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}
