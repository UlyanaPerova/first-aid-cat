//! Копии файлов: одинаковые по содержимому файлы в обычных папках человека
//! (Загрузки, Документы, Изображения, Видео, Музыка, Рабочий стол).
//!
//! Ищем по кнопке, не каждую минуту: сначала группируем по размеру (быстро, только stat), потом
//! сравниваем md5 начала и конца файла, и лишь совпавшие — md5 целиком. Файлы меньше 1 МБ,
//! скрытые папки, папки программных проектов (.git, Cargo.toml, package.json, requirements.txt…) и моделей
//! (checkpoint-*, adapter_config.json, tokenizer_config.json) пропускаем: там одинаковые файлы бывают нарочно.
//! Результат — в ~/.cache/aptechka/dupes.json; лишние копии уходят в Корзину (можно вернуть).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use crate::diag::{fx, Check, INFO, OK};

const MIN_SIZE: u64 = 1 << 20;
const SHOW: usize = 8;

#[derive(Clone, Debug)]
pub struct Group {
    pub size: u64,
    /// Первый — тот, что оставляем; остальные — лишние копии.
    pub files: Vec<PathBuf>,
}

impl Group {
    pub fn waste(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
    /// Постоянный код группы (по размеру и путям): кнопка «в корзину» помнит его, а не номер строки —
    /// номера сдвигаются, когда группы пропадают или пересчитываются, и кнопка указала бы на чужую группу.
    pub fn id(&self) -> String {
        let mut v: Vec<String> = self.files.iter().map(|p| p.to_string_lossy().to_string()).collect();
        v.sort();
        let mut h: u64 = 0xcbf29ce484222325 ^ self.size;
        for b in v.join("\n").bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        format!("{h:016x}")
    }
}

/// Папки человека: Рабочий стол, Загрузки, Документы, Изображения, Видео, Музыка — там, где их держит система
/// (Linux — XDG, с русскими именами; Windows — «известные папки», в том числе перенесённые в OneDrive;
/// macOS — в домашней). Без домашней папки — пусто (не искать в текущем каталоге).
pub fn roots() -> Vec<PathBuf> {
    let Some(h) = crate::store::home_dir() else { return Vec::new() };
    let mut v: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if p != h && p.is_absolute() && p.is_dir() && !v.contains(&p) {
            v.push(p);
        }
    };
    #[cfg(windows)]
    for p in known_folders() {
        add(p);
    }
    let dirs = crate::store::read_text_lossy(h.join(".config/user-dirs.dirs")).unwrap_or_default();
    for key in ["DESKTOP", "DOWNLOAD", "DOCUMENTS", "PICTURES", "VIDEOS", "MUSIC"] {
        let line = dirs.lines().find(|l| l.starts_with(&format!("XDG_{key}_DIR=")));
        if let Some(p) = line.and_then(|l| l.split('"').nth(1)) {
            add(PathBuf::from(p.replace("$HOME", &h.to_string_lossy())));
        }
    }
    for d in ["Downloads", "Desktop", "Documents", "Pictures", "Videos", "Music"] {
        add(h.join(d));
    }
    v
}

