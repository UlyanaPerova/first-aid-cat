//! Разделы дисков (Linux): место, иноды, «только для чтения» — напрямую у ядра (statvfs), без программы df.
//!
//! df обходил все подключения, в том числе отвалившийся сетевой диск, и по таймауту не было данных ни по
//! одному разделу. Теперь: список — из /proc/self/mounts (только файловые системы настоящих дисков),
//! каждый раздел спрашивается в своём потоке не дольше 2 с.

use std::time::Duration;

/// Сведения о разделе.
#[derive(Debug, Clone)]
pub struct Fs {
    pub dev: String,
    pub target: String,
    pub avail: f64,
    pub size: f64,
    /// доля свободных инодов (0..1); None — у этой ФС инодов нет (btrfs, vfat)
    pub inodes_free: Option<f64>,
    /// смонтирован только для чтения
    pub ro: bool,
}

/// Файловые системы настоящих дисков (без памяти, снимков snap, сетевых и виртуальных).
const REAL: [&str; 11] = ["ext4", "ext3", "ext2", "btrfs", "xfs", "f2fs", "zfs", "vfat", "exfat", "ntfs3", "fuseblk"];

// описание struct statvfs ниже — для 64-битного glibc (x86_64, aarch64); на других сборках — без него
#[cfg(all(target_os = "linux", target_pointer_width = "64", target_env = "gnu"))]
fn statvfs(path: &str) -> Option<(f64, f64, Option<f64>, bool)> {
    // struct statvfs у glibc на 64-битных Linux: все поля — unsigned long, плюс запас
    #[repr(C)]
    struct Sv {
        bsize: u64,
        frsize: u64,
        blocks: u64,
        bfree: u64,
        bavail: u64,
        files: u64,
        ffree: u64,
        favail: u64,
        fsid: u64,
        flag: u64,
        namemax: u64,
        spare: [i32; 6],
    }
    extern "C" {
        fn statvfs(path: *const std::ffi::c_char, buf: *mut Sv) -> i32;
    }
    let c = std::ffi::CString::new(path).ok()?;
    let mut sv: Sv = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(c.as_ptr(), &mut sv) } != 0 {
        return None;
    }
    let unit = if sv.frsize > 0 { sv.frsize } else { sv.bsize } as f64;
    let inodes = (sv.files > 0).then(|| sv.favail as f64 / sv.files as f64);
    // ST_RDONLY = 1
    Some((sv.bavail as f64 * unit, sv.blocks as f64 * unit, inodes, sv.flag & 1 == 1))
}

#[cfg(not(all(target_os = "linux", target_pointer_width = "64", target_env = "gnu")))]
fn statvfs(_: &str) -> Option<(f64, f64, Option<f64>, bool)> {
    None
}

/// «\040» в /proc/mounts — пробел (и другие восьмеричные коды).
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Все разделы настоящих дисков. Не ответивший за 2 с (отвалившийся диск) пропускается.
pub fn list() -> Vec<Fs> {
    let mounts = crate::store::read_text_lossy("/proc/self/mounts").unwrap_or_default();
    let mut cand: Vec<(String, String)> = Vec::new();
    for l in mounts.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        let (Some(dev), Some(target), Some(fs)) = (f.first(), f.get(1), f.get(2)) else { continue };
        let target = unescape(target);
        if !REAL.contains(fs) || target.starts_with("/snap") || cand.iter().any(|c| c.0 == *dev) {
            continue;
        }
        cand.push((dev.to_string(), target));
    }
    // раздел, который в прошлый раз так и не ответил (отвалившийся сетевой диск), второй раз не спрашиваем,
    // пока не ответит первый вопрос: иначе каждые 30 с копился бы ещё один застрявший поток
    static WAITING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let (tx, rx) = std::sync::mpsc::channel();
    for (i, (_, t)) in cand.iter().enumerate() {
        {
            let mut w = WAITING.lock().unwrap_or_else(|e| e.into_inner());
            if w.contains(t) {
                continue;
            }
            w.push(t.clone());
        }
        let (tx, t) = (tx.clone(), t.clone());
        std::thread::spawn(move || {
            let r = statvfs(&t);
            WAITING.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| *x != t);
            let _ = tx.send((i, r));
        });
    }
    drop(tx);
    let until = std::time::Instant::now() + Duration::from_secs(2);
    let mut got: Vec<Option<(f64, f64, Option<f64>, bool)>> = vec![None; cand.len()];
    while let Ok((i, r)) = rx.recv_timeout(until.saturating_duration_since(std::time::Instant::now())) {
        got[i] = r;
    }
    cand.into_iter()
        .zip(got)
        .filter_map(|((dev, target), r)| {
            let (avail, size, inodes_free, ro) = r?;
            (size > 0.0).then_some(Fs { dev, target, avail, size, inodes_free, ro })
        })
        .collect()
}

