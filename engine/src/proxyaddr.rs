//! Адрес прокси в любом виде, в каком его пишут люди и программы, — один разбор на все проверки.
//!
//!   socks5h://user:pass@127.0.0.1:10808/   http://[::1]:8080   127.0.0.1:10809   proxy.local
//!   http=h:3128;https=h:3129   (Windows, «Параметры Интернета»)   socks=127.0.0.1:1080
//!
//! Логин и пароль нужны для подключения, но никогда не показываются и не пишутся в файлы: `shown()`.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct ProxyAddr {
    /// "http" или "socks5" (как подключаться)
    pub scheme: &'static str,
    pub host: String,
    pub port: u16,
    /// "user:pass" (как было в адресе) — только для подключения
    auth: Option<String>,
}

impl ProxyAddr {
    /// Разобрать адрес. Без порта — порт по умолчанию, как у curl: 1080 (у https:// — 443).
    pub fn parse(s: &str) -> Option<ProxyAddr> {
        let s = s.trim().trim_matches(['"', '\'']).trim();
        // Windows: «http=h:p;https=h:p;socks=h:p» — предпочитаем https, потом http, потом что есть
        if s.contains('=') && !s.contains("://") {
            let parts: Vec<(&str, &str)> = s.split(';').filter_map(|x| x.split_once('=')).collect();
            let pick = ["https", "http", "socks"].iter().find_map(|k| parts.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)));
            let (k, v) = pick.or(parts.first())?;
            let scheme = if k.eq_ignore_ascii_case("socks") { "socks5://" } else { "http://" };
            return ProxyAddr::parse(&format!("{scheme}{v}"));
        }
        let (scheme_s, rest) = s.split_once("://").unwrap_or(("http", s));
        let scheme_l = scheme_s.to_ascii_lowercase();
        let (scheme, default_port) = match scheme_l.as_str() {
            "http" => ("http", 1080),
            "https" => ("http", 443),
            "socks" | "socks5" | "socks5h" => ("socks5", 1080),
            // SOCKS4 не умеет того же, что SOCKS5 (имена, IPv6) — подключаемся по его же протоколу
            "socks4" => ("socks4", 1080),
            "socks4a" => ("socks4a", 1080),
            _ => return None,
        };
        let rest = rest.split(['/', '?', '#']).next()?;
        let (auth, hostport) = match rest.rsplit_once('@') {
            Some((a, hp)) => (Some(a.to_string()).filter(|a| !a.is_empty()), hp),
            None => (None, rest),
        };
        let (host, port) = if let Some(r) = hostport.strip_prefix('[') {
            // IPv6: [::1]:8080
            let (h, after) = r.split_once(']')?;
            (h.to_string(), after.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(default_port))?)
        } else {
            match hostport.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().ok()?),
                Some(_) => (hostport.to_string(), default_port), // голый IPv6 без скобок
                None => (hostport.to_string(), default_port),
            }
        };
        let ok_host = !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c));
        (ok_host && port != 0).then_some(ProxyAddr { scheme, host, port, auth })
    }

    /// Адрес для подключения (с логином, если был).
    pub fn url(&self) -> String {
        let auth = self.auth.as_ref().map(|a| format!("{a}@")).unwrap_or_default();
        format!("{}://{auth}{}", self.scheme, self.host_port())
    }

    /// «host:port» (IPv6 — в скобках): для показа и для подключения по TCP.
    pub fn host_port(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// Для показа человеку и записи в файлы: без логина и пароля.
    pub fn shown(&self) -> String {
        self.host_port()
    }

    /// Прокси на этом же компьютере.
    pub fn is_local(&self) -> bool {
        let h = self.host.to_ascii_lowercase();
        h == "localhost" || h == "::1" || h.starts_with("127.")
    }

    /// Открыт ли порт (кто-то слушает).
    pub fn listening(&self, timeout: Duration) -> bool {
        use std::net::ToSocketAddrs;
        let Ok(addrs) = (self.host.as_str(), self.port).to_socket_addrs() else { return false };
        addrs.take(2).any(|a| std::net::TcpStream::connect_timeout(&a, timeout).is_ok())
    }
}