/// Windows: где сейчас лежат Документы, Загрузки и т. д. (SHGetKnownFolderPath — учитывает перенос
/// в OneDrive или на другой диск).
#[cfg(windows)]
fn known_folders() -> Vec<PathBuf> {
    #[repr(C)]
    struct Guid(u32, u16, u16, [u8; 8]);
    #[link(name = "shell32")]
    extern "system" {
        fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: *mut core::ffi::c_void, path: *mut *mut u16) -> i32;
    }
    #[link(name = "ole32")]
    extern "system" {
        fn CoTaskMemFree(p: *mut core::ffi::c_void);
    }
    // FOLDERID_Desktop, Downloads, Documents, Pictures, Videos, Music
    let ids = [
        Guid(0xB4BFCC3A, 0xDB2C, 0x424C, [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41]),
        Guid(0x374DE290, 0x123F, 0x4565, [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B]),
        Guid(0xFDD39AD0, 0x238F, 0x46AF, [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7]),
        Guid(0x33E28130, 0x4E1E, 0x4676, [0x83, 0x5A, 0x98, 0x39, 0x5C, 0x3B, 0xC3, 0xBB]),
        Guid(0x18989B1D, 0x99B5, 0x455B, [0x84, 0x1C, 0xAB, 0x7C, 0x74, 0xE4, 0xDD, 0xFC]),
        Guid(0x4BD8D571, 0x6D19, 0x48D3, [0xBE, 0x97, 0x42, 0x22, 0x20, 0x08, 0x0E, 0x43]),
    ];
    let mut out = Vec::new();
    for id in &ids {
        let mut p: *mut u16 = std::ptr::null_mut();
        if unsafe { SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p) } == 0 && !p.is_null() {
            let mut n = 0;
            unsafe {
                while *p.add(n) != 0 && n < 4096 {
                    n += 1;
                }
                out.push(PathBuf::from(String::from_utf16_lossy(std::slice::from_raw_parts(p, n))));
            }
        }
        if !p.is_null() {
            unsafe { CoTaskMemFree(p as *mut core::ffi::c_void) };
        }
    }
    out
}

/// Папка, где одинаковые файлы бывают нарочно: программный проект, модель или контрольная точка обучения.
fn is_project(dir: &Path) -> bool {
    const MARKS: [&str; 12] = [
        ".git",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        "CMakeLists.txt",
        "requirements.txt",
        "setup.py",
        "environment.yml",
        "Pipfile",
        "adapter_config.json",
        "tokenizer_config.json",
    ];
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.starts_with("checkpoint-") || MARKS.iter().any(|m| dir.join(m).exists())
}

/// Папка, которую синхронизирует облако (Dropbox, Nextcloud, OneDrive, Яндекс Диск, Google Drive, Syncthing…):
/// удаление копии там удалит её на всех устройствах — такие папки не трогаем.
fn is_synced(dir: &Path) -> bool {
    const NAMES: [&str; 11] = [
        "dropbox",
        "nextcloud",
        "owncloud",
        "onedrive",
        "google drive",
        "yandex.disk",
        "yandexdisk",
        "mega",
        "pclouddrive",
        "icloud drive",
        "seafile",
    ];
    const MARKS: [&str; 6] = [".stfolder", ".dropbox", ".dropbox.cache", ".owncloudsync.log", ".nextcloudsync.log", ".sync_journal.db"];
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    NAMES.iter().any(|n| name == *n || name.starts_with(&format!("{n} ")) || name.starts_with(&format!("{n}-")))
        || MARKS.iter().any(|m| dir.join(m).exists())
        || std::fs::read_dir(dir).is_ok_and(|rd| {
            rd.flatten().take(200).any(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("._sync_") || n.starts_with(".sync_") && n.ends_with(".db")
            })
        })
}

/// Все файлы ≥ MIN_SIZE: путь, размер, время изменения.
fn walk(roots: &[PathBuf], deadline: Instant) -> (Vec<(PathBuf, u64, f64)>, bool) {
    let mut out = Vec::new();
    // не переходим на другие диски внутри папок (сетевой диск, облако rclone, флешка в «Документах»):
    // их файлы читались бы по сети, а Корзина для них — другая или её нет
    #[cfg(unix)]
    let dev = |p: &Path| std::fs::symlink_metadata(p).ok().map(|m| std::os::unix::fs::MetadataExt::dev(&m));
    #[cfg(not(unix))]
    let dev = |_: &Path| Some(0u64);
    let root_devs: Vec<Option<u64>> = roots.iter().map(|r| dev(r)).collect();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    let mut seen = std::collections::HashSet::new();
    while let Some(d) = stack.pop() {
        if Instant::now() > deadline {
            return (out, false);
        }
        if !seen.insert(d.clone()) || is_project(&d) || is_synced(&d) || roots.contains(&d) && d.ancestors().skip(1).any(is_synced) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            // срок — на каждом файле, а не только между папками (в папке могут быть сотни тысяч файлов)
            if Instant::now() > deadline {
                return (out, false);
            }
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                let p = e.path();
                if root_devs.contains(&dev(&p)) {
                    stack.push(p);
                }
            } else if let Ok(m) = e.metadata() {
                if m.len() >= MIN_SIZE && !cloud_placeholder(&m) {
                    let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0);
                    out.push((e.path(), m.len(), t));
                }
            }
        }
    }
    (out, true)
}

