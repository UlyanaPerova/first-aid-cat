//! Есть ли интернет и работают ли имена (DNS) — одна проверка для Linux, Windows и macOS.
//!
//! Раньше: на Linux — TCP до двух адресов IPv4 (сеть только с IPv6 выглядела «без интернета»), на Windows
//! и macOS — один запрос к одному сайту (он сам по себе недоступен — «интернета нет»); DNS — одно имя.
//! Теперь — несколько независимых путей сразу, параллельно, с повтором перед тем, как сказать «нет»:
//! - TCP до серверов двух стран (не зависит от DNS) и страницы проверки подключения систем (по имени —
//!   работает и в сети только с IPv6);
//! - DNS — несколько имён разных компаний; ответило хоть одно — имена работают.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Выход в интернет без DNS: TCP до Cloudflare и Яндекса (разные страны).
const TCP: [&str; 2] = ["1.1.1.1:443", "77.88.8.8:443"];

/// Страницы проверки подключения, которыми пользуются сами системы (Android/Chrome, Windows, macOS/iOS):
/// по имени, поэтому работают и в сети только с IPv6 (там DNS сам выдаёт адрес через NAT64).
/// (адрес, что должно прийти): код и кусок тела. Засчитываем только ожидаемый ответ — страница входа кафе
/// отвечает чем-то своим, и «интернет есть» было бы неправдой.
const HTTP: [(&str, u16, &str); 3] = [
    ("http://connectivitycheck.gstatic.com/generate_204", 204, ""),
    ("http://www.msftconnecttest.com/connecttest.txt", 200, "Microsoft Connect Test"),
    ("http://captive.apple.com/hotspot-detect.html", 200, "Success"),
];

/// Имена для проверки DNS — разные компании и страны (блокировка одного сайта — не поломка DNS).
const NAMES: [&str; 4] = ["one.one.one.one", "dns.google", "ya.ru", "example.com"];

/// Первый ответ из задач, запущенных параллельно; None — никто не ответил за `wait`.
fn first<T: Send + 'static>(jobs: Vec<Box<dyn FnOnce() -> Option<T> + Send>>, wait: Duration) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    let n = jobs.len();
    for j in jobs {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(j());
        });
    }
    let until = Instant::now() + wait;
    for _ in 0..n {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(Some(v)) => return Some(v),
            Ok(None) => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Выход в интернет напрямую: какой адрес ответил первым. Две попытки (сеть могла моргнуть).
pub fn internet() -> Option<&'static str> {
    for _ in 0..2 {
        let mut jobs: Vec<Box<dyn FnOnce() -> Option<&'static str> + Send>> = Vec::new();
        for a in TCP {
            jobs.push(Box::new(move || {
                let sa: SocketAddr = a.parse().ok()?;
                TcpStream::connect_timeout(&sa, Duration::from_secs(3)).ok().map(|_| a)
            }));
        }
        for (u, code, body) in HTTP {
            jobs.push(Box::new(move || {
                let (c, b, _) = crate::diag::http_get_noredirect(u, 4, 2000);
                (c == Some(code) && b.contains(body)).then_some(u)
            }));
        }
        if let Some(a) = first(jobs, Duration::from_secs(4)) {
            return Some(a);
        }
    }
    None
}

/// Сколько поисков имён ещё висят: системный поиск не прерывается, и при мёртвом DNS каждая проверка
/// оставляла бы новые потоки. Больше 16 — новых не запускаем, пока старые не кончатся.
static DNS_INFLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// DNS: время первого ответившего имени (мс). None — ни одно имя не нашлось (две попытки).
pub fn dns_ms() -> Option<f64> {
    use std::sync::atomic::Ordering::SeqCst;
    if DNS_INFLIGHT.load(SeqCst) > 16 {
        return None;
    }
    for _ in 0..2 {
        let t = Instant::now();
        let jobs: Vec<Box<dyn FnOnce() -> Option<f64> + Send>> = NAMES
            .iter()
            .map(|n| {
                let n: &'static str = n;
                Box::new(move || {
                    // системный поиск имён не умеет таймаут — его ждёт first()
                    DNS_INFLIGHT.fetch_add(1, SeqCst);
                    let r = (n, 443).to_socket_addrs().ok().and_then(|mut a| a.next()).map(|_| t.elapsed().as_secs_f64() * 1000.0);
                    DNS_INFLIGHT.fetch_sub(1, SeqCst);
                    r
                }) as Box<dyn FnOnce() -> Option<f64> + Send>
            })
            .collect();
        if let Some(ms) = first(jobs, Duration::from_secs(5)) {
            return Some(ms);
        }
    }
    None
}