/// Убрать «логин:пароль@» из любого адреса в тексте (для показа и записи): и в «схема://user:pass@host»,
/// и в голом «user:pass@host» (так пишут прокси без схемы и закрытые репозитории). Почта «a@b.ru»
/// не трогается: в ней до «@» нет двоеточия.
pub fn redact(s: &str) -> String {
    let s = &redact_values(s);
    if !s.contains('@') && !s.contains("://") {
        return s.to_string();
    }
    let with_scheme = redact_urls(s);
    // слова «x:y@host» без схемы
    let mut out = String::with_capacity(with_scheme.len());
    for (i, w) in with_scheme.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match w.rfind('@') {
            Some(a) if !w.contains("://") && !w.contains("•••") => {
                let (cred, rest) = w.split_at(a);
                // начало слова — кавычка или «=» (http_proxy=user:pass@…)
                let start = cred.rfind(['=', '"', '\'', '(']).map_or(0, |p| p + 1);
                if cred[start..].contains(':') && !cred[start..].contains('/') {
                    out.push_str(&cred[..start]);
                    out.push_str("•••");
                    out.push_str(rest);
                } else {
                    out.push_str(w);
                }
            }
            _ => out.push_str(w),
        }
    }
    out
}

fn redact_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        // логин и пароль могут содержать «/», «?», «#» сырыми: «@» ищем до пробела или кавычки, берём последний
        let chunk_end = tail.find(|c: char| c.is_whitespace() || "\"'".contains(c)).unwrap_or(tail.len());
        let tail = match tail[..chunk_end].rfind('@') {
            Some(a) => {
                out.push_str("•••@");
                &tail[a + 1..]
            }
            None => tail,
        };
        let end = tail.find(|c: char| c.is_whitespace() || "/?#\"'".contains(c)).unwrap_or(tail.len());
        out.push_str(&tail[..end]);
        // параметры и якорь адреса (?token=…, #…) — часто и есть ключ подписки: не показываем
        let after = &tail[end..];
        let path_end = after.find(|c: char| c.is_whitespace() || "\"'".contains(c)).unwrap_or(after.len());
        let path = &after[..path_end];
        match path.find(['?', '#']) {
            Some(q) => {
                out.push_str(&path[..q]);
                out.push_str(&path[q..q + 1]);
                out.push_str("•••");
            }
            None => out.push_str(path),
        }
        rest = &after[path_end..];
    }
    out.push_str(rest);
    out
}

/// Начинается ли `b[i..]` с `kw` (латиница без учёта регистра; индексы по исходным символам).
fn starts_ci(b: &[char], i: usize, kw: &str) -> bool {
    let mut n = 0;
    for (k, c) in kw.chars().enumerate() {
        match b.get(i + k) {
            Some(x) if x.eq_ignore_ascii_case(&c) => n += 1,
            _ => return false,
        }
    }
    n > 0
}