/// Файл облака, который лежит только «ярлыком» (iCloud без загрузки, OneDrive «по запросу»): его чтение
/// скачало бы файл целиком — такие не сравниваем.
fn cloud_placeholder(m: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // RECALL_ON_DATA_ACCESS | RECALL_ON_OPEN | OFFLINE
        return m.file_attributes() & (0x0040_0000 | 0x0004_0000 | 0x1000) != 0;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        return m.st_flags() & 0x4000_0000 != 0; // SF_DATALESS
    }
    #[allow(unreachable_code)]
    {
        let _ = m;
        false
    }
}

/// Один и тот же файл на диске (жёсткая ссылка): не «копия» — удаление ничего не освободит.
fn same_inode(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    let _ = (a, b);
    false
}

/// Папка загрузок: из настроек системы (XDG_DOWNLOAD_DIR), иначе ~/Downloads.
fn downloads_dir() -> Option<PathBuf> {
    let h = crate::store::home_dir()?;
    let dirs = crate::store::read_text_lossy(h.join(".config/user-dirs.dirs")).unwrap_or_default();
    let xdg = dirs
        .lines()
        .find(|l| l.starts_with("XDG_DOWNLOAD_DIR="))
        .and_then(|l| l.split('"').nth(1))
        .map(|p| PathBuf::from(p.replace("$HOME", &h.to_string_lossy())));
    xdg.filter(|p| *p != h && p.is_dir()).or_else(|| Some(h.join("Downloads")).filter(|p| p.is_dir()))
}

/// md5 первых и последних 64 КБ (быстрый отсев).
fn quick_hash(p: &Path, size: u64) -> Option<[u8; 16]> {
    // только обычный файл (FIFO, подложенный вместо файла, не повесит поиск)
    let mut f = crate::store::open_regular(p).ok()?;
    let mut buf = vec![0u8; 65536];
    let mut ctx = md5::Context::new();
    let n = f.read(&mut buf).ok()?;
    ctx.consume(&buf[..n]);
    if size > 131072 {
        f.seek(SeekFrom::End(-65536)).ok()?;
        let n = f.read(&mut buf).ok()?;
        ctx.consume(&buf[..n]);
    }
    Some(ctx.compute().0)
}

fn full_hash(p: &Path, deadline: Instant) -> Option<[u8; 16]> {
    let mut f = crate::store::open_regular(p).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    let mut ctx = md5::Context::new();
    loop {
        if Instant::now() > deadline {
            return None;
        }
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        ctx.consume(&buf[..n]);
    }
    Some(ctx.compute().0)
}

/// Какой файл оставить: не из «Загрузок», потом самый старый, потом с коротким путём.
fn keep_order(a: &(PathBuf, f64), b: &(PathBuf, f64), downloads: &[PathBuf]) -> std::cmp::Ordering {
    let in_dl = |p: &Path| downloads.iter().any(|d| p.starts_with(d));
    in_dl(&a.0)
        .cmp(&in_dl(&b.0))
        .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .then(a.0.as_os_str().len().cmp(&b.0.as_os_str().len()))
        // последний ключ — сам путь: какой файл останется, не зависит от порядка чтения папки
        .then(a.0.cmp(&b.0))
}

