//! История внешнего IP: когда адрес менялся, на какой и сколько держался — чтобы видеть, насколько он устойчив.
//! Запись — при каждой проверке «Мой внешний IP»; файл ip_history.json в папке настроек (только владелец, 0600),
//! хранится 30 дней и не больше 300 записей. Никуда не отправляется. Удалить историю — удалить этот файл.

/// Один отрезок: адрес и с какого по какой миг его видели (Unix-время).
#[derive(Clone, Debug, PartialEq)]
pub struct Stay {
    pub ip: String,
    pub country: String,
    pub from: i64,
    pub to: i64,
}

const KEEP_SECS: i64 = 30 * 86_400;
const MAX: usize = 300;
/// Между двумя наблюдениями прошло больше — программа не работала (или не было сети): это не «держался».
const GAP_SECS: i64 = 3_600;

/// Новое наблюдение: тот же адрес — отрезок продлевается, другой — начинается новый. Часы, ушедшие назад, не
/// укорачивают отрезок. Старое (30 дней) и лишнее (больше 300) отбрасывается с начала.
pub fn step(mut h: Vec<Stay>, ip: &str, country: &str, now: i64) -> Vec<Stay> {
    match h.last_mut() {
        Some(last) if last.ip == ip => {
            last.to = last.to.max(now);
            if !country.is_empty() {
                last.country = country.to_string();
            }
        }
        _ => {
            let from = h.last().map_or(now, |l| now.max(l.to));
            h.push(Stay { ip: ip.to_string(), country: country.to_string(), from, to: from });
        }
    }
    h.retain(|s| s.to >= now - KEEP_SECS);
    let extra = h.len().saturating_sub(MAX);
    h.drain(..extra);
    h
}

/// Сколько раз адрес сменился начиная с мига `since` (первая запись истории — не смена).
pub fn changes_since(h: &[Stay], since: i64) -> usize {
    h.iter().skip(1).filter(|s| s.from >= since).count()
}

/// «5 мин», «3 ч 12 мин», «2 дн 4 ч» — без секунд: проверка идёт раз в несколько минут, точнее не знаем.
/// Всё здесь — по-русски: подсказку переводит окно при показе (иначе после смены языка она осталась бы на прежнем).
fn span(secs: i64) -> String {
    let m = secs.max(0) / 60;
    if m < 60 {
        format!("{m} мин")
    } else if m < 24 * 60 {
        format!("{} ч {} мин", m / 60, m % 60)
    } else {
        format!("{} дн {} ч", m / 1440, m % 1440 / 60)
    }
}

/// «15.11 21:40» по местному времени.
fn stamp(secs: i64) -> String {
    let tm = crate::ui::localtime(secs);
    format!("{:02}.{:02} {:02}:{:02}", tm.mday, tm.mon, tm.hour, tm.min)
}

/// Строки подсказки: сколько держится нынешний адрес, сколько было смен и последние прежние адреса (свежие сверху).
pub fn summary(h: &[Stay], now: i64) -> Vec<String> {
    let Some(cur) = h.last() else { return Vec::new() };
    let mut out = Vec::new();
    let held = format!("Этот адрес — с {} ({})", stamp(cur.from), span(now.max(cur.to) - cur.from));
    if h.len() == 1 {
        out.push(held);
        out.push("Смен адреса пока не видел.".into());
        return out;
    }
    out.push(held);
    out.push(format!(
        "Смен за сутки: {} · за неделю: {} · за 30 дней: {}",
        changes_since(h, now - 86_400),
        changes_since(h, now - 7 * 86_400),
        changes_since(h, now - KEEP_SECS)
    ));
    // прежние адреса: с какого по какой миг, сколько держался; разрыв в наблюдениях — отдельной пометкой
    for w in h.windows(2).rev().take(4) {
        let (p, next) = (&w[0], &w[1]);
        let c = if p.country.is_empty() { String::new() } else { format!(" {}", p.country) };
        let gap =
            if next.from - p.to > GAP_SECS { format!(", потом без наблюдений {}", span(next.from - p.to)) } else { String::new() };
        out.push(format!("{} – {}: {}{} ({}{})", stamp(p.from), stamp(p.to), p.ip, c, span(p.to - p.from), gap));
    }
    out
}

