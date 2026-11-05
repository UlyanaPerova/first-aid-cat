//! Файлы Аптечки на диске: папка настроек только для владельца (0700), файлы — 0600 (в результатах проверок
//! бывают адреса и имена сетей), запись атомарная (сбой посреди записи не оставит пустой или половинный файл),
//! замок между процессами (окно и виджет пишут одни и те же файлы).

use std::path::{Path, PathBuf};

/// Домашняя папка: HOME, на Windows — USERPROFILE (из меню «Пуск» HOME не задан). Только абсолютный путь:
/// пустой или относительный — None (иначе «~/Documents» превратился бы в папку Documents в текущем каталоге).
pub fn home_dir() -> Option<PathBuf> {
    let order: [&str; 2] = if cfg!(windows) { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    order.iter().filter_map(std::env::var_os).map(PathBuf::from).find(|p| p.is_absolute())
}

/// Домашняя папка строкой (пусто — неизвестна).
pub fn home_str() -> String {
    home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Прочитать текстовый файл не больше `max` байт — только обычный файл (не ссылку, не FIFO, не устройство):
/// FIFO без писателя повесил бы поток навсегда, «/dev/zero» — съел бы память, ссылка увела бы чтение
/// в чужой файл. Открытие без ожидания (O_NONBLOCK) и без перехода по ссылке (O_NOFOLLOW), проверка — по
/// уже открытому файлу.
pub fn read_capped(path: &Path, max: u64) -> std::io::Result<String> {
    read_opts(path, max, false)
}

/// follow — разрешить ссылку на последнем шаге (чужие настройки: ~/.bashrc → ~/dotfiles/bashrc — это
/// нормально; FIFO и огромные файлы всё равно отсекаются проверкой открытого файла).
fn read_opts(path: &Path, max: u64, follow: bool) -> std::io::Result<String> {
    let b = read_bytes_opts(path, max, follow)?;
    String::from_utf8(b).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.utf8_error()))
}

fn read_bytes_opts(path: &Path, max: u64, follow: bool) -> std::io::Result<Vec<u8>> {
    #[cfg(not(unix))]
    let _ = follow;
    use std::io::Read;
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK | O_NOFOLLOW (значения Linux; на macOS — свои)
        // O_NOFOLLOW на Linux зависит от процессора: 0o400000 на x86, 0o100000 на ARM
        #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
        let (nonblock, nofollow) = (0o4000, 0o100000);
        #[cfg(all(target_os = "linux", not(any(target_arch = "aarch64", target_arch = "arm"))))]
        let (nonblock, nofollow) = (0o4000, 0o400000);
        #[cfg(target_os = "macos")]
        let (nonblock, nofollow) = (0x0004, 0x0100);
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        o.custom_flags(nonblock | if follow { 0 } else { nofollow });
    }
    let f = o.open(path)?;
    let m = f.metadata()?;
    if !m.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "не обычный файл"));
    }
    if m.len() > max {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "файл слишком большой"));
    }
    let mut s = Vec::new();
    f.take(max).read_to_end(&mut s)?;
    Ok(s)
}

/// Файл целиком байтами (с переходом по ссылке на последнем шаге, как `read_text`): для файлов, где
/// кодировка не гарантирована (история команд, rc-файлы). Те же защиты: обычный файл, без ожидания, не больше `max`.
pub fn read_bytes(path: impl AsRef<Path>, max: u64) -> std::io::Result<Vec<u8>> {
    read_bytes_opts(path.as_ref(), max, true)
}

/// Как `read_text`, но не-UTF-8 байты (cp1251, koi8, «метафицированный» zsh) заменяются на «�», а не обрывают
/// чтение. Для файлов, где мы лишь ищем подстроки (настройки прокси, ключи): один байт в комментарии
/// не должен «ослеплять» проверку. Файлы, которые пишем сами (JSON, состояние), читаем строго.
pub fn read_text_lossy(path: impl AsRef<Path>) -> std::io::Result<String> {
    let b = read_bytes_opts(path.as_ref(), 32 << 20, true)?;
    Ok(String::from_utf8_lossy(&b).into_owned())
}

