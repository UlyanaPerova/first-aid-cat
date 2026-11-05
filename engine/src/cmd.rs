//! Запуск внешних команд — один слой для проверок (diag::run) и починок (fixes::sh).
//!
//! Различает исходы: команда ответила / завершилась с ошибкой / такой программы нет / не ответила вовремя.
//! Раньше текст ошибки запуска («No such file or directory») подсовывался вместо вывода команды, и проверки
//! делали из него вывод («часовой пояс: No such file…», «упавшие службы: No»).
//!
//! Таймаут: команда запускается своей группой процессов, по таймауту останавливается вся группа (и её дети).
//! Если остановить нельзя — pkexec/sudo работают с правами администратора и обычной программе не подчиняются, —
//! перестаём ждать (раньше wait() висел до конца команды, хотя писали «я её остановила») и говорим об этом.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// код 0; вывод
    Ok(String),
    /// ненулевой код; вывод (stdout, а со stderr — если просили)
    Failed(i32, String),
    /// такой программы нет
    Missing,
    /// не ответила вовремя; stopped — удалось ли её остановить
    Timeout { stopped: bool },
    /// не запустилась по другой причине
    SpawnError(String),
}

impl Outcome {
    /// Вывод, только если команда отработала (успешно или с кодом) — иначе пусто, а не текст ошибки.
    pub fn text(&self) -> &str {
        match self {
            Outcome::Ok(s) | Outcome::Failed(_, s) => s,
            _ => "",
        }
    }
    /// Код в старом виде: 0 — успех, 127 — нет программы / не запустилась, 124 — не ответила.
    pub fn code(&self) -> i32 {
        match self {
            Outcome::Ok(_) => 0,
            Outcome::Failed(c, _) => *c,
            Outcome::Missing | Outcome::SpawnError(_) => 127,
            Outcome::Timeout { .. } => 124,
        }
    }
    /// Программа есть и ответила (с любым кодом): по её выводу можно что-то утверждать.
    pub fn answered(&self) -> bool {
        matches!(self, Outcome::Ok(_) | Outcome::Failed(..))
    }
}

/// Настройки запуска.
#[derive(Clone, Copy)]
pub struct Opts {
    /// LC_ALL=C — ответы по-английски (разбираем текст; на русской системе слова другие)
    pub c_locale: bool,
    /// приклеить stderr к выводу (для сообщений об ошибках)
    pub stderr: bool,
}

pub const CHECK: Opts = Opts { c_locale: true, stderr: false };
pub const FIX: Opts = Opts { c_locale: true, stderr: true };

/// Единственный список переменных окружения, которые получают запускаемые программы. Всё остальное
/// (DATABASE_URL, GH_PAT, *_AUTH, пароли…) не передаётся: список разрешённых, а не запрещённых имён.
/// Прокси-переменные (http_proxy…) не передаются: ни одна наша команда не ходит в сеть через curl,
/// а в адресе прокси бывают логин и пароль.
pub fn env_allowed(name: &str) -> bool {
    let u = name.to_ascii_uppercase();
    if u.starts_with("LC_") || u.starts_with("XDG_") || u.starts_with("PROGRAMFILES") || u.starts_with("COMMONPROGRAMFILES") {
        return true;
    }
    const KEEP: &[&str] = &[
        // Linux / общие
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "SHELL",
        "LANG",
        "LANGUAGE",
        "TZ",
        "TERM",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XAUTHORITY",
        "DBUS_SESSION_BUS_ADDRESS",
        "DESKTOP_SESSION",
        // Windows
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "PROGRAMW6432",
        "ALLUSERSPROFILE",
        "PUBLIC",
        "HOMEDRIVE",
        "HOMEPATH",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "OS",
        "SYSTEMDRIVE",
        "USERNAME",
        "USERDOMAIN",
        "COMPUTERNAME",
        "PSMODULEPATH",
        // macOS
        "TMPDIR",
        "__CF_USER_TEXT_ENCODING",
    ];
    KEEP.contains(&u.as_str())
}

/// Убрать из окружения команды всё, кроме разрешённого (`env_allowed`).
pub fn scrub_env(c: &mut Command) {
    c.env_clear();
    for (k, v) in std::env::vars_os() {
        if k.to_str().is_some_and(env_allowed) {
            c.env(k, v);
        }
    }
}

pub fn exec(cmd: &[&str], timeout: Duration, o: Opts) -> Outcome {
    exec_env(cmd, &[], timeout, o)
}