fn path() -> std::path::PathBuf {
    crate::store::private_dir().join("ip_history.json")
}

fn parse(text: &str) -> Vec<Stay> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let s = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let n = |x: &serde_json::Value, k: &str| x.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    v.as_array()
        .map(|a| a.iter().map(|x| Stay { ip: s(x, "ip"), country: s(x, "country"), from: n(x, "from"), to: n(x, "to") }).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        // мусор в файле: пустой адрес, время задом наперёд или из будущего далеко за пределами часов — не история
        .filter(|s| !s.ip.is_empty() && s.ip.len() <= 64 && s.from > 0 && s.to >= s.from)
        .collect()
}

fn dump(h: &[Stay]) -> String {
    serde_json::Value::Array(h.iter().map(|s| serde_json::json!({"ip": s.ip, "country": s.country, "from": s.from, "to": s.to})).collect())
        .to_string()
}

/// Записать наблюдение и вернуть строки подсказки. Ошибки чтения и записи не мешают проверке: история просто короче.
pub fn note(ip: &str, country: &str, now: i64) -> Vec<String> {
    let p = path();
    let old = crate::store::read_capped(&p, 256 * 1024).map(|t| parse(&t)).unwrap_or_default();
    // записи «из будущего» (часы переводили назад) не должны навсегда заморозить историю
    let old: Vec<Stay> = old.into_iter().filter(|s| s.from <= now + 600).collect();
    let h = step(old.clone(), ip, country, now);
    if h != old {
        let _ = crate::store::write_private_quick(&p, dump(&h).as_bytes());
    }
    summary(&h, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(ip: &str, from: i64, to: i64) -> Stay {
        Stay { ip: ip.into(), country: "DE".into(), from, to }
    }

    /// Подсказка собирается по-русски и переводится при показе: по-английски в ней не должно остаться русского.
    #[test]
    fn summary_is_fully_translated() {
        let now = 2_000_000_000;
        let h = [
            st("198.51.100.12", now - 400_000, now - 200_000),
            st("203.0.113.47", now - 190_000, now - 5_400),
            st("198.51.100.80", now - 5_000, now),
        ];
        let lines = summary(&h, now);
        assert!(lines.len() >= 4, "{lines:?}");
        for l in lines.iter().chain(summary(&h[..1], now).iter()) {
            let en = crate::i18n::to_english(l);
            assert!(!en.chars().any(|c| matches!(c, 'А'..='я' | 'Ё' | 'ё')), "«{l}» → «{en}»");
        }
    }

    #[test]
    fn same_address_extends_other_starts_new() {
        let h = step(Vec::new(), "1.1.1.1", "DE", 1_000);
        assert_eq!(h, vec![st("1.1.1.1", 1_000, 1_000)]);
        let h = step(h, "1.1.1.1", "DE", 1_900);
        assert_eq!(h, vec![st("1.1.1.1", 1_000, 1_900)]);
        let h = step(h, "2.2.2.2", "DE", 2_500);
        assert_eq!(h, vec![st("1.1.1.1", 1_000, 1_900), st("2.2.2.2", 2_500, 2_500)]);
        // вернулся прежний адрес — это новая смена, а не продолжение старого отрезка
        let h = step(h, "1.1.1.1", "DE", 3_000);
        assert_eq!(h.len(), 3);
        assert_eq!(changes_since(&h, 0), 2);
        assert_eq!(changes_since(&h, 2_600), 1);
        assert_eq!(changes_since(&h, 3_001), 0);
    }

    #[test]
    fn clock_going_back_never_shortens_or_reorders() {
        let h = step(vec![st("1.1.1.1", 1_000, 5_000)], "1.1.1.1", "", 4_000);
        assert_eq!(h, vec![st("1.1.1.1", 1_000, 5_000)]);
        let h = step(h, "2.2.2.2", "FR", 4_500);
        assert_eq!(h[1].from, 5_000);
        assert!(h.windows(2).all(|w| w[0].to <= w[1].from));
    }

    #[test]
    fn old_and_extra_records_are_dropped() {
        let now = 100 * 86_400;
        // держался до «31 день назад» — уходит; до «29 дней назад» — остаётся
        let h = step(vec![st("a", 1, now - 31 * 86_400), st("b", now - 30 * 86_400, now - 29 * 86_400)], "c", "", now);
        assert_eq!(h.iter().map(|s| s.ip.as_str()).collect::<Vec<_>>(), ["b", "c"]);
        let mut h = Vec::new();
        for i in 0..400 {
            h = step(h, &format!("10.0.{}.{}", i / 250, i % 250), "", now + i);
        }
        assert_eq!(h.len(), 300);
        assert_eq!(h.last().unwrap().ip, "10.0.1.149");
        assert_eq!(h[0].ip, "10.0.0.100");
    }

    #[test]
    fn summary_lines() {
        // один адрес: держится 2 ч 5 мин (7500 с)
        let s = summary(&[st("1.1.1.1", 1_000_000, 1_007_000)], 1_007_500);
        assert_eq!(s.len(), 2);
        assert!(s[0].ends_with("(2 ч 5 мин)"), "{}", s[0]);
        assert!(s[1].contains("пока не видел"));
        // три адреса; между вторым и третьим 2 ч без наблюдений
        let h = [st("1.1.1.1", 1_000_000, 1_000_600), st("2.2.2.2", 1_000_900, 1_004_500), st("3.3.3.3", 1_011_700, 1_011_700)];
        let s = summary(&h, 1_011_760);
        assert!(s[0].ends_with("(1 мин)"), "{}", s[0]);
        assert_eq!(s[1], "Смен за сутки: 2 · за неделю: 2 · за 30 дней: 2");
        // свежие сверху: сначала 2.2.2.2 (держался 3600 с = 1 ч 0 мин, потом разрыв 7200 с), затем 1.1.1.1 (10 мин)
        assert!(s[2].contains(": 2.2.2.2 DE (1 ч 0 мин, потом без наблюдений 2 ч 0 мин)"), "{}", s[2]);
        assert!(s[3].contains(": 1.1.1.1 DE (10 мин)"), "{}", s[3]);
        assert_eq!(s.len(), 4);
        assert!(summary(&[], 5).is_empty());
    }

    #[test]
    fn file_round_trip_and_garbage() {
        let h = vec![st("1.1.1.1", 10, 20), st("2001:db8::1", 30, 40)];
        assert_eq!(parse(&dump(&h)), h);
        assert!(parse("не json").is_empty());
        assert!(parse(r#"{"ip":"1.1.1.1"}"#).is_empty());
        // пустой адрес, время задом наперёд, нулевое время — выбрасываются, годная запись остаётся
        let t = r#"[{"ip":"","from":1,"to":2},{"ip":"a","from":9,"to":2},{"ip":"b","from":0,"to":2},{"ip":"c","from":3,"to":4,"country":7}]"#;
        assert_eq!(parse(t), vec![Stay { ip: "c".into(), country: String::new(), from: 3, to: 4 }]);
    }

    #[test]
    fn note_writes_only_into_the_test_folder() {
        let _g = crate::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let p = path();
        assert!(!p.starts_with(crate::store::home_dir().unwrap_or_default().join(".config")), "{}", p.display());
        let _ = std::fs::remove_file(&p);
        assert_eq!(note("1.1.1.1", "DE", 1_000_000).len(), 2);
        assert_eq!(note("2.2.2.2", "FR", 1_000_900).len(), 3);
        assert_eq!(parse(&std::fs::read_to_string(&p).unwrap()).len(), 2);
        let _ = std::fs::remove_file(&p);
    }
}