/// Открыть чужой файл для чтения по частям (сессии агентов): только обычный файл, без ожидания FIFO.
pub fn open_regular(path: impl AsRef<Path>) -> std::io::Result<std::fs::File> {
    let path = path.as_ref();
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(target_os = "linux")]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut o, 0o4000);
    #[cfg(target_os = "macos")]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut o, 0x0004);
    let f = o.open(path)?;
    if !f.metadata()?.is_file() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "не обычный файл"));
    }
    Ok(f)
}

/// Чтение чужого или общего файла (настройки браузеров, сессии агентов, кэши, файлы оболочки) — как
/// std::fs::read_to_string, но через read_capped: не больше 32 МБ, только обычный файл, без ссылок и ожидания.
pub fn read_text(path: impl AsRef<Path>) -> std::io::Result<String> {
    read_opts(path.as_ref(), 32 << 20, true)
}

/// Папка настроек (создаётся; на Linux и macOS — закрыта от других пользователей).
pub fn private_dir() -> PathBuf {
    let d = crate::prefs::config_dir();
    if !d.is_dir() {
        // сразу 0700 (create_dir_all создал бы 0755 и на миг открыл бы папку)
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        let _ = b.create(&d);
    }
    // папка чужая (кто-то создал её заранее, например в общем /tmp) — в ней не храним ничего
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // свой номер пользователя — у системы (getuid), а не через /proc (на macOS его нет)
        extern "C" {
            fn getuid() -> u32;
        }
        let me = unsafe { getuid() };
        if let Ok(m) = std::fs::symlink_metadata(&d) {
            if m.uid() != me || m.file_type().is_symlink() {
                crate::diag::log("папка настроек принадлежит не мне (или это ссылка) — работаю без сохранения");
                return refused_dir(me);
            }
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(&d) {
            if m.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
            }
        }
    }
    d
}

/// Запасная папка, когда настоящая папка настроек чужая: одна на процесс, создаётся заново (не «создать
/// если нет» — чужую готовую не берём), права 0700, имя непредсказуемое, лежит в личной папке сеанса
/// ($XDG_RUNTIME_DIR), а при её отсутствии — в temp. Не получилось создать — путь, куда записать нельзя
/// (запись просто не удастся, ничего не попадёт в общие папки).
#[cfg(unix)]
fn refused_dir(me: u32) -> PathBuf {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute() && std::fs::metadata(p).is_ok_and(|m| m.uid() == me))
            .unwrap_or_else(std::env::temp_dir);
        for i in 0..8u32 {
            let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
            let d = base.join(format!("aptechka-refused-{me}-{}-{i}-{salt:08x}", std::process::id()));
            let mut b = std::fs::DirBuilder::new();
            b.mode(0o700);
            // не recursive и без «уже есть»: занято — следующее имя
            if b.create(&d).is_ok() && std::fs::symlink_metadata(&d).is_ok_and(|m| m.uid() == me && m.is_dir()) {
                return d;
            }
        }
        PathBuf::from("/dev/null/aptechka-refused")
    })
    .clone()
}

/// Записать файл целиком: во временный рядом (с правами 0600), потом подменить.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_mode(path, data, 0o600)
}

/// Записать атомарно с правами `mode`: временный файл рядом — с непредсказуемым именем и create_new
/// (O_EXCL: заранее подложенная на это имя ссылка не сработает, создание просто не удастся), потом подмена.
pub fn write_mode(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    write_full(path, data, mode, true)
}

/// Как write_private, но без принудительной записи на диск (fsync): для частой служебной отметки
/// («проверяющий жив» раз в 5 с) — потерять её при сбое питания не страшно, а диск не дёргается.
pub fn write_private_quick(path: &Path, data: &[u8]) -> std::io::Result<()> {
    write_full(path, data, 0o600, false)
}

