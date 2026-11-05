#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
//! Запуск с правами администратора — один путь для всех починок, с наименьшими правами.
//!
//! Linux (pkexec):
//! - программа — только по абсолютному пути из системных папок (/usr/bin, /usr/sbin, /bin, /sbin), и только
//!   если она и все папки над ней принадлежат root и не доступны на запись другим: подложенная в PATH или
//!   в домашнюю папку «systemctl» с правами root не запустится;
//! - Аптечку (её файл лежит в домашней папке и доступен на запись пользователю) с правами root не запускаем
//!   никогда — иначе любая программа пользователя, подменив этот файл, получила бы root;
//! - лёгкая песочница systemd-run (если есть): домашние папки только для чтения (root-починка не может
//!   ничего записать или удалить в домашней папке — подмена ссылкой туда ничего не даст), свой /tmp,
//!   запрет новых привилегий (setuid-программы внутри не поднимут права), для служебных починок — ещё и без
//!   загрузки модулей ядра и без изменения его настроек. Для служебных починок ещё и вся файловая система
//!   только для чтения (кроме журнала) и урезанный набор возможностей (capabilities). Установка пакетов
//!   (apt, dpkg) песочницу получает мягче: им нужно писать в /usr, /var, /etc, /lib/modules и запускать
//!   скрипты пакетов (а те бывают какими угодно), поэтому ProtectSystem и урезание возможностей там не стоят.
//!   Окно pkexec показывает путь `systemd-run`, а не само действие: настоящее название действия polkit
//!   берёт из своего файла политик, который ставится в систему при установке пакета, — из программы его не
//!   подменить. Точная команда показана пользователю в окне Аптечки (manual()).
//!
//! Windows: команда PowerShell передаётся закодированной (-EncodedCommand), а не склеенной строкой —
//! кавычки в данных (в том числе «типографские») не превратятся в код.
//! macOS: скрипт передаётся AppleScript аргументом, а не вклеивается в текст AppleScript.

use std::path::{Path, PathBuf};

/// Насколько строгая песочница.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Profile {
    /// службы, часы, DNS, журнал, Bluetooth, принтер: почти всё закрыто
    Service,
    /// установка и удаление пакетов, драйвер видеокарты: пишут в систему, запускают скрипты пакетов
    Packages,
}

/// Свойства песочницы systemd для профиля.
fn sandbox(p: Profile) -> &'static [&'static str] {
    const COMMON: [&str; 6] =
        ["ProtectHome=read-only", "PrivateTmp=yes", "LockPersonality=yes", "RestrictRealtime=yes", "UMask=0022", "KeyringMode=private"];
    const SERVICE: [&str; 15] = [
        "ProtectHome=read-only",
        "PrivateTmp=yes",
        "LockPersonality=yes",
        "RestrictRealtime=yes",
        "UMask=0022",
        "KeyringMode=private",
        "NoNewPrivileges=yes",
        "RestrictSUIDSGID=yes",
        "ProtectKernelModules=yes",
        "ProtectKernelLogs=yes",
        "ProtectControlGroups=yes",
        "SystemCallArchitectures=native",
        // вся файловая система только для чтения (/dev, /proc, /sys и так особые): службы, часы, DNS, Bluetooth
        // и принтер меняются через D-Bus и сокеты (подключиться к сокету на «только чтении» можно), сами файлы
        // не пишут. Исключение — журнал (journalctl --vacuum удаляет там файлы); «-» — если папки нет, не беда
        "ProtectSystem=strict",
        "ReadWritePaths=-/var/log/journal -/run/log/journal",
        // ни одной из этих возможностей служебным починкам не нужно: загрузка модулей, перезагрузка и выключение
        // самим процессом, отладка чужих процессов, прямой доступ к портам и памяти, смена системного времени
        // напрямую (timedatectl просит об этом службу timedated), создание файлов-устройств и журнал аудита (старые имена возможностей, поэтому старые systemd понимают список)
        "CapabilityBoundingSet=~CAP_SYS_MODULE CAP_SYS_BOOT CAP_SYS_PTRACE CAP_SYS_RAWIO CAP_SYS_TIME CAP_MKNOD CAP_SYS_PACCT CAP_LINUX_IMMUTABLE CAP_AUDIT_CONTROL CAP_AUDIT_WRITE",
    ];
    match p {
        Profile::Service => &SERVICE,
        Profile::Packages => &COMMON,
    }
}

