//! Общие помощники регрессионных тестов (только `cfg(test)`): детерминированный генератор, временные папки,
//! подмена окружения, чтение счётчиков процесса, фальшивый HTTP-сервер.
//!
//! Правило: ожидаемые значения в тестах считаются НЕ через проверяемую функцию — руками (с выкладкой в комментарии),
//! наивной отдельной реализацией или константами, посчитанными отдельным Python-скриптом.

use std::path::{Path, PathBuf};

/// splitmix64 — собственный детерминированный генератор (новых зависимостей не добавляем).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Число в [0, n).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    /// Число в [a, b].
    pub fn range(&mut self, a: i64, b: i64) -> i64 {
        a + self.below((b - a + 1) as u64) as i64
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// Прогнать `f` по набору фиксированных зёрен; при падении печатает зерно (чтобы воспроизвести одним числом).
/// Число зёрен можно увеличить переменной REGRESS_SEEDS (по умолчанию 200).
pub fn for_seeds(name: &str, f: impl Fn(&mut Rng)) {
    let n: u64 = std::env::var("REGRESS_SEEDS").ok().and_then(|v| v.parse().ok()).unwrap_or(200);
    for i in 0..n {
        let seed = 0x5EED_0000_0000 + i;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut Rng(seed))));
        if let Err(e) = r {
            eprintln!("[{name}] ПАДЕНИЕ на зерне {seed:#x} (повторить: Rng({seed:#x}))");
            std::panic::resume_unwind(e);
        }
    }
}

/// Временная папка, удаляется при выходе.
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new(name: &str) -> Tmp {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "aptechka_regress_{name}_{}_{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }
    pub fn p(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Подмена HOME / XDG_CONFIG_HOME / XDG_RUNTIME_DIR на временную папку на время `f`. Окружение общее на весь процесс,
/// поэтому берётся общий замок `crate::ENV_LOCK`. Настоящая домашняя папка и настройки не затрагиваются.
pub fn with_fake_home<R>(name: &str, f: impl FnOnce(&Path) -> R) -> R {
    let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let t = Tmp::new(name);
    let home = t.p("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(t.p("run")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(t.p("run"), std::fs::Permissions::from_mode(0o700));
    }
    let keys = ["HOME", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR", "CLAUDE_CONFIG_DIR", "USERPROFILE"];
    let old: Vec<_> = keys.iter().map(|k| (*k, std::env::var_os(k))).collect();
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);
    std::env::set_var("XDG_CONFIG_HOME", t.p("cfg"));
    std::env::set_var("XDG_RUNTIME_DIR", t.p("run"));
    std::env::remove_var("CLAUDE_CONFIG_DIR");
    // защита от промаха: настоящие данные не должны оказаться под ногами
    assert_eq!(crate::store::home_dir().as_deref(), Some(home.as_path()), "HOME не подменился — тест остановлен");
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&home)));
    for (k, v) in old {
        match v {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    match r {
        Ok(r) => r,
        Err(e) => std::panic::resume_unwind(e),
    }
}

/// Счётчики своего процесса: (RSS в КБ, число потоков, число открытых дескрипторов). Только Linux; иначе нули.
pub fn proc_stats() -> (u64, u64, u64) {
    #[cfg(target_os = "linux")]
    {
        let st = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let get = |k: &str| st.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
        let fds = std::fs::read_dir("/proc/self/fd").map(|d| d.count() as u64).unwrap_or(0);
        (get("VmRSS:"), get("Threads:"), fds)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (0, 0, 0)
    }
}

/// Длительность длинных испытаний в секундах: переменная REGRESS_SOAK_SECS (по умолчанию `def`).
pub fn soak_secs(def: u64) -> u64 {
    std::env::var("REGRESS_SOAK_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(def)
}

/// Строка CSV для длинных испытаний (если задан REGRESS_CSV — дописывается в файл, иначе в stderr).
pub fn csv_line(line: &str) {
    use std::io::Write;
    match std::env::var_os("REGRESS_CSV") {
        Some(p) => {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = writeln!(f, "{line}");
            }
        }
        None => eprintln!("{line}"),
    }
}

/// FNV-1a 64.
pub fn fnv64(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Выдуманный «ключ» нужного вида: начало + случайный хвост с буквами обеих регистров и цифрами
/// (проходит проверку «правдоподобности» — как настоящий, но случайный и ничей).
pub fn fake_key(r: &mut Rng, prefix: &str, tail_len: usize) -> String {
    const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    assert!(tail_len >= 16, "хвост выдуманного ключа короче 16 знаков не удовлетворит условиям правдоподобности");
    loop {
        let t: String = (0..tail_len).map(|_| A[r.below(A.len() as u64) as usize] as char).collect();
        let up = t.chars().filter(|c| c.is_ascii_uppercase()).count();
        let dg = t.chars().filter(|c| c.is_ascii_digit()).count();
        let lt = t.chars().filter(|c| c.is_ascii_alphabetic()).count();
        // повтор куска из 8 знаков «выдаёт выдумку» — при короткой длине его почти нет, но проверим
        let mut seen = std::collections::HashSet::new();
        let rep = t.as_bytes().windows(8).any(|w| !seen.insert(w.to_vec()));
        // «xxxx» и «EXAMPLE» программа считает признаком выдуманного примера (случайный настоящий ключ их почти не содержит)
        let example = t.to_ascii_lowercase().contains("xxxx") || t.contains("EXAMPLE");
        if up >= 3 && dg >= 3 && lt >= 12 && !rep && !example {
            return format!("{prefix}{t}");
        }
    }
}

/// Локальный HTTP-сервер: каждое соединение обрабатывает `handler(поток, номер запроса)`.
/// Возвращает адрес «http://127.0.0.1:порт». Поток живёт до конца процесса теста (порт закрывается вместе с ним).
pub fn local_server(handler: impl Fn(std::net::TcpStream, usize) + Send + Sync + 'static) -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let h = std::sync::Arc::new(handler);
    std::thread::spawn(move || {
        let n = std::sync::atomic::AtomicUsize::new(0);
        for c in l.incoming().flatten() {
            let h = h.clone();
            let i = n.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::thread::spawn(move || h(c, i));
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// Прочитать заголовок запроса до пустой строки; вернуть первую строку («GET /… HTTP/1.1») и заголовки целиком.
pub fn read_head(c: &mut std::net::TcpStream) -> Option<String> {
    use std::io::Read;
    let mut acc = Vec::new();
    let mut b = [0u8; 1];
    while !acc.ends_with(b"\r\n\r\n") {
        if c.read(&mut b).unwrap_or(0) == 0 {
            return None;
        }
        acc.push(b[0]);
        if acc.len() > 64 * 1024 {
            return None;
        }
    }
    Some(String::from_utf8_lossy(&acc).into_owned())
}

/// Тесты с допусками по настоящему времени (задержка и скорость против локального сервера) на общих машинах CI
/// ненадёжны: соседние задачи растягивают паузы в разы (на macOS-раннере джиттер вышел 90 мс вместо 20, скорость 4,6
/// вместо 8 Мбит/с). Под CI (переменная CI) такие тесты пропускаются; REGRESS_TIMING=1 включает их и там.
pub fn timing_ok(test: &str) -> bool {
    let ok = std::env::var_os("CI").is_none() || std::env::var_os("REGRESS_TIMING").is_some();
    if !ok {
        eprintln!("{test}: пропущен под CI (зависит от настоящего времени; REGRESS_TIMING=1 — включить)");
    }
    ok
}
