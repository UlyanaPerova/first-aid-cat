//! Реплики кота на тычки мышкой: показываются в облачке на 4 секунды.
//! Сами фразы — в простом текстовом файле engine/assets/фразы_кота.txt (вшит в программу); свой файл
//! с тем же именем в папке настроек Аптечки читается при каждом запуске — править можно без пересборки.

use std::collections::HashMap;

/// Фразы по умолчанию (вшиты при сборке).
const DEFAULT: &str = include_str!("../assets/фразы_кота.txt");

/// Группы фраз из текста: «[Ухо]» — заголовок, дальше по фразе на строку; «#» и пустые — пропуск.
/// Кавычки вокруг фразы (если их поставили по привычке) снимаются.
fn parse(text: &'static str) -> HashMap<String, Vec<&'static str>> {
    let mut out: HashMap<String, Vec<&'static str>> = HashMap::new();
    let mut cur = String::new();
    for line in text.lines() {
        let l = line.trim().trim_start_matches('\u{feff}');
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(name) = l.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            cur = name.trim().to_lowercase();
            continue;
        }
        let q = l.trim_end_matches(',').trim_matches(|c| c == '"' || c == '«' || c == '»').trim();
        if !cur.is_empty() && !q.is_empty() {
            out.entry(cur.clone()).or_default().push(q);
        }
    }
    out
}

/// Свой файл фраз (папка настроек), иначе вшитый. Каждая группа, которой в своём файле нет, — из вшитого.
fn load() -> HashMap<String, Vec<&'static str>> {
    let mut all = parse(DEFAULT);
    let own = crate::prefs::config_dir().join("фразы_кота.txt");
    if let Ok(t) = crate::store::read_text(&own) {
        // строки живут до конца программы (фраз немного, читаем раз за запуск)
        let t: &'static str = Box::leak(t.into_boxed_str());
        for (k, v) in parse(t) {
            all.insert(k, v);
        }
    }
    all
}

pub struct Quips {
    last: &'static str,
    seed: u64,
    groups: HashMap<String, Vec<&'static str>>,
}

impl Quips {
    pub fn new() -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1) | 1;
        Quips { last: "", seed, groups: load() }
    }

    /// Реплика на тычок: `layer` — имя слоя, `what` — реакция из Life::poke.
    pub fn pick(&mut self, layer: &str, what: &str) -> Option<&'static str> {
        let group = if what.starts_with("смущение") || what == "уже смущается" {
            if layer == "тело" {
                "ножки"
            } else {
                "ухо"
            }
        } else if what.starts_with("возмущение") {
            "голова"
        } else if what.starts_with("беспокойство") {
            "хвост"
        } else if what.starts_with("растение") {
            "растения"
        } else {
            return None;
        };
        let pool = self.groups.get(group).filter(|v| !v.is_empty())?;
        // xorshift; одна и та же реплика два раза подряд не выпадает
        for _ in 0..8 {
            self.seed ^= self.seed << 13;
            self.seed ^= self.seed >> 7;
            self.seed ^= self.seed << 17;
            let q = pool[(self.seed % pool.len() as u64) as usize];
            if q != self.last {
                self.last = q;
                return Some(q);
            }
        }
        Some(self.last)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn default_phrases_parse() {
        let g = super::parse(super::DEFAULT);
        for k in ["ухо", "ножки", "голова", "хвост", "растения"] {
            assert!(g.get(k).is_some_and(|v| !v.is_empty()), "нет группы {k}");
        }
        assert!(g["голова"].contains(&"Ну всё, я обиделся."));
    }
}
