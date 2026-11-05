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
        // значения флагов — из libc (на Linux они зависят от процессора)
        o.custom_flags(libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW });
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
    open_regular_opts(path.as_ref(), true)
}

/// То же, но follow=false не идёт по ссылке на последнем шаге (O_NOFOLLOW).
pub fn open_regular_opts(path: &Path, follow: bool) -> std::io::Result<std::fs::File> {
    #[cfg(not(unix))]
    let _ = follow;
    let mut o = std::fs::OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::custom_flags(&mut o, libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW });
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

/// Имена файлов с данными измерений в папке сеанса (их же убирают при выходе и при старте, если старше 12 часов).
pub const RUNTIME_DATA: [&str; 6] = ["checks.json", "checks.alive", "check_now", "speed.json", "speed.net", "dupes.json"];

/// Папка сеанса для данных измерений (результаты проверок, замер скорости, поиск дублей, замок проверяющего):
/// то, что не должно пережить выход из системы. Одна функция и для окна, и для виджета.
/// Linux — $XDG_RUNTIME_DIR/aptechka (tmpfs, очищается при выходе; сам $XDG_RUNTIME_DIR обязан быть абсолютным,
/// своим и 0700), запасной путь — /tmp/aptechka-<uid>; macOS — $TMPDIR/aptechka (личная папка пользователя,
/// система её чистит); Windows — %TEMP%\aptechka (там уборка при выходе и по возрасту). Папка своя, 0700,
/// не ссылка; чужая или ссылка — не берём: персональная запасная (refused_dir), в общие папки ничего не попадёт.
/// В тестах XDG_RUNTIME_DIR подменяет базу на любой системе.
pub fn runtime_dir() -> PathBuf {
    #[cfg(unix)]
    {
        extern "C" {
            fn getuid() -> u32;
        }
        let me = unsafe { getuid() };
        runtime_dir_for(me)
    }
    #[cfg(not(unix))]
    {
        let base =
            std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| cfg!(test) && p.is_absolute()).unwrap_or_else(std::env::temp_dir);
        let d = base.join("aptechka");
        let _ = std::fs::create_dir_all(&d);
        d
    }
}

#[cfg(unix)]
fn runtime_dir_for(me: u32) -> PathBuf {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let xdg = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute() && (cfg!(target_os = "linux") || cfg!(test)));
    if let Some(x) = xdg {
        let ok = std::fs::symlink_metadata(&x).is_ok_and(|m| m.is_dir() && m.uid() == me && m.permissions().mode() & 0o077 == 0);
        if ok {
            if let Some(d) = own_subdir(&x.join("aptechka"), me) {
                return d;
            }
        }
    }
    // в тестах общий запасной путь не трогаем: только заданная база или персональная запасная папка
    if cfg!(test) {
        return refused_dir(me);
    }
    let fallback = if cfg!(target_os = "macos") && std::env::var_os("TMPDIR").is_some() {
        std::env::temp_dir().join("aptechka")
    } else {
        std::env::temp_dir().join(format!("aptechka-{me}"))
    };
    own_subdir(&fallback, me).unwrap_or_else(|| {
        crate::diag::log("папка сеанса чужая или ссылка — данные измерений только в запасной личной папке");
        refused_dir(me)
    })
}

/// Создать (не «создать если нет»: занятое имя проверяем) папку 0700 и убедиться, что она наша, настоящая (не
/// ссылка) и закрыта. None — не подходит.
#[cfg(unix)]
fn own_subdir(path: &Path, me: u32) -> Option<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let mut b = std::fs::DirBuilder::new();
    b.mode(0o700);
    match b.create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let m = std::fs::symlink_metadata(path).ok()?;
    if !m.is_dir() || m.uid() != me {
        return None;
    }
    if m.permissions().mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    Some(path.to_path_buf())
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