/// «Bearer xxx», «Basic xxx», «api_key=xxx», «"token": "xxx"», «password: xxx», «Cookie: a=b», «sessionid=xxx»,
/// «curl -u user:pass» → значение заменено на «•••».
fn redact_values(s: &str) -> String {
    const KEYS: [&str; 12] =
        ["api_key", "apikey", "api-key", "token", "secret", "password", "passwd", "authorization", "bearer", "cookie", "sessionid", "session_id"];
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        // ключ (без учёта регистра), за ним «=», «:», пробел или кавычки, потом значение до пробела/кавычки/запятой
        let hit = KEYS.iter().find(|k| starts_ci(&b, i, k) && (i == 0 || !b[i - 1].is_alphanumeric()));
        if let Some(k) = hit {
            let mut j = i + k.chars().count();
            // хвост имени ключа (OPENAI_API_KEY, x_auth_token…)
            while j < b.len() && (b[j].is_alphanumeric() || b[j] == '_') {
                j += 1;
            }
            let mut v = j;
            while v < b.len() && matches!(b[v], ' ' | '=' | ':' | '"' | '\'') {
                v += 1;
            }
            // «Bearer » / «Basic » перед значением
            if starts_ci(&b, v, "bearer ") {
                v += 7;
            } else if starts_ci(&b, v, "basic ") {
                v += 6;
            }
            let mut e = v;
            if *k == "cookie" {
                // «Cookie: a=1; b=2» — значение до конца строки
                while e < b.len() && !matches!(b[e], '\n' | '\r' | '"') {
                    e += 1;
                }
                if !b[v..e].contains(&'=') {
                    e = v;
                }
            } else {
                while e < b.len() && !b[e].is_whitespace() && !matches!(b[e], '"' | '\'' | ',' | ';' | '&' | ')') {
                    e += 1;
                }
            }
            // значение — что-то похожее на секрет (не «да/нет», не одно слово по-русски)
            if v > j && e - v >= 6 {
                out.extend(&b[i..v]);
                out.push_str("•••");
                i = e;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    redact_cli_auth(&out)
}

/// `curl -u user:pass`, `--user user:pass`, `--proxy-user=user:pass`: логин с паролем в аргументах команды.
fn redact_cli_auth(s: &str) -> String {
    const FLAGS: [&str; 4] = ["-u", "-U", "--user", "--proxy-user"];
    if !s.contains("-u") && !s.contains("-U") && !s.contains("--user") {
        return s.to_string();
    }
    let words: Vec<&str> = s.split(' ').collect();
    let mut out = String::with_capacity(s.len());
    for (n, w) in words.iter().enumerate() {
        if n > 0 {
            out.push(' ');
        }
        let after_flag = n > 0 && FLAGS.contains(&words[n - 1]);
        let eq = FLAGS.iter().find(|f| w.starts_with(&format!("{f}="))).map(|f| f.len() + 1);
        if after_flag && w.contains(':') && !w.contains("://") {
            let q = if w.starts_with(['"', '\'']) { &w[..1] } else { "" };
            out.push_str(q);
            out.push_str("•••");
            if !q.is_empty() && w.len() > 1 && w.ends_with(q) {
                out.push_str(q);
            }
        } else if let Some(k) = eq.filter(|k| w[*k..].contains(':')) {
            out.push_str(&w[..k]);
            out.push_str("•••");
        } else {
            out.push_str(w);
        }
    }
    out
}

/// Что получилось, когда сходили в интернет через прокси.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Through {
    /// сайты открываются
    Works,
    /// прокси жив, но просит логин и пароль (407) или запрещает (403)
    Refuses(u16),
    /// порт открыт, но дальше прокси не пускает (сервер VPN недоступен)
    NoInternet,
}

/// Сходить через прокси на несколько адресов проверки (один сайт может быть недоступен сам по себе).
pub fn through(p: &ProxyAddr) -> Through {
    const PROBES: [&str; 3] =
        ["https://www.google.com/generate_204", "https://cp.cloudflare.com/generate_204", "http://connectivitycheck.gstatic.com/generate_204"];
    let url = p.url();
    let mut refused = None;
    for probe in PROBES {
        match crate::diag::http_get(probe, 6, 10_000, Some(&url)).0 {
            Some(c) if (200..400).contains(&c) || c == 404 => return Through::Works,
            Some(c @ (401 | 403 | 407)) => refused = Some(c),
            // 5xx от самого прокси (502/503/504) — он не дошёл до сайта; пробуем следующий
            _ => {}
        }
    }
    refused.map_or(Through::NoInternet, Through::Refuses)
}

/// Прокси в переменных окружения, с которыми запущена Аптечка (их наследуют программы её сеанса) —
/// одна проверка на всех системах.
pub fn env_row() -> Option<crate::diag::Check> {
    use crate::diag::{Check, BAD, OK, WARN};
    // vars_os: переменная не в UTF-8 (бывает в чужих сеансах) не должна ронять программу — env::vars() паникует
    let (k, v) = std::env::vars_os()
        .map(|(k, v)| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
        .find(|(k, v)| matches!(k.to_lowercase().as_str(), "http_proxy" | "https_proxy" | "all_proxy") && !v.trim().is_empty())?;
    const T: &str = "Прокси в переменных сеанса";
    Some(match ProxyAddr::parse(&v) {
        Some(p) if p.listening(Duration::from_secs(2)) => {
            Check::new("net", T, OK, &p.shown()).hint(&format!("Программы этого сеанса ходят через {k}."))
        }
        Some(p) => Check::new("net", T, BAD, &format!("{} — там пусто", p.shown())).hint(&format!(
            "Переменная {k} указывает на мёртвый порт: программы, запущенные с ней (терминал, curl, git, pip), без сети. \
             Найди, где она задана (строка «Прокси в …» ниже), или в терминале выполни: {}",
            if cfg!(windows) {
                "Remove-Item Env:http_proxy, Env:https_proxy, Env:all_proxy"
            } else {
                "unset http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY"
            }
        )),
        None => Check::new("net", T, WARN, &redact(&v).chars().take(40).collect::<String>())
            .hint(&format!("В {k} записан адрес, который не получается разобрать.")),
    })
}

/// Строка проверки для системного прокси — одна на Linux, Windows и macOS.
/// Порт закрыт — BAD (VPN закрылся, прокси остался); открыт — идём через него в интернет.
pub fn system_row(title: &str, p: &ProxyAddr, off: Vec<crate::diag::Fix>) -> crate::diag::Check {
    use crate::diag::{Check, BAD, OK, WARN};
    let a = p.shown();
    if !p.listening(Duration::from_secs(2)) {
        let why = if p.is_local() {
            "VPN-клиент закрылся и оставил прокси — сайты и программы без сети. Выключи прокси."
        } else {
            "Прокси-сервер не отвечает — сайты и программы без сети. Выключи прокси или проверь его адрес."
        };
        return Check::new("net", title, BAD, &format!("включён на {a}, но там пусто!")).hint(why).fixes(off);
    }
    match through(p) {
        Through::Works => Check::new("net", title, OK, &format!("работает ({a})")).hint("Сеть идёт через прокси.").fixes(off),
        Through::Refuses(407) => Check::new("net", title, WARN, &format!("{a} просит логин и пароль"))
            .hint("Прокси работает, но без логина и пароля не пускает. Программы, где пароль не сохранён, останутся без сети."),
        Through::Refuses(c) => Check::new("net", title, WARN, &format!("{a} не пускает (код {c})"))
            .hint("Прокси работает, но запрещает выход в интернет с этого компьютера."),
        Through::NoInternet => Check::new("net", title, BAD, &format!("{a} не пускает в интернет"))
            .hint("Порт открыт, но через него сайты не грузятся: сервер VPN недоступен. Перезапусти VPN или выключи прокси.")
            .fixes(off),
    }
}

/// Для показа адреса, который может быть секретным целиком (PAC, подписка VPN): только «схема://сервер».
pub fn url_host(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = auth.rsplit('@').next().unwrap_or("");
            format!("{scheme}://{host}/…")
        }
        None if url.starts_with('/') => "файл на этом компьютере".into(),
        None => "адрес задан".into(),
    }
}