/// Сведения о разделе, где лежит путь (самая длинная точка подключения — начало пути).
pub fn for_path<'a>(all: &'a [Fs], p: &str) -> Option<&'a Fs> {
    all.iter().filter(|f| std::path::Path::new(p).starts_with(&f.target)).max_by_key(|f| f.target.len())
}

// ───────────────────────────── Здоровье дисков ─────────────────────────────

/// Здоровье физических дисков (S.M.A.R.T.) без пароля: Linux — udisks2 по D-Bus (NVMe: критические
/// предупреждения; SATA: «скоро откажет», плохие секторы), macOS — diskutil (SMART Status), Windows —
/// Get-PhysicalDisk (HealthStatus). Диск не сообщает здоровье — строки нет.
pub fn check_health() -> Vec<crate::diag::Check> {
    // WARN нужен Linux и Windows; у macOS diskutil знает только «в порядке» и «откажет»
    #[cfg_attr(target_os = "macos", allow(unused_imports))]
    use crate::diag::{Check, BAD, OK, WARN};
    // (модель, беда / предупреждение / в порядке, пояснение)
    let mut drives: Vec<(String, &'static str, String)> = Vec::new();
    #[cfg(target_os = "linux")]
    {
        let tree_r = crate::diag::run_ex(&["busctl", "--system", "--list", "tree", "org.freedesktop.UDisks2"], 8);
        // не ответил вовремя / не запустился — «не проверить», а не «дисков нет». Нет службы UDisks2 (код ошибки) — у системы
        // просто нет этой проверки, строки не будет, как и раньше
        if matches!(tree_r, crate::cmd::Outcome::Timeout { .. } | crate::cmd::Outcome::SpawnError(_)) {
            return vec![Check::new("sys", "Здоровье дисков", crate::diag::INFO, "не удалось проверить")
                .unknown()
                .hint("Служба дисков (UDisks2) не ответила вовремя.")];
        }
        let tree = tree_r.text().to_string();
        for path in tree.lines().map(str::trim).filter(|l| l.starts_with("/org/freedesktop/UDisks2/drives/")) {
            let get = |iface: &str, prop: &str| {
                let r = crate::cmd::exec(
                    &["busctl", "--system", "get-property", "org.freedesktop.UDisks2", path, iface, prop],
                    Duration::from_secs(5),
                    crate::cmd::CHECK,
                );
                (r.code() == 0).then(|| r.text().trim().to_string())
            };
            // CD/DVD (в том числе виртуальный у виртуальной машины) — не диск с данными, его «секторы» не про здоровье
            if get("org.freedesktop.UDisks2.Drive", "MediaCompatibility").is_some_and(|m| m.contains("optical")) {
                continue;
            }
            let model =
                get("org.freedesktop.UDisks2.Drive", "Model").map(|m| m.trim_start_matches("s ").trim_matches('"').to_string()).unwrap_or_default();
            let model = if model.is_empty() { path.rsplit('/').next().unwrap_or("диск").replace('_', " ") } else { model };
            // данные S.M.A.R.T. хоть раз прочитаны (SmartUpdated — время в секундах; 0 — никогда): иначе
            // «предупреждений нет» — просто значения по умолчанию, и «в порядке» было бы неправдой
            let updated = |iface: &str| get(iface, "SmartUpdated").and_then(|v| v.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0) > 0;
            // NVMe: «as 0» — предупреждений нет; иначе список: spare, temperature, degraded, readonly, volatile_mem
            if !updated("org.freedesktop.UDisks2.NVMe.Controller") && !updated("org.freedesktop.UDisks2.Drive.Ata") {
                continue;
            }
            if let Some(w) = get("org.freedesktop.UDisks2.NVMe.Controller", "SmartCriticalWarning") {
                let list: Vec<String> = w.split('"').skip(1).step_by(2).map(String::from).collect();
                if list.is_empty() {
                    drives.push((model, OK, String::new()));
                } else {
                    let what: Vec<&str> = list
                        .iter()
                        .map(|x| match x.as_str() {
                            "spare" => "кончается запас ячеек",
                            "temperature" => "перегрев",
                            "degraded" => "надёжность снижена",
                            "readonly" => "перешёл в «только чтение»",
                            "volatile_mem" => "сбой резервной памяти",
                            _ => "предупреждение",
                        })
                        .collect();
                    let bad = list.iter().any(|x| matches!(x.as_str(), "degraded" | "readonly" | "spare"));
                    drives.push((model, if bad { BAD } else { WARN }, what.join(", ")));
                }
                continue;
            }
            // SATA/ATA: «b true» — сам диск говорит, что скоро откажет; плохие секторы — тревожный признак
            if let Some(f) = get("org.freedesktop.UDisks2.Drive.Ata", "SmartFailing") {
                let bad_sectors: i64 = get("org.freedesktop.UDisks2.Drive.Ata", "SmartNumBadSectors")
                    .and_then(|v| v.split_whitespace().nth(1)?.parse().ok())
                    .unwrap_or(0);
                if f.ends_with("true") {
                    drives.push((model, BAD, "диск сообщает, что скоро откажет".into()));
                } else if bad_sectors > 0 {
                    drives.push((model, WARN, format!("плохих секторов: {bad_sectors}")));
                } else if get("org.freedesktop.UDisks2.Drive.Ata", "SmartSupported").is_some_and(|v| v.ends_with("true")) {
                    drives.push((model, OK, String::new()));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let list = crate::diag::run(&["diskutil", "list", "physical"], 10).1;
        for disk in list.lines().filter(|l| l.starts_with("/dev/disk")).filter_map(|l| l.split_whitespace().next()) {
            let info = crate::diag::run(&["diskutil", "info", disk], 10).1;
            let get = |k: &str| info.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim_start_matches(':').trim().to_string()));
            let model = get("Device / Media Name").unwrap_or_else(|| disk.to_string());
            match get("SMART Status").as_deref() {
                Some("Verified") => drives.push((model, OK, String::new())),
                Some("Failing") => drives.push((model, BAD, "диск сообщает, что скоро откажет".into())),
                _ => {}
            }
        }
    }
    #[cfg(windows)]
    {
        for d in crate::win::ps("Get-PhysicalDisk | Select-Object FriendlyName,HealthStatus", 30) {
            let name = d["FriendlyName"].as_str().unwrap_or("диск").to_string();
            match d["HealthStatus"].as_str().or_else(|| {
                d["HealthStatus"].as_i64().map(|n| {
                    if n == 0 {
                        "Healthy"
                    } else if n == 1 {
                        "Warning"
                    } else {
                        "Unhealthy"
                    }
                })
            }) {
                Some("Healthy") => drives.push((name, OK, String::new())),
                Some("Warning") => drives.push((name, WARN, "Windows предупреждает о его здоровье".into())),
                Some("Unhealthy") => drives.push((name, BAD, "Windows считает его неисправным".into())),
                _ => {}
            }
        }
    }
    let troubled: Vec<&(String, &str, String)> = drives.iter().filter(|d| d.1 != OK).collect();
    if drives.is_empty() {
        return Vec::new();
    }
    if troubled.is_empty() {
        return vec![Check::new("sys", "Здоровье дисков", OK, &format!("в порядке ({})", drives.len()))
            .hint(&format!("Сами диски не сообщают о неполадках: {}.", drives.iter().map(|d| d.0.as_str()).collect::<Vec<_>>().join(", ")))];
    }
    troubled
        .into_iter()
        .map(|(m, st, why)| {
            Check::new("sys", &format!("Здоровье диска {m}"), st, why)
                .hint("Диск сам сообщает о неполадке. Сделай копию важных файлов как можно скорее и подумай о замене диска.")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn mounts_unescape() {
        assert_eq!(super::unescape("/media/a\\040b"), "/media/a b");
        assert_eq!(super::unescape("/plain"), "/plain");
        assert_eq!(super::unescape("/x\\"), "/x\\");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn root_has_size() {
        let all = super::list();
        assert!(super::for_path(&all, "/").is_some_and(|f| f.size > 0.0) || all.is_empty());
    }
}