/// Найти копии. Не дольше `budget`; второй элемент — успели ли пройти всё.
pub fn scan(roots: &[PathBuf], budget: Duration) -> (Vec<Group>, bool) {
    let deadline = Instant::now() + budget;
    let (files, mut complete) = walk(roots, deadline);
    let mut by_size: HashMap<u64, Vec<(PathBuf, f64)>> = HashMap::new();
    for (p, s, t) in files {
        by_size.entry(s).or_default().push((p, t));
    }
    let downloads: Vec<PathBuf> = downloads_dir().into_iter().collect();
    let mut groups = Vec::new();
    for (size, list) in by_size.into_iter().filter(|(_, l)| l.len() > 1) {
        let mut by_quick: HashMap<[u8; 16], Vec<(PathBuf, f64)>> = HashMap::new();
        for (p, t) in list {
            if Instant::now() > deadline {
                complete = false;
                break;
            }
            if let Some(h) = quick_hash(&p, size) {
                by_quick.entry(h).or_default().push((p, t));
            }
        }
        for (_, l2) in by_quick.into_iter().filter(|(_, l)| l.len() > 1) {
            let mut by_full: HashMap<[u8; 16], Vec<(PathBuf, f64)>> = HashMap::new();
            for (p, t) in l2 {
                match full_hash(&p, deadline) {
                    Some(h) => by_full.entry(h).or_default().push((p, t)),
                    None => complete = false,
                }
            }
            for (_, mut same) in by_full.into_iter().filter(|(_, l)| l.len() > 1) {
                same.sort_by(|a, b| keep_order(a, b, &downloads));
                // жёсткие ссылки на тот же файл — не копии: оставляем одно имя на каждый настоящий файл
                let mut uniq: Vec<(PathBuf, f64)> = Vec::new();
                for x in same {
                    if !uniq.iter().any(|u| same_inode(&u.0, &x.0)) {
                        uniq.push(x);
                    }
                }
                let same = uniq;
                if same.len() < 2 {
                    continue;
                }
                groups.push(Group { size, files: same.into_iter().map(|x| x.0).collect() });
            }
        }
    }
    // порядок постоянный: при равном размере — по пути (раньше одинаковые группы менялись местами)
    groups.sort_by(|a, b| b.waste().cmp(&a.waste()).then_with(|| a.files.first().cmp(&b.files.first())));
    (groups, complete)
}

/// В Корзину средствами системы: Linux — gio trash, Windows — Корзина через Shell (Microsoft.VisualBasic),
/// macOS — Finder (osascript).
fn to_trash(path: &str) -> bool {
    // путь передаётся отдельно (аргументом / структурой), а не вклеивается в текст команды:
    // имя файла с кавычками не выполнится как команда.
    let moved = {
        #[cfg(windows)]
        {
            win_recycle(path)
        }
        #[cfg(not(windows))]
        {
            // не дольше минуты: Finder или Корзина на сетевом диске могут зависнуть
            let t = std::time::Duration::from_secs(60);
            let r = if cfg!(target_os = "macos") {
                crate::cmd::exec(
                    &[
                        "osascript",
                        "-e",
                        "on run argv",
                        "-e",
                        "tell application \"Finder\" to delete POSIX file (item 1 of argv)",
                        "-e",
                        "end run",
                        path,
                    ],
                    t,
                    crate::cmd::FIX,
                )
            } else {
                crate::cmd::exec(&["gio", "trash", "--", path], t, crate::cmd::FIX)
            };
            r.code() == 0
        }
    };
    // «ушёл» — только если файла по старому пути действительно нет
    moved && !Path::new(path).exists()
}