/// То же с переменными окружения (так передают данные, которые нельзя вклеивать в текст команды).
pub fn exec_env(cmd: &[&str], env: &[(&str, &str)], timeout: Duration, o: Opts) -> Outcome {
    let Some((prog, args)) = cmd.split_first() else { return Outcome::SpawnError("пустая команда".into()) };
    let mut c = Command::new(prog);
    // сначала чистка окружения, потом свои переменные (иначе чистка стёрла бы и их)
    crate::diag::quiet(&mut c);
    for (k, v) in env {
        c.env(k, v);
    }
    c.args(args).stdin(Stdio::null()).stdout(Stdio::piped());
    c.stderr(if o.stderr { Stdio::piped() } else { Stdio::null() });
    if o.c_locale {
        c.env("LC_ALL", "C").env("LANG", "C");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0); // своя группа: по таймауту остановим и её детей
    }
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Outcome::Missing,
        Err(e) => return Outcome::SpawnError(e.to_string()),
    };
    // Windows: команда и её дети — в одном «задании» (Job Object), по таймауту останавливаем всё задание,
    // а не только саму команду (раньше её дети оставались работать)
    #[cfg(windows)]
    let job = winjob::Job::attach(&child);
    // вывод читаем в своих потоках и забираем через канал с ограничением по времени: трубу может держать
    // открытой не сама команда, а оставленный ею процесс (команда уже вышла, а чтение ждало бы его конца)
    // прочитанное копится в общем буфере по мере чтения: если трубу держит оставленный процесс и конца
    // не дождаться, уже пришедший вывод не пропадает (раньше он отдавался только по концу трубы)
    type Shared = std::sync::Arc<(std::sync::Mutex<(Vec<u8>, bool)>, std::sync::Condvar)>;
    let pipe = |r: Option<Box<dyn Read + Send>>| -> Option<Shared> {
        r.map(|mut r| {
            let sh: Shared = std::sync::Arc::new((std::sync::Mutex::new((Vec::new(), false)), std::sync::Condvar::new()));
            let s2 = sh.clone();
            std::thread::spawn(move || {
                // не больше 4 МБ: команда, пишущая гигабайты, не раздует память; остаток дочитываем впустую,
                // чтобы команда не встала на полной трубе
                let mut buf = [0u8; 8192];
                loop {
                    match r.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let mut g = s2.0.lock().unwrap_or_else(|e| e.into_inner());
                            let room = (4usize << 20).saturating_sub(g.0.len());
                            g.0.extend_from_slice(&buf[..n.min(room)]);
                        }
                    }
                }
                s2.0.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
                s2.1.notify_all();
            });
            sh
        })
    };
    let out_r = pipe(child.stdout.take().map(|x| Box::new(x) as Box<dyn Read + Send>));
    let err_r = pipe(child.stderr.take().map(|x| Box::new(x) as Box<dyn Read + Send>));
    let deadline = Instant::now() + timeout;
    let code = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(-1),
            Ok(None) if Instant::now() >= deadline => {
                #[cfg(windows)]
                if let Some(j) = &job {
                    j.terminate();
                }
                let stopped = stop(&mut child);
                if stopped {
                    let _ = child.wait();
                } else {
                    // не подчинилась (права администратора): не ждём — доберёт фоновый поток
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                // читающие потоки не ждём: трубы держит команда (или её дети), пока жива
                return Outcome::Timeout { stopped };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Outcome::SpawnError(e.to_string()),
        }
    };
    // команда вышла: вывод дочитываем ещё 2 с (если трубу держит оставленный ею процесс — не ждём его)
    let until = Instant::now() + Duration::from_secs(2);
    let take = |h: Option<Shared>| -> Vec<u8> {
        let Some(h) = h else { return Vec::new() };
        let mut g = h.0.lock().unwrap_or_else(|e| e.into_inner());
        while !g.1 {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            g = h.1.wait_timeout(g, left).map(|x| x.0).unwrap_or_else(|e| e.into_inner().0);
        }
        // что успели прочитать — отдаём, даже если конца трубы не дождались
        std::mem::take(&mut g.0)
    };
    let mut s = String::from_utf8_lossy(&take(out_r)).into_owned();
    let e = take(err_r);
    if !e.is_empty() {
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&String::from_utf8_lossy(&e));
    }
    let s = s.trim().to_string();
    if code == 0 {
        Outcome::Ok(s)
    } else {
        Outcome::Failed(code, s)
    }
}