/// Уборка своих данных при запуске: копии правок старше 7 дней (и больше 10 штук), прежние постоянные файлы
/// данных измерений (теперь они только в памяти и в папке сеанса), устаревшее в папке сеанса. Чужие данные (чаты, история,
/// настройки VPN) не трогаются.
pub fn housekeeping() {
    let d = private_dir();
    let now = std::time::SystemTime::now();
    let age = |p: &Path| std::fs::symlink_metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok());
    let day = std::time::Duration::from_secs(86400);
    // копии правок
    // backups — настоящая папка (не ссылка на чужую); удаляем только свои файлы вида «имя.<число>.bak»
    let bdir = d.join("backups");
    let bdir_ok = std::fs::symlink_metadata(&bdir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink());
    let is_backup = |p: &Path| {
        let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        n.strip_suffix(".bak").and_then(|r| r.rsplit_once('.')).is_some_and(|(_, num)| !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()))
    };
    if let Ok(rd) = std::fs::read_dir(if bdir_ok { bdir.clone() } else { PathBuf::from("/nonexistent-aptechka") }) {
        let mut v: Vec<(PathBuf, std::time::Duration)> = rd
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()) && is_backup(&e.path()))
            .filter_map(|e| Some((e.path(), age(&e.path())?)))
            .collect();
        v.sort_by_key(|x| x.1);
        for (i, (p, a)) in v.iter().enumerate() {
            if i >= 10 || *a > 7 * day {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    // переезд: прежние версии держали данные измерений в папке настроек; теперь они живут только в памяти и в
    // папке сеанса. Удаляем ровно эти имена, не переходя по ссылкам (ссылка удаляется сама, цель цела).
    for name in OLD_PERSISTENT {
        remove_file_nofollow(&d.join(name));
    }
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            let n = e.file_name();
            let n = n.to_string_lossy();
            // и недописанные временные копии этих же файлов от прежних версий («checks.tmp1234.0.ab12cd34»)
            let stem = n.split_once(".tmp").map(|(s, _)| s);
            let old_tmp = stem.is_some_and(|s| s.starts_with("footprint") || OLD_PERSISTENT.iter().any(|o| o.split('.').next() == Some(s)));
            if (n.starts_with("footprint") && n.ends_with(".json")) || old_tmp {
                remove_file_nofollow(&e.path());
            }
        }
    }
    crate::dupes::remove_legacy_cache();
    // папка сеанса: файлы старше 12 часов (на Windows и macOS её не чистят при выходе из системы)
    let rd_dir = runtime_dir();
    for name in RUNTIME_DATA {
        let p = rd_dir.join(name);
        if age(&p).is_some_and(|a| a > day / 2) {
            remove_file_nofollow(&p);
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
    // забытые копии базы OpenCode (oc_<pid>_<ns>_<n>): только свои каталоги (не ссылки), старше 10 минут
    if let Ok(rd) = std::fs::read_dir(&d) {
        for e in rd.flatten() {
            let p = e.path();
            let ours = e.file_name().to_str().is_some_and(|n| n.starts_with("oc_"));
            if ours && e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) && age(&p).is_some_and(|a| a > std::time::Duration::from_secs(600))
            {
                remove_tree_nofollow(&p);
            }
        }
    }
}

/// Старые постоянные файлы данных измерений в папке настроек (их больше не пишем).
pub const OLD_PERSISTENT: [&str; 9] =
    ["checks.json", "checks.alive", "check_now", "checks.lock", "takeover.lock", "speed.json", "speed.net", "speed_history.jsonl", "dupes.json"];

/// Удалить файл или ссылку (саму ссылку, не цель); каталоги не трогаем.
pub fn remove_file_nofollow(p: &Path) {
    if std::fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir()) {
        let _ = std::fs::remove_file(p);
    }
}

/// Удалить каталог со своими временными файлами, не переходя по ссылкам (ссылка внутри удаляется сама).
fn remove_tree_nofollow(p: &Path) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let q = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                remove_tree_nofollow(&q);
            } else {
                let _ = std::fs::remove_file(&q);
            }
        }
    }
    let _ = std::fs::remove_dir(p);
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
        let flags = libc::O_NONBLOCK | libc::O_NOFOLLOW;
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