/// DNS-серверы системы: Linux — /etc/resolv.conf; macOS — он же, а если там пусто — `scutil --dns`;
/// Windows — `ipconfig /all`.
fn system_resolvers() -> Vec<std::net::IpAddr> {
    #[cfg(windows)]
    {
        let out = crate::cmd::exec(&["ipconfig", "/all"], Duration::from_secs(10), crate::cmd::CHECK);
        resolvers_from_ipconfig(out.text())
    }
    #[cfg(not(windows))]
    {
        let v = resolvers_from_resolv_conf(&crate::store::read_text_lossy("/etc/resolv.conf").unwrap_or_default());
        #[cfg(target_os = "macos")]
        if v.is_empty() {
            let out = crate::cmd::exec(&["scutil", "--dns"], Duration::from_secs(8), crate::cmd::CHECK);
            return resolvers_from_scutil(out.text());
        }
        v
    }
}

/// Годится ли адрес для прямого вопроса: не «любой», не link-local IPv6 (без номера интерфейса туда не достучаться)
/// и не заглушка fec0:0:0:ffff::N, которую Windows показывает у отключённых адаптеров.
fn usable_resolver(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(a) => !a.is_unspecified(),
        std::net::IpAddr::V6(a) => !a.is_unspecified() && (a.segments()[0] & 0xffc0) != 0xfe80 && a.segments()[0] != 0xfec0,
    }
}

fn push_resolver(v: &mut Vec<std::net::IpAddr>, word: &str) -> bool {
    let Ok(ip) = word.split('%').next().unwrap_or("").parse::<std::net::IpAddr>() else { return false };
    if usable_resolver(&ip) && !v.contains(&ip) {
        v.push(ip);
    }
    true
}

/// /etc/resolv.conf: строки «nameserver адрес».
#[cfg_attr(windows, allow(dead_code))]
fn resolvers_from_resolv_conf(t: &str) -> Vec<std::net::IpAddr> {
    let mut v = Vec::new();
    for l in t.lines() {
        if let Some(w) = l.trim().strip_prefix("nameserver").and_then(|r| r.split_whitespace().next()) {
            push_resolver(&mut v, w);
        }
    }
    v.truncate(3);
    v
}

/// macOS `scutil --dns`: строки «nameserver[0] : адрес» (повторяются в каждом «resolver #N» — убираем повторы).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn resolvers_from_scutil(t: &str) -> Vec<std::net::IpAddr> {
    let mut v = Vec::new();
    for l in t.lines() {
        let l = l.trim();
        if l.starts_with("nameserver[") {
            if let Some(w) = l.split(" : ").nth(1).and_then(|r| r.split_whitespace().next()) {
                push_resolver(&mut v, w);
            }
        }
    }
    v.truncate(3);
    v
}