fn write_full(path: &Path, data: &[u8], mode: u32, sync: bool) -> std::io::Result<()> {
    use std::io::Write;
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let tmp = path.with_extension(format!("tmp{}.{n}.{salt:08x}", std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, mode);
    #[cfg(not(unix))]
    let _ = mode;
    let res = o.open(&tmp).and_then(|mut f| {
        f.write_all(data)?;
        if sync {
            f.sync_all()?;
        }
        drop(f);
        std::fs::rename(&tmp, path)?;
        // переименование надёжно на диске, только когда записана и сама папка
        #[cfg(unix)]
        if sync {
            if let Some(d) = path.parent().and_then(|d| std::fs::File::open(d).ok()) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    });
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// Уборка своих данных при запуске: копии правок старше 7 дней (и больше 10 штук), устаревший замер
/// скорости, память проверки «цифрового следа» старше 30 дней и прежних версий. Чужие данные (чаты, история,
/// настройки VPN) не трогаются.
pub fn housekeeping() {
    let d = private_dir();
    let now = std::time::SystemTime::now();
    let age = |p: &Path| std::fs::symlink_metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok());
    let day = std::time::Duration::from_secs(86400);
    // копии правок
    if let Ok(rd) = std::fs::read_dir(d.join("backups")) {
        let mut v: Vec<(PathBuf, std::time::Duration)> =
            rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).filter_map(|e| Some((e.path(), age(&e.path())?))).collect();
        v.sort_by_key(|x| x.1);
        for (i, (p, a)) in v.iter().enumerate() {
            if i >= 10 || *a > 7 * day {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    // замер скорости — 6 часов; память «цифрового следа» — 30 дней, прежние версии — сразу
    for (name, ttl) in [("speed.json", day / 4), ("speed.net", day / 4), ("footprint-v4.json", 30 * day)] {
        let p = d.join(name);
        if age(&p).is_some_and(|a| a > ttl) {
            let _ = std::fs::remove_file(&p);
        }
    }
    // история скорости: не старше 90 дней и не больше 200 строк (дописывает speed_remember; здесь — подчистка,
    // если замеров давно нет и файл так и лежал бы)
    trim_history(&d.join("speed_history.jsonl"), 90 * 86400, 200);
    // кэши без своего срока: если не обновлялись 60 дней, значит, давно не нужны (пересоздадутся сами)
    for name in ["checks.json", "checks.alive", "check_now", "dupes.json"] {
        let p = d.join(name);
        if age(&p).is_some_and(|a| a > 60 * day) {
            let _ = std::fs::remove_file(&p);
        }
    }
    // забытые временные файлы записи (процесс упал между созданием и переименованием)
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            let p = e.path();
            let tmp = p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.starts_with("tmp") && x.contains('.'));
            if tmp && e.file_type().is_ok_and(|t| t.is_file()) && age(&p).is_some_and(|a| a > day) {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    for old in ["footprint.json", "footprint-v2.json", "footprint-v3.json"] {
        let _ = std::fs::remove_file(d.join(old));
    }
}

/// Оставить в журнале JSON-строк (по одной на строку) только свежие (поле "t" — секунды, не старше `max_age`
/// и не из будущего) и не больше `max_lines` последних. Файл без изменений не переписывается.
pub fn trim_history(p: &Path, max_age: u64, max_lines: usize) {
    let Ok(text) = read_text(p) else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let fresh: Vec<&str> = all
        .iter()
        .copied()
        .filter(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v["t"].as_f64())
                .is_some_and(|t| (-60.0..max_age as f64).contains(&(now - t)))
        })
        .collect();
    let start = fresh.len().saturating_sub(max_lines);
    let keep = &fresh[start..];
    if keep.len() == all.len() {
        return;
    }
    if keep.is_empty() {
        let _ = std::fs::remove_file(p);
    } else {
        let _ = write_private(p, (keep.join("\n") + "\n").as_bytes());
    }
}

/// Подождать замок до `wait` (для коротких дел: прочитать-поправить-записать настройки).
/// None — так и не дождались; тогда лучше работать без замка, чем зависнуть.
pub fn lock_wait(p: &Path, wait: std::time::Duration) -> Option<lock::Held> {
    let until = std::time::Instant::now() + wait;
    loop {
        if let Some(h) = lock::try_take(p) {
            return Some(h);
        }
        if std::time::Instant::now() >= until {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

// ───────────────────────────── замок ─────────────────────────────

/// Файл-замок: кто держит — тот проверяет. Держится, пока процесс жив (система снимает его сама при выходе
/// или падении).
pub mod lock {
    use std::fs::{File, OpenOptions};
    use std::path::Path;

    pub struct Held(#[allow(dead_code)] File);

    /// Файл замка — только для владельца (0600).
    #[cfg(unix)]
    fn open_unix(p: &Path) -> std::io::Result<File> {
        use std::os::unix::fs::OpenOptionsExt;
        // без перехода по ссылке и без ожидания (подложенный FIFO не повесит планировщик); только обычный файл
        #[cfg(all(target_os = "linux", any(target_arch = "aarch64", target_arch = "arm")))]
        let flags = 0o4000 | 0o100000;
        #[cfg(all(target_os = "linux", not(any(target_arch = "aarch64", target_arch = "arm"))))]
        let flags = 0o4000 | 0o400000;
        #[cfg(target_os = "macos")]
        let flags = 0x0004 | 0x0100;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let flags = 0;
        let f = OpenOptions::new().create(true).truncate(false).write(true).mode(0o600).custom_flags(flags).open(p)?;
        if !f.metadata()?.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "замок — не обычный файл"));
        }
        Ok(f)
    }

    #[cfg(unix)]
    fn try_lock(f: &File) -> bool {
        use std::os::unix::io::AsRawFd;
        extern "C" {
            fn flock(fd: i32, op: i32) -> i32;
        }
        // LOCK_EX | LOCK_NB
        unsafe { flock(f.as_raw_fd(), 2 | 4) == 0 }
    }

    /// Взять замок (не ждать): Some — теперь проверяем мы.
    pub fn try_take(p: &Path) -> Option<Held> {
        #[cfg(unix)]
        {
            let f = open_unix(p).ok()?;
            return try_lock(&f).then_some(Held(f));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // без совместного доступа: второй процесс файл не откроет, пока первый держит
            return OpenOptions::new().create(true).truncate(false).write(true).share_mode(0).open(p).ok().map(Held);
        }
        #[allow(unreachable_code)]
        None
    }

    /// Результат попытки взять замок с различением «занят» и «замок испорчен».
    pub enum Take {
        Got(Held),
        /// держит другой процесс
        Busy,
        /// файл замка не обычный (ссылка, FIFO, каталог) или не открывается
        Broken(std::io::Error),
    }

    /// Как try_take, но отличает занятость от порчи; ссылку и FIFO на месте замка убирает и пробует ещё раз
    /// (убирается сама запись в нашей закрытой папке, а не то, куда она вела). Каталог не трогаем — это ошибка.
    pub fn take(p: &Path) -> Take {
        #[cfg(unix)]
        {
            for attempt in 0..2 {
                match open_unix(p) {
                    Ok(f) => return if try_lock(&f) { Take::Got(Held(f)) } else { Take::Busy },
                    Err(e) => {
                        let odd = std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_file() && !m.is_dir());
                        if attempt == 0 && odd && std::fs::remove_file(p).is_ok() {
                            continue;
                        }
                        return Take::Broken(e);
                    }
                }
            }
            unreachable!()
        }
        #[cfg(not(unix))]
        match try_take(p) {
            Some(h) => Take::Got(h),
            None if p.parent().is_some_and(|d| d.is_dir()) && !p.exists() => Take::Broken(std::io::Error::other("замок не создаётся")),
            None => Take::Busy,
        }
    }

    /// Свободен ли замок сейчас (проверяющий закрылся).
    pub fn free(p: &Path) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            extern "C" {
                fn flock(fd: i32, op: i32) -> i32;
            }
            let Ok(f) = open_unix(p) else { return false };
            if try_lock(&f) {
                // взяли на миг — отпустить (закрытие файла снимает замок); возьмёт try_take
                unsafe {
                    flock(f.as_raw_fd(), 8); // LOCK_UN
                }
                return true;
            }
            return false;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            return OpenOptions::new().create(true).truncate(false).write(true).share_mode(0).open(p).is_ok();
        }
        #[allow(unreachable_code)]
        false
    }
}