/// Системная программа по имени: абсолютный путь, если файл и папки над ним — root и не доступны на запись
/// группе и остальным. Иначе None (не запускаем с правами root то, что мог подменить не-root).
pub fn trusted_binary(name: &str) -> Option<PathBuf> {
    if name.contains('/') || name.is_empty() {
        return None;
    }
    for dir in ["/usr/bin", "/usr/sbin", "/bin", "/sbin"] {
        let p = Path::new(dir).join(name);
        // проверяем и настоящий путь (bin → usr/bin на современных системах), и саму ссылку с её папками,
        // а запускаем по исходному имени: часть программ решает, что делать, по имени запуска
        // (cupsenable — ссылка на cupsaccept; запуск по настоящему пути делал бы не то)
        let Ok(real) = std::fs::canonicalize(&p) else { continue };
        if root_owned_chain(&real) && link_root_owned(&p) {
            return Some(p);
        }
    }
    None
}

#[cfg(unix)]
fn root_owned_chain(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let safe = |q: &Path| std::fs::metadata(q).is_ok_and(|m| m.uid() == 0 && m.mode() & 0o022 == 0);
    safe(p) && p.ancestors().skip(1).all(safe)
}

/// Сама запись в папке (ссылка или файл) и папки над ней — root и не открыты на запись другим:
/// иначе ссылку подменили бы между проверкой и запуском.
#[cfg(unix)]
fn link_root_owned(p: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let parent_ok = p.parent().and_then(|d| std::fs::canonicalize(d).ok()).is_some_and(|d| root_owned_chain(&d));
    parent_ok && std::fs::symlink_metadata(p).is_ok_and(|m| m.uid() == 0 && (m.file_type().is_symlink() || m.mode() & 0o022 == 0))
}

#[cfg(not(unix))]
fn link_root_owned(_: &Path) -> bool {
    false
}

#[cfg(not(unix))]
fn root_owned_chain(_: &Path) -> bool {
    false
}

/// Аргумент годится для передачи программе с правами root: не пустой, без нулевого байта и перевода строки,
/// разумной длины. Смысловую проверку (имя службы, принтера) делает вызывающий.
fn arg_ok(a: &str) -> bool {
    !a.contains('\0') && !a.contains('\n') && a.len() <= 4096
}

/// Собрать команду для pkexec: [pkexec, (systemd-run …песочница… --,) /абсолютный/путь, аргументы…].
/// None — программа не найдена среди доверенных или аргумент недопустим.
#[cfg(target_os = "linux")]
pub fn linux_cmd(prog: &str, args: &[&str], p: Profile) -> Option<Vec<String>> {
    if !args.iter().all(|a| arg_ok(a)) {
        return None;
    }
    let bin = trusted_binary(prog)?;
    let pkexec = trusted_binary("pkexec")?;
    let mut c = vec![pkexec.to_string_lossy().into_owned()];
    if let Some(run) = trusted_binary("systemd-run") {
        c.push(run.to_string_lossy().into_owned());
        // --pipe: вывод — нам; --wait: код выхода — программы; --collect: не оставлять упавший блок
        c.extend(["--quiet", "--wait", "--pipe", "--collect", "--service-type=exec"].map(String::from));
        for prop in sandbox(p) {
            c.push("-p".into());
            c.push(prop.to_string());
        }
        c.push("--".into());
    }
    c.push(bin.to_string_lossy().into_owned());
    c.extend(args.iter().map(|a| a.to_string()));
    Some(c)
}

/// Windows: аргументы powershell для запуска команды без склейки строк (-EncodedCommand: UTF-16LE в base64).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn ps_encoded(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64(&bytes)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= ch.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Для тестов других модулей: base64 без настоящего кодировщика под рукой.
#[cfg(test)]
pub fn b64_for_test(b: &[u8]) -> String {
    base64(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        // «ls» в UTF-16LE: 6C 00 73 00
        assert_eq!(ps_encoded("ls"), "bABzAA==");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn trusted_only() {
        assert!(trusted_binary("sh").is_some());
        assert!(trusted_binary("../tmp/x").is_none());
        assert!(trusted_binary("нет-такой-программы").is_none());
        // ссылка запускается по своему имени: cupsenable → cupsaccept ведёт себя по имени запуска
        for name in ["cupsenable", "sh"] {
            if let Some(p) = trusted_binary(name) {
                assert!(p.ends_with(name), "{name} → {}", p.display());
            }
        }
        assert!(!arg_ok("a\nb"));
        // команда собирается из абсолютных путей
        if let Some(c) = linux_cmd("sh", &["-c", "true"], Profile::Service) {
            assert!(c.iter().any(|x| x == "ProtectSystem=strict"));
            assert!(c.iter().filter(|x| x.ends_with("/sh")).all(|x| x.starts_with('/')));
            assert!(c[0].starts_with('/'));
        }
    }
}