#[cfg(windows)]
mod winjob {
    use core::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> *mut c_void;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn TerminateJobObject(job: *mut c_void, code: u32) -> i32;
        fn CloseHandle(h: *mut c_void) -> i32;
    }
    pub struct Job(*mut c_void);
    // handle задания можно передавать между потоками
    unsafe impl Send for Job {}

    impl Job {
        /// Задание без «убить при закрытии»: дети, запущенные починкой нарочно (Проводник), живут дальше.
        pub fn attach(child: &std::process::Child) -> Option<Job> {
            let j = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            if j.is_null() {
                return None;
            }
            let job = Job(j);
            (unsafe { AssignProcessToJobObject(j, child.as_raw_handle()) } != 0).then_some(job)
        }
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Остановить команду по таймауту: всю её группу (unix) или сам процесс. true — остановилась.
fn stop(child: &mut std::process::Child) -> bool {
    #[cfg(unix)]
    {
        extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        let pid = child.id() as i32;
        // группа = номер процесса (process_group(0)); 15 — TERM, потом 9 — KILL
        unsafe {
            kill(-pid, 15);
        }
        for _ in 0..20 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        unsafe {
            kill(-pid, 9);
        }
    }
    let _ = child.kill();
    for _ in 0..20 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn env_allowlist() {
        for k in ["PATH", "HOME", "LC_TIME", "XDG_RUNTIME_DIR", "SystemRoot", "ProgramFiles(x86)", "DBUS_SESSION_BUS_ADDRESS", "TMPDIR", "XAUTHORITY"]
        {
            assert!(super::env_allowed(k), "{k}");
        }
        for k in [
            "DATABASE_URL",
            "GH_PAT",
            "SERVICE_AUTH",
            "DB_PASS",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "https_proxy",
            "ALL_PROXY",
            "SSH_AUTH_SOCK",
            "AWS_SECRET_ACCESS_KEY",
        ] {
            assert!(!super::env_allowed(k), "{k}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn env_scrubbed_in_child() {
        std::env::set_var("AUDIT_DATABASE_URL", "postgres://u:p@h/db");
        std::env::set_var("AUDIT_GH_PAT", "x");
        let r = super::exec(&["sh", "-c", "echo \"[$AUDIT_DATABASE_URL$AUDIT_GH_PAT][$HOME]\""], std::time::Duration::from_secs(5), super::CHECK);
        let t = r.text().trim().to_string();
        assert!(t.starts_with("[]["), "{t}");
        assert!(!t.ends_with("[]"), "HOME должен остаться: {t}");
    }

    use super::*;

    #[test]
    fn outcomes() {
        assert_eq!(exec(&["нет-такой-программы-aptechka"], Duration::from_secs(2), CHECK), Outcome::Missing);
        #[cfg(unix)]
        {
            assert_eq!(exec(&["sh", "-c", "echo hi"], Duration::from_secs(5), CHECK), Outcome::Ok("hi".into()));
            assert!(matches!(exec(&["sh", "-c", "echo x; exit 3"], Duration::from_secs(5), CHECK), Outcome::Failed(3, ref s) if s == "x"));
            // дети команды тоже останавливаются: без группы sleep держал бы трубу 30 с
            let t = Instant::now();
            let r = exec(&["sh", "-c", "sleep 30 & sleep 30"], Duration::from_millis(300), CHECK);
            assert_eq!(r, Outcome::Timeout { stopped: true });
            assert!(t.elapsed() < Duration::from_secs(5));
            // команда сразу вышла, а оставленный ею процесс держит вывод открытым — не ждём его
            let t = Instant::now();
            let r = exec(&["sh", "-c", "echo ok; (setsid sleep 30 &)"], Duration::from_millis(500), CHECK);
            assert!(t.elapsed() < Duration::from_secs(3), "ждали {:?}", t.elapsed());
            assert_eq!(r.code(), 0);
            // команда напечатала и вышла, а фоновый потомок держит трубу: напечатанное не теряется
            let r = exec(&["sh", "-c", "echo important; (setsid sleep 4 &)"], Duration::from_millis(500), CHECK);
            assert_eq!(r.text(), "important");
            // ответ на английском: LC_ALL=C
            assert_eq!(exec(&["sh", "-c", "echo $LC_ALL"], Duration::from_secs(5), CHECK).text(), "C");
        }
    }
}