/// Windows: в Корзину через SHFileOperationW с FOF_ALLOWUNDO. FOF_WANTNUKEWARNING: если файл в Корзину не
/// помещается (мала квота, сетевой или съёмный диск), Windows спросит, удалить ли его насовсем, — а не удалит
/// молча (так делал прежний способ: DeleteFile с OnlyErrorDialogs).
#[cfg(windows)]
fn win_recycle(path: &str) -> bool {
    use core::ffi::c_void;
    #[repr(C)]
    struct FileOp {
        hwnd: *mut c_void,
        func: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        aborted: i32,
        mappings: *mut c_void,
        title: *const u16,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut FileOp) -> i32;
    }
    // список путей — с двумя нулями в конце
    let mut from: Vec<u16> = path.encode_utf16().collect();
    from.extend([0, 0]);
    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_WANTNUKEWARNING: u16 = 0x4000;
    let mut op = FileOp {
        hwnd: std::ptr::null_mut(),
        func: FO_DELETE,
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_WANTNUKEWARNING,
        aborted: 0,
        mappings: std::ptr::null_mut(),
        title: std::ptr::null(),
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    r == 0 && op.aborted == 0
}

// ───────────────────────────── Кэш результата ─────────────────────────────

fn cache_path() -> PathBuf {
    // результат — данные измерения: лежит в папке сеанса (исчезает при выходе из системы), не в кэше на диске
    crate::store::runtime_dir().join("dupes.json")
}