/// Строка проверки для автонастройки прокси (PAC-файл).
pub fn pac_row(url: &str, off: Vec<crate::diag::Fix>) -> crate::diag::Check {
    use crate::diag::{Check, BAD, INFO, OK};
    let url = url.trim().trim_matches('\'');
    // адрес PAC или подписки целиком бывает ключом доступа (в пути, параметрах, якоре) — показываем только сервер
    let shown = url_host(url);
    if url.is_empty() {
        return Check::new("net", "Прокси по PAC-файлу", BAD, "без адреса").hint("Автонастройка прокси включена, но адрес файла пустой.").fixes(off);
    }
    // локальный файл: file:///…, просто путь
    if let Some(path) = url.strip_prefix("file://").or_else(|| url.starts_with('/').then_some(url)) {
        // file:///C:/x.pac → C:/x.pac (Windows)
        let path = match path.strip_prefix('/') {
            Some(p) if p.as_bytes().get(1) == Some(&b':') => p,
            _ => path,
        };
        let ok = std::path::Path::new(path).is_file();
        return Check::new("net", "Прокси по PAC-файлу", if ok { INFO } else { BAD }, &shown)
            .hint(if ok {
                "Правила прокси берутся из файла на этом компьютере."
            } else {
                "Файла автонастройки прокси нет на месте."
            })
            .fixes(if ok { Vec::new() } else { off });
    }
    match crate::diag::http_get(url, 5, 200_000, None).0 {
        Some(200) => Check::new("net", "Прокси по PAC-файлу", OK, &shown).hint("Правила прокси берутся из PAC-файла."),
        Some(c) => Check::new("net", "Прокси по PAC-файлу", BAD, &format!("{shown} — ошибка {c}"))
            .hint("Файл автонастройки прокси не открывается.")
            .fixes(off),
        None => Check::new("net", "Прокси по PAC-файлу", BAD, &shown).hint("Файл автонастройки прокси не открывается.").fixes(off),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hp(s: &str) -> Option<(String, u16, &'static str)> {
        ProxyAddr::parse(s).map(|p| (p.host, p.port, p.scheme))
    }

    #[test]
    fn forms() {
        assert_eq!(hp("socks5h://127.0.0.1:10808"), Some(("127.0.0.1".into(), 10808, "socks5")));
        assert_eq!(hp("\"http://user:p@ss@localhost:8080/\""), Some(("localhost".into(), 8080, "http")));
        assert_eq!(hp("127.0.0.1:10809"), Some(("127.0.0.1".into(), 10809, "http")));
        assert_eq!(hp("proxy.corp"), Some(("proxy.corp".into(), 1080, "http")));
        assert_eq!(hp("http://[::1]:8080"), Some(("::1".into(), 8080, "http")));
        assert_eq!(hp("socks=127.0.0.1:1080"), Some(("127.0.0.1".into(), 1080, "socks5")));
        assert_eq!(hp("http=a:1;https=b:2"), Some(("b".into(), 2, "http")));
        assert_eq!(hp("нет"), None);
        assert_eq!(hp("ftp://x:1"), None);
        assert_eq!(hp("http://h:0"), None);
        let p = ProxyAddr::parse("http://u:secret@h:3128").unwrap();
        assert_eq!(p.url(), "http://u:secret@h:3128");
        assert_eq!(p.shown(), "h:3128");
        assert_eq!(ProxyAddr::parse("[::1]:9").unwrap().host_port(), "[::1]:9");
    }

    #[test]
    fn redacts() {
        assert_eq!(redact("http_proxy=http://u:secret@h:3128/x"), "http_proxy=http://•••@h:3128/x");
        assert_eq!(redact("socks5://h:1 и http://a@b"), "socks5://h:1 и http://•••@b");
        assert_eq!(redact("без адреса"), "без адреса");
        assert_eq!(redact("user:S3cret@host_name!:3128x"), "•••@host_name!:3128x");
        assert_eq!(redact("Не отвечают: user42:Tok@private-ppa.invalid"), "Не отвечают: •••@private-ppa.invalid");
        assert_eq!(redact("пиши на a@b.ru"), "пиши на a@b.ru");
        assert_eq!(redact("http_proxy=u:p@h:1"), "http_proxy=•••@h:1");
        // ключи вне адресов и параметры адресов
        assert!(!redact("Authorization: Bearer AUDIT_CANARY_NOT_A_KEY").contains("AUDIT_CANARY"));
        assert!(!redact("OPENAI_API_KEY=AUDIT_CANARY_NOT_A_KEY").contains("AUDIT_CANARY"));
        assert!(!redact("https://example.invalid/pac?token=AUDIT_CACHE_CANARY").contains("AUDIT_CACHE"));
        assert!(!redact("https://example.invalid/pac#AUDIT_FRAG").contains("AUDIT_FRAG"));
        assert!(!redact("https://AUDIT_TOKEN_ONLY@repo.invalid/repo").contains("AUDIT_TOKEN_ONLY"));
        // обычный текст не трогаем
        assert_eq!(redact("Токен входа лежит в файле"), "Токен входа лежит в файле");
        assert_eq!(redact("прокси 127.0.0.1:2080 работает"), "прокси 127.0.0.1:2080 работает");
    }

    #[test]
    fn redact_bypasses() {
        // 1. «İ» меняет число символов при to_lowercase
        let r = redact("İ cmd password=Sup3rSecretPass1");
        assert!(!r.contains("Sup3rSecret") && r.starts_with("İ cmd password="), "{r}");
        let r = redact("İİİ TOKEN: abcdef123456 и ещё password=Zzzzzzzz1");
        assert!(!r.contains("abcdef") && !r.contains("Zzzzzzzz"), "{r}");
        // 2. Basic
        let r = redact("Authorization: Basic dXNlcjpwYXNzd29yZA==");
        assert!(!r.contains("dXNlcjpw"), "{r}");
        // 3. «/» в пароле
        let r = redact("http://user:pa/ss@host:8080");
        assert_eq!(r, "http://•••@host:8080");
        assert_eq!(redact("proxy http://u:p?x#y@h:1/path ok"), "proxy http://•••@h:1/path ok");
        // 4. curl -u, --user, --proxy-user, Cookie, sessionid
        assert!(!redact("curl -u admin:hunter22 https://x").contains("hunter22"));
        assert!(!redact("curl --user admin:hunter22 https://x").contains("hunter22"));
        assert!(!redact("curl --proxy-user=admin:hunter22 https://x").contains("hunter22"));
        assert!(!redact("curl --proxy-user admin:hunter22 https://x").contains("hunter22"));
        assert!(!redact("Cookie: sid=abc123def; theme=dark").contains("abc123"));
        assert!(!redact("Set-Cookie: sid=abc123def; Path=/").contains("abc123"));
        assert!(!redact("x sessionid=abcdef123456 y").contains("abcdef"));
        // 5. длинный текст не квадратичен
        let long = "token=abcdefghij ".repeat(20000);
        assert!(!redact(&long).contains("abcdefghij"));
        // обычное остаётся
        assert_eq!(redact("curl -u 5 https://x"), "curl -u 5 https://x");
        assert_eq!(redact("Cookie: нет"), "Cookie: нет");
    }
}