/// Windows `ipconfig /all`. Название строки зависит от языка («DNS Servers», «DNS-серверы»), поэтому не по названию:
/// строка со словом «DNS» до двоеточия начинает список, следующие строки, где только адрес, его продолжают.
/// Адреса одинаковы на любом языке.
#[cfg_attr(not(windows), allow(dead_code))]
fn resolvers_from_ipconfig(t: &str) -> Vec<std::net::IpAddr> {
    let mut v = Vec::new();
    let mut in_list = false;
    for l in t.lines() {
        let trimmed = l.trim();
        if let Some((key, val)) = l.split_once(" : ").or_else(|| l.split_once(": ")) {
            in_list = false;
            // «DNS Servers . . . : …», «DNS-серверы. . . : …»; не «DNS Suffix» (это имя домена, а не адрес)
            let first = val.split_whitespace().next().unwrap_or("");
            if key.to_ascii_uppercase().contains("DNS") && first.split('%').next().unwrap_or("").parse::<std::net::IpAddr>().is_ok() {
                in_list = true;
                push_resolver(&mut v, val.split_whitespace().next().unwrap_or(""));
            }
        } else if in_list && !trimmed.is_empty() && push_resolver(&mut v, trimmed.split_whitespace().next().unwrap_or("")) {
            continue;
        } else {
            in_list = false;
        }
    }
    v.truncate(3);
    v
}

/// Ответ DNS «вживую», мимо кэша: случайное имя внутри example.com (его не может быть в кэше — резолвер
/// обязан спросить настоящие серверы). Ответ «такого имени нет» (NXDOMAIN) или адрес — DNS работает до конца;
/// SERVFAIL или молчание — DNS сломан, даже если знакомые имена ещё находятся из кэша.
/// None — не проверить (нет серверов в resolv.conf).
pub fn dns_live() -> Option<bool> {
    let servers = system_resolvers();
    if servers.is_empty() {
        return None;
    }
    let id: u16 = (std::process::id() as u16)
        ^ (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as u16);
    let label =
        format!("aptechka-{:08x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u32).unwrap_or(1));
    // запрос: заголовок (id, RD=1, 1 вопрос) + имя метками + тип A + класс IN
    let mut q = vec![(id >> 8) as u8, id as u8, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for part in [label.as_str(), "example", "com"] {
        q.push(part.len() as u8);
        q.extend_from_slice(part.as_bytes());
    }
    q.extend_from_slice(&[0, 0, 1, 0, 1]);
    for srv in servers {
        let bind = if srv.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        let Ok(sock) = std::net::UdpSocket::bind(bind) else { continue };
        if sock.send_to(&q, (srv, 53)).is_err() {
            continue;
        }
        // читаем до срока: чужой или запоздавший пакет (другой источник, id, вопрос) пропускаем,
        // а не считаем ответом; неудача — только когда срок вышел
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut buf = [0u8; 512];
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                break;
            }
            let _ = sock.set_read_timeout(Some(left));
            let Ok((n, from)) = sock.recv_from(&mut buf) else { break };
            if dns_reply_matches(&buf[..n], &q, srv, from) {
                let rcode = buf[3] & 0x0f;
                // 0 — нашлось, 3 — «такого нет»: сервер ответил по существу. Это не доказывает, что спросили
                // самый главный сервер имени (бывают свои правила и кэш отказов), но DNS-путь работает
                return Some(rcode == 0 || rcode == 3);
            }
        }
    }
    Some(false)
}

/// Ответ — от того сервера, которого спрашивали, с нашим id, это ответ (флаг QR), и в нём наш же вопрос.
fn dns_reply_matches(buf: &[u8], q: &[u8], srv: std::net::IpAddr, from: std::net::SocketAddr) -> bool {
    let qlen = q.len() - 12;
    buf.len() >= q.len()
        && from.ip() == srv
        && from.port() == 53
        && buf[0] == q[0]
        && buf[1] == q[1]
        && buf[2] & 0x80 != 0
        && buf[4..6] == [0, 1]
        && buf[12..12 + qlen].eq_ignore_ascii_case(&q[12..])
}