pub fn save(groups: &[Group], complete: bool) {
    let j = serde_json::json!({
        "time": std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "complete": complete,
        "groups": groups.iter().map(|g| serde_json::json!({
            "size": g.size,
            "files": g.files.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let p = cache_path();
    // атомарно, только для владельца (в списке — пути к его файлам)
    let _ = crate::store::write_private(&p, j.to_string().as_bytes());
}

/// Последний результат: (группы, где все файлы ещё на месте; время поиска; полный ли).
pub fn load() -> Option<(Vec<Group>, u64, bool)> {
    let j: serde_json::Value = serde_json::from_str(&crate::store::read_capped(&cache_path(), 16 << 20).ok()?).ok()?;
    let roots: Vec<PathBuf> = roots().iter().filter_map(|r| std::fs::canonicalize(r).ok()).collect();
    let groups = j["groups"]
        .as_array()?
        .iter()
        .filter_map(|g| {
            let size = g["size"].as_u64()?;
            let files: Vec<PathBuf> =
                g["files"].as_array()?.iter().filter_map(|f| f.as_str().map(PathBuf::from)).filter(|p| allowed(p, &roots)).collect();
            (files.len() > 1).then_some(Group { size, files })
        })
        .collect();
    Some((groups, j["time"].as_u64().unwrap_or(0), j["complete"].as_bool().unwrap_or(true)))
}

fn human(b: u64) -> String {
    // округление, а не отбрасывание: 1,9 МБ — «2 МБ», а не «1 МБ»
    if b >= 1 << 30 {
        format!("{:.1} ГБ", b as f64 / (1u64 << 30) as f64)
    } else {
        format!("{:.0} МБ", b as f64 / (1u64 << 20) as f64)
    }
}

fn short(p: &Path) -> String {
    let h = crate::store::home_str();
    let s = p.to_string_lossy().to_string();
    if !h.is_empty() && s.starts_with(&h) {
        s.replacen(&h, "~", 1)
    } else {
        s
    }
}

// ───────────────────────────── Проверка и лечение ─────────────────────────────

pub(crate) fn check_dupes() -> Vec<Check> {
    let Some((groups, _, complete)) = load() else {
        return vec![Check::new("sys", "Копии файлов", INFO, "ещё не искал")
            .hint("Найду одинаковые файлы в Загрузках, Документах, Изображениях, Видео, Музыке и на Рабочем столе. Папки программных проектов не трогаю.")
            .fixes(vec![fx("dupes_scan", "Найти копии", None)])];
    };
    let total: u64 = groups.iter().map(Group::waste).sum();
    let mut out = Vec::new();
    if groups.is_empty() {
        // неполный поиск без находок — не «копий нет», а «не нашла в том, что успела посмотреть»
        out.push(if complete {
            Check::new("sys", "Копии файлов", OK, "копий нет").fixes(vec![fx("dupes_scan", "Поискать снова", None)])
        } else {
            Check::new("sys", "Копии файлов", INFO, "просмотрел не всё")
                .hint("Файлов много — за отведённое время просмотрел часть и копий в ней не нашёл.")
                .fixes(vec![fx("dupes_scan", "Поискать снова", None)])
        });
        return out;
    }
    // копии — не беда: только сведения, без тревоги; убираются лишь по кнопке
    out.push(
        Check::new(
            "sys",
            "Копии файлов",
            INFO,
            &format!("{} · лишнее {}", crate::diag::plural(groups.len(), "группа", "группы", "групп"), human(total)),
        )
        .hint(&format!(
            "Одинаковые по содержимому файлы. Сам ничего не удаляю: лишние копии уходят в Корзину только по кнопке, оттуда можно вернуть.{}",
            if complete {
                ""
            } else {
                " Искал не до конца (много файлов): часть файлов не просмотрена. «Поискать снова» начнёт поиск сначала."
            }
        ))
        .fixes(vec![
            // «все» — именно те группы, что были в списке, когда его показали (метка всего набора): если
            // список с тех пор поменялся, ничего не удалю
            fx(
                "dupes_trash",
                &format!("Все лишние ({}) — в корзину", crate::diag::plural(groups.len(), "группа", "группы", "групп")),
                Some(&format!("all:{}", set_id(&groups))),
            ),
            fx("dupes_scan", "Поискать снова", None),
        ]),
    );
    for g in groups.iter().take(SHOW) {
        let name = g.files[0].file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let others: Vec<String> = g.files[1..].iter().map(|p| short(p)).collect();
        out.push(
            Check::new(
                "sys",
                &format!("Копия: {name}"),
                INFO,
                &format!("{} · лишнее {}", crate::diag::plural(g.files.len(), "копия", "копии", "копий"), human(g.waste())),
            )
            .hint(&format!("Оставлю: {}. В корзину: {}.", short(&g.files[0]), others.join(", ")))
            .fixes(vec![fx("dupes_trash", "Лишние — в корзину", Some(&g.id()))]),
        );
    }
    out
}

/// Найти копии (кнопка): до 3 минут, результат — в кэш.
pub fn fix_scan() -> (bool, String) {
    let t = Instant::now();
    let r = roots();
    if r.is_empty() {
        // без папок «копий нет» было бы неправдой
        return (false, "Не нашёл твоих папок (Документы, Загрузки…) — искать негде.".into());
    }
    let (groups, complete) = scan(&r, Duration::from_secs(180));
    if crate::fixes::dry_run() {
        crate::fixes::log_dry("записал бы найденные копии в кэш");
    } else {
        save(&groups, complete);
    }
    let total: u64 = groups.iter().map(Group::waste).sum();
    let msg = if groups.is_empty() {
        format!("Копий не нашёл (искал {:.0} с{}).", t.elapsed().as_secs_f64(), if complete { "" } else { ", успел посмотреть не всё" })
    } else {
        format!(
            "Нашёл {} копий, лишнее — {}. Смотри вкладку «Система».",
            crate::diag::plural(groups.len(), "группу", "группы", "групп"),
            human(total)
        )
    };
    (true, msg)
}

/// Лишние копии всех групп — в Корзину (в каждой группе первый файл остаётся).
/// Метка всего набора групп (из меток групп).
fn set_id(groups: &[Group]) -> String {
    let mut ids: Vec<String> = groups.iter().map(Group::id).collect();
    ids.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in ids.join(",").bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Все лишние копии — в Корзину. `expect` — метка набора, который видел человек; другой набор — отказ.
pub fn fix_trash_all(expect: &str) -> (bool, String) {
    let Some((groups, _, complete)) = load() else { return (false, "Сначала найди копии.".into()) };
    if set_id(&groups) != expect {
        return (false, "Список копий изменился — посмотри его ещё раз, я ничего не трогал.".into());
    }
    let (mut moved, mut want, mut freed, mut skipped) = (0, 0, 0u64, 0usize);
    let mut rest = Vec::new();
    let mut skipped_all = 0usize;
    for g in &groups {
        let mut ok = 0;
        for p in &g.files[1..] {
            let ps = p.to_string_lossy().to_string();
            if !still_copy(&g.files[0], p, g.size) {
                crate::fixes::log(&format!("не копия (изменилась или пропала) — не трогаю: {ps}"));
                skipped += 1;
                continue;
            }
            if crate::fixes::dry_run() {
                crate::fixes::log_dry(&format!("в корзину: {ps}"));
                ok += 1;
            } else if to_trash(&ps) {
                ok += 1;
            }
        }
        want += g.files.len() - 1;
        want -= skipped.min(g.files.len() - 1);
        skipped_all += skipped;
        skipped = 0;
        moved += ok;
        if ok == g.files.len() - 1 {
            freed += g.waste()
        } else {
            rest.push(g.clone())
        }
    }
    if !crate::fixes::dry_run() {
        save(&rest, complete);
    }
    let all = moved == want;
    // изменившиеся после поиска файлы не трогаем — это не ошибка, а осторожность; скажем об этом
    let note = match skipped_all {
        0 => String::new(),
        1 => " 1 файл изменился после поиска — его не тронул.".to_string(),
        n => format!(" {} изменились после поиска — их не тронул.", crate::diag::plural(n, "файл", "файла", "файлов")),
    };
    (
        all,
        if all {
            format!("{} в Корзине, освободится {} после её очистки.{note}", crate::diag::plural(moved, "копия", "копии", "копий"), human(freed))
        } else {
            format!("В корзину ушло {moved} из {want}.{note}")
        },
    )
}

/// Копия всё ещё та же, что оставляемый файл: тот на месте, размер и содержимое совпадают, и это не
/// жёсткая ссылка на него. Список копий взят из прошлого поиска — за это время файлы могли поменяться.
fn still_copy(keep: &Path, p: &Path, size: u64) -> bool {
    // оба файла — те же, что были (номер файла, размер, время), до и после сравнения; сравнение — побайтовое,
    // а не по MD5 (хеш годится для отбора кандидатов, но удаление решает только точное совпадение байтов)
    let (k0, p0) = (ident(keep), ident(p));
    let (Some(k0), Some(p0)) = (k0, p0) else { return false };
    if k0.2 != size || p0.2 != size || same_inode(keep, p) {
        return false;
    }
    let far = Instant::now() + Duration::from_secs(600);
    same_bytes(keep, p, far) && ident(keep) == Some(k0) && ident(p) == Some(p0)
}

/// Два файла совпадают байт в байт (читаем кусками, не дольше `until`; ошибка или таймаут — «не совпадают»).
fn same_bytes(a: &Path, b: &Path, until: Instant) -> bool {
    use std::io::Read;
    let (Ok(mut fa), Ok(mut fb)) = (crate::store::open_regular(a), crate::store::open_regular(b)) else { return false };
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        if Instant::now() > until {
            return false;
        }
        let na = match read_full(&mut fa, &mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let nb = match read_full(&mut fb, &mut bb) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
    fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0;
        while n < buf.len() {
            match f.read(&mut buf[n..])? {
                0 => break,
                k => n += k,
            }
        }
        Ok(n)
    }
}

/// (устройство, номер файла, размер, время изменения) — без перехода по ссылке.
fn ident(p: &Path) -> Option<(u64, u64, u64, i128)> {
    let m = std::fs::symlink_metadata(p).ok().filter(|m| m.is_file())?;
    let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as i128;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((m.dev(), m.ino(), m.len(), t))
    }
    #[cfg(not(unix))]
    Some((0, 0, m.len(), t))
}

/// Путь из кэша годится для удаления: обычный файл (не ссылка), настоящий путь — внутри одной из папок
/// поиска, не в скрытой, не в проекте и не в облачной папке (те же правила, что при поиске). Кэш —
/// файл на диске, его могли подправить: слепо ему не верим.
fn allowed(p: &Path, roots: &[PathBuf]) -> bool {
    if !std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()) {
        return false;
    }
    let Ok(real) = std::fs::canonicalize(p) else { return false };
    let Some(root) = roots.iter().find(|r| real.starts_with(r)) else { return false };
    let rel = real.strip_prefix(root).unwrap_or(&real);
    if rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
        return false;
    }
    let mut d = real.parent();
    while let Some(dir) = d {
        if !dir.starts_with(root) {
            break;
        }
        if is_project(dir) || is_synced(dir) {
            return false;
        }
        d = dir.parent();
    }
    true
}

/// Лишние копии группы i — в Корзину (gio trash), первый файл остаётся.
pub fn fix_trash(id: &str) -> (bool, String) {
    let Some((groups, _, complete)) = load() else { return (false, "Сначала найди копии.".into()) };
    // группу ищем по её коду (не по номеру строки); не нашли — список поменялся, ничего не трогаем
    let Some(i) = groups.iter().position(|g| g.id() == id) else {
        return (false, "Список копий изменился — найди копии заново, я ничего не трогал.".into());
    };
    let g = &groups[i];
    let mut moved = 0;
    let mut left: Vec<PathBuf> = vec![g.files[0].clone()];
    for p in &g.files[1..] {
        let ps = p.to_string_lossy().to_string();
        if !still_copy(&g.files[0], p, g.size) {
            crate::fixes::log(&format!("не копия (изменилась или пропала) — не трогаю: {ps}"));
            continue;
        }
        if crate::fixes::dry_run() {
            crate::fixes::log_dry(&format!("gio trash {ps}"));
            moved += 1;
            continue;
        }
        if to_trash(&ps) {
            moved += 1;
        } else {
            left.push(p.clone());
        }
    }
    // в кэше остаётся то, что не ушло (частичная неудача — группа не «разобрана»), полнота поиска — как была
    if !crate::fixes::dry_run() {
        let mut rest = groups.clone();
        if left.len() > 1 {
            rest[i] = Group { size: g.size, files: left };
        } else {
            rest.remove(i);
        }
        save(&rest, complete);
    }
    let ok = moved == g.files.len() - 1;
    (
        ok,
        if ok {
            format!("{} в Корзине, освободится {} после её очистки.", crate::diag::plural(moved, "копия", "копии", "копий"), human(g.waste()))
        } else {
            format!("В корзину ушло {moved} из {}.", g.files.len() - 1)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_copies_and_skips_projects() {
        let t = std::env::temp_dir().join(format!("aptechka-dupes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (dl, docs, proj) = (t.join("Загрузки"), t.join("Документы"), t.join("Документы/код"));
        for d in [&dl, &docs, &proj] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(proj.join("Cargo.toml"), "x").unwrap();
        let big = vec![7u8; 2 << 20];
        let mut other = big.clone();
        other[1 << 20] = 1; // тот же размер, другое содержимое в середине
        std::fs::write(docs.join("фото.jpg"), &big).unwrap();
        std::fs::write(dl.join("фото (1).jpg"), &big).unwrap();
        std::fs::write(dl.join("похоже.jpg"), &other).unwrap();
        std::fs::write(proj.join("копия.jpg"), &big).unwrap(); // в проекте — не трогаем
        let (g, complete) = scan(&[dl.clone(), docs.clone()], Duration::from_secs(10));
        assert!(complete);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].files.len(), 2);
        assert_eq!(g[0].files[0], docs.join("фото.jpg")); // оставляем не из «Загрузок»
        let _ = std::fs::remove_dir_all(&t);
    }
}
