//! Одна копия Аптечки на пользователя: вторая копия просит первую показать окно и выходит.
//! «Почтовый ящик»: Linux — абстрактный unix-сокет (в памяти ядра, без файлов: после падения исчезает сам);
//! macOS — unix-сокет в папке временных файлов пользователя; Windows — TCP на 127.0.0.1, порт от папки пользователя.
//! Вторая копия пишет «show <кто>», первая отвечает «aptechka», только если это тот же пользователь.

use std::io::{Read, Write};

#[cfg(target_os = "linux")]
mod imp {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
    pub type Listener = UnixListener;
    fn addr() -> std::io::Result<SocketAddr> {
        let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
        SocketAddr::from_abstract_name(format!("io.github.aptechka-{uid}"))
    }
    pub fn connect() -> std::io::Result<UnixStream> {
        UnixStream::connect_addr(&addr()?)
    }
    pub fn bind() -> std::io::Result<UnixListener> {
        UnixListener::bind_addr(&addr()?)
    }
    /// Чей процесс на том конце (SO_PEERCRED): имя абстрактного сокета может занять любой пользователь.
    pub fn same_user(s: &UnixStream) -> bool {
        use std::os::unix::io::AsRawFd;
        #[repr(C)]
        struct Ucred {
            pid: i32,
            uid: u32,
            gid: u32,
        }
        extern "C" {
            fn getsockopt(fd: i32, level: i32, name: i32, val: *mut core::ffi::c_void, len: *mut u32) -> i32;
        }
        let mut c = Ucred { pid: 0, uid: u32::MAX, gid: 0 };
        let mut len = std::mem::size_of::<Ucred>() as u32;
        // SOL_SOCKET = 1, SO_PEERCRED = 17
        let r = unsafe { getsockopt(s.as_raw_fd(), 1, 17, &mut c as *mut Ucred as *mut core::ffi::c_void, &mut len) };
        let me = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(u32::MAX - 1);
        r == 0 && c.uid == me
    }
    /// Пользователя сверяет само ядро (same_user).
    pub fn who() -> String {
        String::new()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::os::unix::net::{UnixListener, UnixStream};
    pub type Listener = UnixListener;
    fn path() -> std::path::PathBuf {
        std::env::temp_dir().join("io.github.aptechka.sock")
    }
    pub fn connect() -> std::io::Result<UnixStream> {
        UnixStream::connect(path())
    }
    pub fn bind() -> std::io::Result<UnixListener> {
        // файл от упавшей копии: если никто не отвечает — убираем и занимаем
        if connect().is_err() {
            let _ = std::fs::remove_file(path());
        }
        UnixListener::bind(path())
    }
    /// Сокет лежит в личной папке временных файлов пользователя — чужой туда не доберётся.
    pub fn same_user(_: &UnixStream) -> bool {
        true
    }
    pub fn who() -> String {
        String::new()
    }
}

#[cfg(windows)]
mod imp {
    use std::net::{TcpListener, TcpStream};
    pub type Listener = TcpListener;
    /// Кто мы: папка профиля (C:\Users\Имя) у каждого пользователя своя. Порт общий на всю машину
    /// (быстрое переключение пользователей, RDP) и может совпасть у двоих — тогда чужая копия на «show»
    /// с нашим профилем не ответит «aptechka», и мы работаем сами, а не молча выходим.
    pub fn who() -> String {
        std::env::var("USERPROFILE").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
    }
    fn port() -> u16 {
        let h = who().to_lowercase().bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
        47000 + (h % 2000) as u16
    }
    pub fn connect() -> std::io::Result<TcpStream> {
        TcpStream::connect_timeout(&([127, 0, 0, 1], port()).into(), std::time::Duration::from_millis(300))
    }
    pub fn bind() -> std::io::Result<TcpListener> {
        TcpListener::bind(("127.0.0.1", port()))
    }
    /// TCP не говорит, чей процесс: сверяем «кто» в самом запросе (см. who) и ответ «aptechka».
    pub fn same_user(_: &TcpStream) -> bool {
        true
    }
}

pub use imp::Listener;

/// Ok(Some(ящик)) — мы первая копия; Ok(None) — уже открыта другая, ей сказали «покажись».
pub fn claim() -> std::io::Result<Option<Listener>> {
    for _ in 0..2 {
        if let Ok(mut s) = imp::connect() {
            // настоящая Аптечка этого же пользователя отвечает «aptechka»; чужой процесс, занявший ящик,
            // не должен мешать запуску — тогда работаем без «одной копии»
            let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(1)));
            let _ = s.write_all(format!("show {}\n", imp::who()).as_bytes());
            if imp::same_user(&s) && read_line(&mut s).starts_with(b"aptechka") {
                return Ok(None);
            }
            return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "ящик занят чужим процессом"));
        }
        match imp::bind() {
            Ok(l) => return Ok(Some(l)),
            // ящик заняли между нашими двумя шагами — попробуем постучаться ещё раз
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

/// Слушать ящик в фоне: на каждое «покажись» вызвать `show`.
pub fn listen(l: Listener, show: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        // не чаще раза в секунду: поток «покажись» не будет дёргать окно без конца
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(5);
        for s in l.incoming().flatten() {
            if last.elapsed() < std::time::Duration::from_secs(1) {
                continue;
            }
            let mut s = s;
            let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(1)));
            if !imp::same_user(&s) {
                continue; // «покажись» от чужого пользователя не слушаем
            }
            let line = read_line(&mut s);
            if line.strip_prefix(b"show").is_some_and(|w| w.trim_ascii() == imp::who().as_bytes()) {
                let _ = s.write_all(b"aptechka\n");
                last = std::time::Instant::now();
                show();
            }
        }
    });
}

/// Одна строка запроса или ответа (до «\n», не больше 1 КБ; тайм-аут чтения задан заранее).
fn read_line(s: &mut impl Read) -> Vec<u8> {
    let mut out = Vec::new();
    let mut b = [0u8; 256];
    while out.len() < 1024 {
        match s.read(&mut b) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                out.extend_from_slice(&b[..n]);
                if out.contains(&b'\n') {
                    break;
                }
            }
        }
    }
    if let Some(i) = out.iter().position(|&c| c == b'\n') {
        out.truncate(i);
    }
    out
}