/// Строки «Интернет» и «DNS» — общие для всех систем. `extra` — кнопки починки сети (переподключить).
pub fn rows(dns_fix: &str, extra: Vec<crate::diag::Fix>) -> Vec<crate::diag::Check> {
    use crate::diag::{fx, Check, BAD, OK, WARN};
    let mut out = Vec::new();
    // DNS и интернет проверяем одновременно
    let dns = std::thread::spawn(dns_ms);
    match internet() {
        Some(a) => out.push(Check::new("net", "Интернет", OK, "есть").hint(&format!(
            "Ответил {}. Настоящий пинг и скорость — в строках «Пинг» и «Скорость».",
            a.split("://").nth(1).map_or(a, |r| r.split('/').next().unwrap_or(r)).trim_end_matches(":443")
        ))),
        None => match crate::diag::proxy_internet() {
            // напрямую не пускают, а через прокси системы — да (так устроены рабочие сети)
            Some(px) => out.push(Check::new("net", "Интернет", OK, &format!("есть, через прокси {px}"))
                .hint("Напрямую сеть закрыта, программы ходят через прокси — это нормально для рабочих сетей.")),
            None => out.push(Check::new("net", "Интернет", BAD, "недоступен")
                .hint("Не ответили ни Cloudflare, ни Яндекс, ни страницы проверки Google, Microsoft и Apple (две попытки). Проверь роутер, кабель или Wi-Fi.")
                .fixes(extra)),
        },
    }
    let dfix = vec![fx(dns_fix, "Сбросить кэш DNS", None)];
    let live = std::thread::spawn(dns_live);
    let (dns, live) = (dns.join().ok().flatten(), live.join().ok().flatten());
    match dns {
        // знакомые имена находятся, а живой запрос — нет: отвечает только кэш, новые сайты не откроются
        Some(_) if live == Some(false) => out.push(
            Check::new("net", "DNS", WARN, "новые имена не находятся")
                .hint(
                    "Знакомые сайты находятся (вероятно, из памяти), а прямой вопрос DNS-серверу системы остался без ответа. \
                Бывает, что сеть закрывает обычный DNS (порт 53) и пускает только другой путь, — тогда всё в порядке. \
                Если новые сайты правда не открываются — сбрось кэш DNS; не поможет — DNS, оставленный VPN, или провайдер.",
                )
                .fixes(dfix.clone()),
        ),
        Some(ms) => out.push(
            Check::new("net", "DNS", if ms > 1500.0 { WARN } else { OK }, &format!("{ms:.0} мс"))
                .hint(if ms > 1500.0 {
                    "Имена сайтов находятся медленно — страницы будут открываться с задержкой."
                } else {
                    ""
                })
                .fixes(dfix),
        ),
        None => out.push(
            Check::new("net", "DNS", BAD, "имена не находятся")
                .hint("Интернет может быть, но адреса сайтов не определяются. Помогает сброс кэша DNS; иначе — DNS, оставленный VPN.")
                .fixes(dfix),
        ),
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn first_wins_and_times_out() {
        use std::time::Duration;
        let jobs: Vec<Box<dyn FnOnce() -> Option<u8> + Send>> = vec![
            Box::new(|| None),
            Box::new(|| {
                std::thread::sleep(Duration::from_millis(50));
                Some(2)
            }),
        ];
        assert_eq!(super::first(jobs, Duration::from_secs(2)), Some(2));
        let slow: Vec<Box<dyn FnOnce() -> Option<u8> + Send>> = vec![Box::new(|| {
            std::thread::sleep(Duration::from_secs(2));
            Some(1)
        })];
        assert_eq!(super::first(slow, Duration::from_millis(100)), None);
        assert!(super::TCP.iter().all(|a| a.parse::<std::net::SocketAddr>().is_ok()));
    }
}

#[cfg(test)]
mod dns_tests {
    use super::*;

    #[test]
    fn stray_packets_do_not_match() {
        let srv: std::net::IpAddr = "192.0.2.1".parse().unwrap();
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        q.extend_from_slice(&[3, b'a', b'b', b'c', 0, 0, 1, 0, 1]);
        let mut good = q.clone();
        good[2] |= 0x80;
        let from = std::net::SocketAddr::new(srv, 53);
        assert!(dns_reply_matches(&good, &q, srv, from));
        // чужой источник, чужой порт, другой id, это запрос (нет QR), другой вопрос
        assert!(!dns_reply_matches(&good, &q, srv, "192.0.2.9:53".parse().unwrap()));
        assert!(!dns_reply_matches(&good, &q, srv, "192.0.2.1:5353".parse().unwrap()));
        let mut o = good.clone();
        o[1] ^= 1;
        assert!(!dns_reply_matches(&o, &q, srv, from));
        assert!(!dns_reply_matches(&q, &q, srv, from));
        let mut o = good.clone();
        o[13] = b'z';
        assert!(!dns_reply_matches(&o, &q, srv, from));
        assert!(!dns_reply_matches(&good[..10], &q, srv, from));
    }
}

#[cfg(test)]
mod resolver_tests {
    use super::*;

    fn ips(v: Vec<std::net::IpAddr>) -> Vec<String> {
        v.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn resolv_conf() {
        let t = "# comment\nsearch lan\nnameserver 192.168.2.1\nnameserver fe80::1%en0\nnameserver 2606:4700:4700::1111\n";
        assert_eq!(ips(resolvers_from_resolv_conf(t)), ["192.168.2.1", "2606:4700:4700::1111"]);
    }

    #[test]
    fn scutil_dns() {
        // настоящий вывод macOS 26 (обезличен), плюс второй адаптер с тем же сервером
        let t = "DNS configuration\n\nresolver #1\n  search domain[0] : lan\n  nameserver[0] : 192.168.2.1\n  if_index : 11 (en0)\n  flags    : Request A records\n  reach    : 0x00020002 (Reachable,Directly Reachable Address)\n\nresolver #2\n  domain   : local\n  options  : mdns\n\nDNS configuration (for scoped queries)\n\nresolver #1\n  search domain[0] : lan\n  nameserver[0] : 192.168.2.1\n  nameserver[1] : 1.1.1.1\n";
        assert_eq!(ips(resolvers_from_scutil(t)), ["192.168.2.1", "1.1.1.1"]);
        assert!(resolvers_from_scutil("").is_empty());
    }

    #[test]
    fn ipconfig_english_and_russian() {
        let en = "Windows IP Configuration\r\n\r\nEthernet adapter Ethernet:\r\n\r\n   Connection-specific DNS Suffix  . : lan\r\n   Description . . . . . . . . . . . : Intel(R) PRO/1000\r\n   IPv6 Address. . . . . . . . . . . : fd00::5(Preferred)\r\n   DHCP Enabled. . . . . . . . . . . : Yes\r\n   DNS Servers . . . . . . . . . . . : 192.168.1.1\r\n                                       8.8.8.8\r\n   NetBIOS over Tcpip. . . . . . . . : Enabled\r\n";
        assert_eq!(ips(resolvers_from_ipconfig(en)), ["192.168.1.1", "8.8.8.8"]);
        let ru = "Настройка протокола IP для Windows\r\n\r\nАдаптер Ethernet Ethernet:\r\n\r\n   DNS-суффикс подключения . . . . . : lan\r\n   DNS-серверы. . . . . . . . . . . : fe80::1%12\r\n                                       192.168.1.1\r\n                                       2001:4860:4860::8888\r\n   NetBios через TCP/IP. . . . . . . . : Включен\r\n   Основной шлюз. . . . . . . . . : 192.168.1.1\r\n";
        assert_eq!(ips(resolvers_from_ipconfig(ru)), ["192.168.1.1", "2001:4860:4860::8888"]);
        // отключённый адаптер с заглушкой и без серверов — пусто
        let off = "Wireless LAN adapter Wi-Fi:\r\n   Media State . . . . . . . . . . . : Media disconnected\r\n   DNS Servers . . . . . . . . . . . : fec0:0:0:ffff::1%1\r\n                                       fec0:0:0:ffff::2%1\r\n";
        assert!(resolvers_from_ipconfig(off).is_empty());
    }
}