#[cfg(all(test, unix))]
#[test]
fn capped_reads() {
    let d = std::env::temp_dir().join(format!("aptechka_capped_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    let f = d.join("a.json");
    std::fs::write(&f, "{}").unwrap();
    assert_eq!(read_capped(&f, 100).unwrap(), "{}");
    // ссылка — не читаем
    let l = d.join("link");
    let _ = std::os::unix::fs::symlink(&f, &l);
    assert!(read_capped(&l, 100).is_err());
    // слишком большой — отказ
    std::fs::write(&f, "x".repeat(200)).unwrap();
    assert!(read_capped(&f, 100).is_err());
    // FIFO — не ждём писателя, отказ сразу
    let fifo = d.join("fifo");
    let _ = std::process::Command::new("mkfifo").arg(&fifo).status();
    if fifo.exists() {
        let t = std::time::Instant::now();
        assert!(read_capped(&fifo, 100).is_err());
        assert!(t.elapsed() < std::time::Duration::from_secs(1));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(all(test, unix))]
#[test]
fn private_files() {
    use std::os::unix::fs::PermissionsExt;
    let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let d = std::env::temp_dir().join(format!("aptechka_store_{}", std::process::id()));
    std::fs::create_dir_all(d.join("aptechka")).unwrap();
    std::fs::set_permissions(d.join("aptechka"), std::fs::Permissions::from_mode(0o775)).unwrap();
    let old = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &d);
    let dir = private_dir();
    match old {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
    let f = dir.join("x.json");
    std::fs::write(&f, "старое").unwrap();
    write_private(&f, b"new").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    // временных файлов не осталось
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(all(test, unix))]
#[test]
fn lossy_reads() {
    let d = std::env::temp_dir().join(format!("aptechka_lossy_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&d);
    let f = d.join("rc");
    std::fs::write(&f, b"# \xef\xf0\n export http_proxy=x\n").unwrap();
    assert!(read_text(&f).is_err());
    assert!(read_text_lossy(&f).unwrap().contains("http_proxy"));
    // защиты те же: каталог — отказ
    assert!(read_text_lossy(&d).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(all(test, unix))]
#[test]
fn lock_replaced_by_symlink_or_dir() {
    let d = std::env::temp_dir().join(format!("aptechka_lock_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let target = d.join("target");
    std::fs::write(&target, "x").unwrap();
    let l = d.join("fix-x.lock");
    std::os::unix::fs::symlink(&target, &l).unwrap();
    // ссылку убираем и берём замок; то, куда она вела, цело
    let h = lock::take(&l);
    assert!(matches!(h, lock::Take::Got(_)));
    assert!(std::fs::symlink_metadata(&l).unwrap().is_file());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "x");
    // второй раз — занято (а не «испорчен»)
    assert!(matches!(lock::take(&l), lock::Take::Busy));
    drop(h);
    // каталог на месте замка — испорчен, не «занят»
    let dd = d.join("dir.lock");
    std::fs::create_dir(&dd).unwrap();
    assert!(matches!(lock::take(&dd), lock::Take::Broken(_)));
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(all(test, unix))]
#[test]
fn refused_dir_is_private_and_unpredictable() {
    use std::os::unix::fs::PermissionsExt;
    extern "C" {
        fn getuid() -> u32;
    }
    let a = refused_dir(unsafe { getuid() });
    assert!(a.is_dir());
    assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(a.to_string_lossy().contains("aptechka-refused-"));
    let _ = std::fs::remove_dir(&a);
}

#[cfg(test)]
#[test]
fn history_trim() {
    let d = std::env::temp_dir().join(format!("aptechka_hist_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("h.jsonl");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let mut t = format!("{{\"t\": {}}}\n", now - 100.0 * 86400.0);
    for i in 0..5 {
        t += &format!("{{\"t\": {}}}\n", now - 1000.0 + i as f64);
    }
    std::fs::write(&p, t).unwrap();
    trim_history(&p, 90 * 86400, 3);
    assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 3);
    std::fs::write(&p, format!("{{\"t\": {}}}\n", now - 100.0 * 86400.0)).unwrap();
    trim_history(&p, 90 * 86400, 3);
    assert!(!p.exists());
    let _ = std::fs::remove_dir_all(&d);
}