/// Уборка при запуске: забытые копии базы OpenCode (старше 10 минут, только свои каталоги, ссылки не трогаем),
/// а `backups` — ссылка на чужую папку — не очищается.
#[cfg(all(test, unix))]
#[test]
fn housekeeping_stale_oc_dirs_and_backups_symlink() {
    let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("aptechka_hk_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let old = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &root);
    let d = private_dir();
    let age = |p: &Path, secs: u64| {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
        std::fs::File::open(p).unwrap().set_modified(t).unwrap();
    };
    // старый и свежий каталоги копий; в старом лежит файл и ссылка на чужой файл
    let victim = root.join("victim.txt");
    std::fs::write(&victim, "INTACT").unwrap();
    std::fs::create_dir(d.join("oc_1_2_3")).unwrap();
    std::fs::write(d.join("oc_1_2_3/opencode.db"), "x").unwrap();
    std::os::unix::fs::symlink(&victim, d.join("oc_1_2_3/link")).unwrap();
    age(&d.join("oc_1_2_3"), 3600);
    std::fs::create_dir(d.join("oc_9_9_9")).unwrap(); // свежий — не трогаем
                                                      // ссылка с именем oc_*: удаляется не её цель и не она сама (это не наш каталог)
    let other = root.join("other_dir");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("keep"), "1").unwrap();
    std::os::unix::fs::symlink(&other, d.join("oc_link")).unwrap();
    // backups — ссылка на чужую папку со старыми файлами
    std::fs::write(other.join("a.1.bak"), "1").unwrap();
    age(&other.join("a.1.bak"), 30 * 86400);
    std::os::unix::fs::symlink(&other, d.join("backups")).unwrap();
    housekeeping();
    assert!(!d.join("oc_1_2_3").exists());
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "INTACT");
    assert!(d.join("oc_9_9_9").exists());
    assert!(other.join("keep").exists() && other.join("a.1.bak").exists());
    match old {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Папка сеанса: чужая или ссылка — отказ; своя — 0700 и берётся; подмена базы через XDG_RUNTIME_DIR.
#[cfg(all(test, unix))]
#[test]
fn runtime_dir_safety() {
    use std::os::unix::fs::PermissionsExt;
    extern "C" {
        fn getuid() -> u32;
    }
    let me = unsafe { getuid() };
    let root = std::env::temp_dir().join(format!("aptechka_rt_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    // своя: создаётся 0700; открытые права исправляются
    let mine = root.join("mine");
    assert_eq!(own_subdir(&mine, me), Some(mine.clone()));
    assert_eq!(std::fs::metadata(&mine).unwrap().permissions().mode() & 0o777, 0o700);
    std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(own_subdir(&mine, me), Some(mine.clone()));
    assert_eq!(std::fs::metadata(&mine).unwrap().permissions().mode() & 0o777, 0o700);
    // «чужая»: для нас владелец другой пользователь
    assert_eq!(own_subdir(&mine, me + 1), None);
    // ссылка (даже на свою папку) — отказ
    let link = root.join("link");
    std::os::unix::fs::symlink(&mine, &link).unwrap();
    assert_eq!(own_subdir(&link, me), None);
    // целиком: база из XDG_RUNTIME_DIR берётся, а ссылка на месте «aptechka» — нет
    let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let old = std::env::var_os("XDG_RUNTIME_DIR");
    let base = root.join("run");
    std::fs::create_dir(&base).unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("XDG_RUNTIME_DIR", &base);
    assert_eq!(runtime_dir(), base.join("aptechka"));
    std::fs::remove_dir(base.join("aptechka")).unwrap();
    std::os::unix::fs::symlink(&mine, base.join("aptechka")).unwrap();
    let d = runtime_dir();
    assert_ne!(d, base.join("aptechka"));
    assert!(d.to_string_lossy().contains("aptechka-refused-"));
    // база открыта для других (0755) — тоже отказ
    std::fs::remove_file(base.join("aptechka")).unwrap();
    std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(runtime_dir().to_string_lossy().contains("aptechka-refused-"));
    match old {
        Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
        None => std::env::remove_var("XDG_RUNTIME_DIR"),
    }
    let _ = std::fs::remove_dir(refused_dir(me));
    let _ = std::fs::remove_dir_all(&root);
}

/// Уборка при запуске удаляет ровно прежние файлы данных измерений (и ссылку — не цель), остальное цело.
#[cfg(all(test, unix))]
#[test]
fn housekeeping_removes_exactly_old_measurement_files() {
    let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("aptechka_hk2_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("run")).unwrap();
    let (oc, orr) = (std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("XDG_RUNTIME_DIR"));
    std::env::set_var("XDG_CONFIG_HOME", root.join("cfg"));
    std::env::set_var("XDG_RUNTIME_DIR", root.join("run"));
    std::fs::set_permissions(root.join("run"), std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
    let d = private_dir();
    let victim = root.join("victim.txt");
    std::fs::write(&victim, "INTACT").unwrap();
    for n in OLD_PERSISTENT.iter().chain(["footprint.json", "footprint-v4.json", "footprint-v9.json"].iter()) {
        std::fs::write(d.join(n), "x").unwrap();
    }
    // ссылка с таким именем: удаляется сама, цель цела
    std::fs::remove_file(d.join("speed.json")).unwrap();
    std::os::unix::fs::symlink(&victim, d.join("speed.json")).unwrap();
    let keep = ["ui.json", "snapshot.json", "footprint_notes.txt", "backups.json"];
    for n in keep {
        std::fs::write(d.join(n), "keep").unwrap();
    }
    std::fs::create_dir(d.join("backups")).unwrap();
    std::fs::write(d.join("backups/a.1.bak"), "keep").unwrap();
    housekeeping();
    let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    let mut want: Vec<String> = keep.iter().map(|s| s.to_string()).chain(["backups".to_string()]).collect();
    want.sort();
    assert_eq!(left, want);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "INTACT");
    assert!(d.join("backups/a.1.bak").exists());
    // папка сеанса: старые данные уходят, свежие остаются
    let rd = runtime_dir();
    std::fs::write(rd.join("checks.json"), "{}").unwrap();
    std::fs::write(rd.join("speed.json"), "{}").unwrap();
    let t = std::time::SystemTime::now() - std::time::Duration::from_secs(13 * 3600);
    std::fs::File::open(rd.join("speed.json")).unwrap().set_modified(t).unwrap();
    housekeeping();
    assert!(rd.join("checks.json").exists() && !rd.join("speed.json").exists());
    for (k, o) in [("XDG_CONFIG_HOME", oc), ("XDG_RUNTIME_DIR", orr)] {
        match o {
            Some(v) => std::env::set_var(k, v),
            None => std::env::remove_var(k),
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
