//! «Справка от Мяудоктора» — перенос aptechka/spravka.py: шуточный бланк поликлиники по
//! результатам проверок. Текст (диагнозы, нормы, советы) собирает `build`, раскладку бланка в
//! список «что где нарисовать» — `layout`, рисует его `draw`. Раскладка считается один раз при
//! открытии справки, а не при каждой перерисовке, поэтому лист не тормозит, пока кот дышит под ним.
//!
//! Отличие от Python: нет шрифтов Liberation Serif / Marck Script (в программе вшит только DejaVu
//! Sans Condensed), поэтому курсива нет; подпись врача — картинка от руки (assets/подпись_*.png).

use std::collections::BTreeMap;

use crate::diag::{Check, Session};
use crate::gfx::{Canvas, Fonts, Rect};
use crate::ui::{localtime, now_secs};

pub const DOCTOR: &str = "Мяудоктор Даня Н.Я.";
const SENIOR: &str = "старшему лечащему врачу Claude A.";

/// Нормы показателей (что считается здоровым), по началу названия проверки.
const NORMS: &[(&str, &str)] = &[
    (
        "Пинг",
        "до 100 мс — отлично; 100–250 мс — нормально для VPN; больше 250 мс — видео и \
             звонки начнут подлагивать. Джиттер (насколько пинг «скачет»): до 30 мс — хорошо, \
             до 60 мс — терпимо, больше — связь рваная.",
    ),
    (
        "Скорость ↓ скачивание",
        "от 20 Мбит/с — хватает даже на 4K-видео; 5–20 Мбит/с — HD без \
                              проблем; меньше 5 Мбит/с — тормоза.",
    ),
    ("Скорость ↑ отдача", "от 5 Мбит/с — видеозвонки и отправка файлов без проблем."),
    ("DNS", "ответ быстрее 50 мс."),
    ("Температура CPU", "до 85 °C под нагрузкой, в покое 35–60 °C."),
    ("Видеокарта", "до 80 °C, загрузка в покое — несколько процентов."),
    ("Память", "занято до 85 %."),
    ("Диск", "свободно больше 20 ГБ."),
    ("Системное время", "расхождение с точным временем меньше 90 секунд."),
    ("Без перезагрузки", "до 14 дней."),
    ("Процессор", "загрузка в покое до 30 %."),
    ("Связь с Claude API", "ответ быстрее 1500 мс."),
    ("Связь с claude.ai", "ответ быстрее 1500 мс."),
    ("Доступ к репозиториям", "отвечают все источники обновлений."),
];

/// Короткая норма для таблицы показателей.
const NORM_SHORT: &[(&str, &str)] = &[
    ("Пинг", "до 250 мс (через VPN), джиттер до 60 мс"),
    ("Скорость ↓ скачивание", "от 20 Мбит/с"),
    ("Скорость ↑ отдача", "от 5 Мбит/с"),
    ("DNS", "до 50 мс"),
    ("Температура CPU", "до 85 °C"),
    ("Видеокарта", "до 80 °C"),
    ("Память", "занято до 85 %"),
    ("Диск", "свободно от 20 ГБ"),
    ("Системное время", "расхождение до 90 с"),
    ("Доступ к репозиториям", "отвечают все"),
];

/// Что делать, если приложение само не лечит (по началу названия проверки).
const ADVICE: &[(&str, &str)] = &[
    (
        "Пинг",
        "Переключитесь на ближайший сервер VPN (Европа обычно быстрее Америки); закройте \
             торренты и большие загрузки; по возможности подключитесь кабелем вместо Wi-Fi. \
             Самопроверка: в терминале «ping -c 30 1.1.1.1» — смотрите avg (средний пинг) и \
             mdev (джиттер).",
    ),
    (
        "Скорость",
        "Сравните со скоростью без VPN: выключите его на минуту и нажмите «Измерить \
                 скорость». Если без VPN быстро — смените сервер VPN. Если медленно и без \
                 VPN — перезагрузите роутер и проверьте тариф. Независимая проверка: \
                 speedtest.net в браузере.",
    ),
    (
        "Серверы Anthropic",
        "Лечения не требуется: недомогание на стороне Anthropic. Наблюдать \
                          на status.claude.com, повторить приём через час.",
    ),
    (
        "Связь с Claude",
        "Проверьте, включён ли VPN; если включён — переподключитесь к другому \
                       серверу.",
    ),
    (
        "Доступ к репозиториям",
        "Проверьте вручную: «sudo apt update» покажет, какой источник \
                              не отвечает. Часто помогает выключить или включить VPN.",
    ),
    (
        "Системное время",
        "Включите автосверку времени (кнопка в Аптечке) и проверьте часовой \
                        пояс в Настройках → Дата и время.",
    ),
    (
        "Температура",
        "Проверьте, не забиты ли пылью вентиляторы, и не работает ли что-то тяжёлое \
                    в фоне (Монитор системы).",
    ),
    ("Видеокарта", "Если горячо в покое — посмотрите, кто нагружает видеокарту: «nvidia-smi»."),
    (
        "Память",
        "Закройте лишние вкладки и программы; посмотреть, кто ест память, — Монитор \
               системы.",
    ),
    (
        "Диск",
        "Очистите Загрузки и корзину; большие папки покажет «Анализатор использования \
             дисков» (baobab).",
    ),
    ("Без перезагрузки", "Перезагрузите компьютер — это лечит много странностей."),
    ("Упавшие службы", "Подробности: «systemctl --failed», затем «journalctl -u <имя службы>»."),
    ("Тема", "Если тема пропала после обновления — переустановите WhiteSur/Tahoe."),
    ("Расширение", "Если не помог перезапуск — обновите расширение на extensions.gnome.org."),
    ("Канал", "Откройте клиент VPN и подключитесь заново."),
    ("VPN", "Откройте Happ и нажмите «подключиться»."),
    (
        "Кто рисует экран",
        "Перезагрузите компьютер; если повторяется — переключите на NVIDIA \
                         (кнопка в Аптечке).",
    ),
];

/// Сложные случаи — направление к старшему врачу.
const COMPLEX: &[&str] =
    &["Драйвер NVIDIA", "Маршрут", "Прокси в /etc", "Упавшие службы", "Конфликт VPN", "Тема окон", "Тема панели", "Иконки", "Курсор"];

/// Показатели в таблице «Основные показатели», в этом порядке.
const VITALS: &[&str] = &[
    "Пинг",
    "Скорость ↓ скачивание",
    "Скорость ↑ отдача",
    "DNS",
    "Температура CPU",
    "Видеокарта",
    "Память",
    "Диск",
    "Системное время",
    "Доступ к репозиториям",
];

pub struct Finding {
    pub severity: &'static str,
    pub status: String,
    pub title: String,
    pub value: String,
    pub norm: String,
    pub procedure: String,
    pub advice: String,
}

pub struct Doc {
    pub number: String,
    pub date: String,
    pub time: String,
    pub owner: String,
    pub patient: String,
    pub os: String,
    pub age: String,
    pub healthy: bool,
    /// Сколько проверок не смогли выполниться: «здоров» тогда не пишем.
    pub unchecked: usize,
    pub claude: &'static str,
    pub findings: Vec<Finding>,
    /// (показатель, результат, оценка, норма)
    pub vitals: Vec<(String, String, &'static str, &'static str)>,
    pub referral: Vec<String>,
}

fn lookup<'a>(table: &'a [(&str, &str)], title: &str) -> Option<&'a str> {
    table.iter().find(|(k, _)| title.starts_with(k)).map(|(_, v)| *v)
}

/// Кому выдаётся справка (можно поменять в ~/.config/aptechka/owner.txt).
/// Кому выдана справка: строка из owner.txt в папке настроек (например, «хозяйке Анне Сергеевне»);
/// личных имён в программе нет — без файла «хозяину компьютера».
fn owner() -> String {
    // только обычный файл и не больше 4 КБ (FIFO или огромный файл не повесят окно), имя — не длиннее 120 знаков
    match crate::store::read_capped(&crate::prefs::config_dir().join("owner.txt"), 4096) {
        Ok(s) if !s.trim().is_empty() => crate::diag::term_safe(s.trim()).chars().take(120).collect(),
        _ => "хозяину компьютера".to_string(),
    }
}

/// Система, ядро и имя компьютера — у каждой системы свои источники.
#[cfg(target_os = "macos")]
fn machine() -> (String, String, String) {
    let r = |c: &[&str]| crate::diag::run(c, 5).1.trim().to_string();
    let os = format!("{} {}", r(&["sw_vers", "-productName"]), r(&["sw_vers", "-productVersion"]));
    (os.trim().to_string(), format!("Darwin {}", r(&["uname", "-r"])), r(&["scutil", "--get", "ComputerName"]))
}

#[cfg(windows)]
fn machine() -> (String, String, String) {
    // «Microsoft Windows [Version 10.0.26100.1234]»
    let ver = crate::diag::run(&["cmd", "/c", "ver"], 5).1;
    let num = ver.split("Version").nth(1).unwrap_or("").trim().trim_end_matches(']').trim().to_string();
    let build: u32 = num.split('.').nth(2).and_then(|b| b.parse().ok()).unwrap_or(0);
    let os = if build >= 22000 { "Windows 11" } else { "Windows 10" };
    (os.to_string(), format!("NT {num}"), std::env::var("COMPUTERNAME").unwrap_or_default())
}

#[cfg(target_os = "linux")]
fn machine() -> (String, String, String) {
    (os_name(), read_trim("/proc/sys/kernel/osrelease"), read_trim("/proc/sys/kernel/hostname"))
}

/// Сколько без перезагрузки — если в проверках такой строки нет (macOS, Windows).
fn uptime_text() -> Option<String> {
    #[cfg(target_os = "macos")]
    let secs = {
        // «{ sec = 1793612345, usec = 0 } …»
        let b = crate::diag::run(&["sysctl", "-n", "kern.boottime"], 5).1;
        let boot: i64 = b.split("sec =").nth(1)?.split(',').next()?.trim().parse().ok()?;
        now_secs() - boot
    };
    #[cfg(windows)]
    let secs = {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetTickCount64() -> u64;
        }
        (unsafe { GetTickCount64() } / 1000) as i64
    };
    #[cfg(target_os = "linux")]
    let secs: i64 = read_trim("/proc/uptime").split('.').next()?.parse().ok()?;
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    Some(if d > 0 { format!("{d} д {h} ч") } else { format!("{h} ч {m} мин") })
}

/// Название системы для шапки: «Linux», «macOS», «Windows».
fn os_family() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(windows) {
        "Windows"
    } else {
        "Linux"
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn os_name() -> String {
    let txt = crate::store::read_text_lossy("/etc/os-release").unwrap_or_default();
    txt.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME=\""))
        .and_then(|r| r.split('"').next())
        .filter(|s| !s.is_empty())
        .unwrap_or("Linux")
        .to_string()
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn read_trim(path: &str) -> String {
    crate::store::read_text_lossy(path).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Собрать содержимое справки по результатам проверок.
pub fn build(results: &[Check], sessions: &[Session]) -> Doc {
    let tm = localtime(now_secs());
    let year = tm.year;
    let bad: Vec<&Check> = results.iter().filter(|r| r.status == "bad").collect();
    let warn: Vec<&Check> = results.iter().filter(|r| r.status == "warn").collect();
    let age = results.iter().rev().find(|r| r.title == "Без перезагрузки").map(|r| r.value.clone());
    let (os, kernel, host) = machine();
    let mut doc = Doc {
        number: format!("{:02}{:02}{:02}-{:02}{:02}", tm.mday, tm.mon, year % 100, tm.hour, tm.min),
        date: format!("{:02}.{:02}.{year}", tm.mday, tm.mon),
        time: format!("{:02}:{:02}", tm.hour, tm.min),
        owner: owner(),
        patient: format!("компьютер «{}»", host),
        os: format!("{os}, ядро {kernel}"),
        age: age.or_else(uptime_text).unwrap_or_else(|| "—".to_string()),
        // «здоров» — только если проверки есть и все пройдены (пустой набор или всё «проверяю» — не здоровье)
        unchecked: crate::diag::unchecked_count(results),
        healthy: crate::diag::unchecked_count(results) == 0
            && bad.is_empty()
            && warn.is_empty()
            && results.iter().any(|r| r.status == "ok")
            && !results.iter().any(|r| r.status == "checking"),
        claude: if sessions.iter().any(|s| s.live) { "на смене" } else { "отдыхает" },
        findings: Vec::new(),
        vitals: Vec::new(),
        referral: Vec::new(),
    };
    for r in bad.iter().chain(warn.iter()) {
        let fix = r.fixes.first().map(|f| f.label.as_str());
        doc.findings.push(Finding {
            severity: if r.status == "bad" { "Диагноз" } else { "Замечание" },
            status: r.status.clone(),
            title: r.title.clone(),
            value: r.value.clone(),
            norm: lookup(NORMS, &r.title).unwrap_or("").to_string(),
            procedure: fix.map(|l| format!("в Аптечке нажать «{l}»")).unwrap_or_default(),
            advice: lookup(ADVICE, &r.title).map(String::from).unwrap_or_else(|| r.detail.clone()),
        });
        if r.status == "bad" && (COMPLEX.iter().any(|c| r.title.starts_with(c)) || r.fixes.is_empty()) {
            doc.referral.push(r.title.clone());
        }
    }
    for title in VITALS {
        if let Some(r) = results.iter().rev().find(|r| r.title == *title) {
            let mark = match r.status.as_str() {
                "ok" => "в норме",
                "warn" => "погранично",
                "bad" => "вне нормы",
                _ => "—",
            };
            doc.vitals.push((title.to_string(), r.value.clone(), mark, lookup(NORM_SHORT, title).unwrap_or("")));
        }
    }
    doc
}

// ───────────────────────────── раскладка бланка ─────────────────────────────

const INK: u32 = 0x1F2133;
const BLUE_INK: u32 = 0x1F338C;
const GREY: u32 = 0x61616B;
const RED: u32 = 0xB3001B;
const AMBER: u32 = 0x8A5A00;
const GREEN: u32 = 0x1A7333;
const PINK: u32 = 0xED6B9E;
const PAPER: u32 = 0xFCFAF2;

/// Логическая ширина листа и поля (как в Python: W = 820, M = 56).
pub const W: i32 = 820;
const M: i32 = 56;

pub enum Item {
    Text {
        x: i32,
        base: i32,
        s: String,
        col: u32,
        px: u16,
        bold: bool,
    },
    Rule {
        x: i32,
        y: i32,
        w: i32,
        thick: i32,
    },
    Paw {
        x: i32,
        y: i32,
        size: f32,
    },
    /// Подпись врача от руки (картинка): левый верхний угол и высота русской подписи.
    Sign {
        x: i32,
        y: i32,
        h: i32,
        en: bool,
    },
    /// Закрашенный прямоугольник (столбики аналитики).
    Box {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        col: u32,
    },
}

pub struct Page {
    pub items: Vec<Item>,
    pub w: i32,
    pub h: i32,
    /// Во сколько раз логический пиксель увеличен.
    pub scale: f32,
}

/// Кусок строки одного цвета и начертания.
struct Span {
    t: String,
    col: u32,
    bold: bool,
}

/// Перевод (i18n.rs) — здесь, целым куском: flow режет абзац на слова, а по одному слову фразу не перевести.
fn sp(t: &str, col: u32, bold: bool) -> Span {
    Span { t: crate::i18n::t(t), col, bold }
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Center,
}

struct B<'a> {
    fonts: &'a mut Fonts,
    items: Vec<Item>,
    y: i32,
    s: f32,
}

impl B<'_> {
    fn l(&self, n: i32) -> i32 {
        (n as f32 * self.s).round() as i32
    }

    /// Абзац из кусков разного цвета с переносом по словам. Размеры — логические пиксели.
    fn flow(&mut self, spans: &[Span], x: i32, width: i32, px: i32, align: Align, gap: i32) {
        let (lx, lw, pxs) = (self.l(x), self.l(width), self.l(px).max(1) as u16);
        let lh = (pxs as f32 * 1.3).round() as i32;
        let space = self.fonts.width(" ", pxs, false);
        // слова: (текст, цвет, жирность, ширина, нужен ли пробел перед словом)
        let mut words: Vec<(String, u32, bool, i32, bool)> = Vec::new();
        let mut pending = false;
        for sp in spans {
            if sp.t.is_empty() {
                continue;
            }
            if sp.t.starts_with(char::is_whitespace) {
                pending = true;
            }
            for w in sp.t.split_whitespace() {
                let ww = self.fonts.width(w, pxs, sp.bold);
                words.push((w.to_string(), sp.col, sp.bold, ww, pending && !words.is_empty()));
                pending = true;
            }
            pending = sp.t.ends_with(char::is_whitespace);
        }
        // строки
        let mut lines: Vec<Vec<(String, u32, bool, i32, bool)>> = vec![Vec::new()];
        let mut cur_w = 0;
        for w in words {
            let add = w.3 + if w.4 { space } else { 0 };
            let starts_line = lines.last().is_some_and(|l| l.is_empty());
            if !starts_line && cur_w + add > lw {
                lines.push(Vec::new());
                cur_w = 0;
            }
            let add = if lines.last().is_some_and(|l| l.is_empty()) { w.3 } else { add };
            cur_w += add;
            if let Some(l) = lines.last_mut() {
                l.push(w);
            }
        }
        for line in &lines {
            let total: i32 = line.iter().enumerate().map(|(i, w)| w.3 + if i > 0 && w.4 { space } else { 0 }).sum();
            let mut pen = if align == Align::Center { lx + (lw - total) / 2 } else { lx };
            let base = self.fonts.center_base(self.y, lh, pxs);
            for (i, (t, col, bold, ww, sp_before)) in line.iter().enumerate() {
                if i > 0 && *sp_before {
                    pen += space;
                }
                self.items.push(Item::Text { x: pen, base, s: t.clone(), col: *col, px: pxs, bold: *bold });
                pen += ww;
            }
            self.y += lh;
        }
        self.y += self.l(gap);
    }

    fn text(&mut self, t: &str, px: i32, bold: bool, col: u32, align: Align, gap: i32) {
        self.flow(&[sp(t, col, bold)], M, W - 2 * M, px, align, gap);
    }

    fn rule(&mut self, thick: i32, gap: i32) {
        let y = self.y;
        self.items.push(Item::Rule { x: self.l(M), y, w: self.l(W - 2 * M), thick: self.l(thick).max(1) });
        self.y += self.l(gap);
    }

    fn field(&mut self, label: &str, value: &str) {
        // подпись переводится до двоеточия: «Сегодня» есть в таблице, а «Сегодня:  » — нет
        self.flow(&[sp(&format!("{}:  ", crate::i18n::t(label)), INK, false), sp(value, BLUE_INK, true)], M, W - 2 * M, 15, Align::Left, 4);
    }

    fn detail(&mut self, label: &str, text: &str, col: u32) {
        self.flow(&[sp(label, GREY, true), sp(&format!(" {text}"), col, false)], M + 22, W - 2 * M - 22, 14, Align::Left, 3);
    }
}

/// Раскладка листа шириной `pw` пикселей (логическая ширина W растягивается до неё).
pub fn layout(fonts: &mut Fonts, doc: &Doc, pw: i32) -> Page {
    let s = pw as f32 / W as f32;
    let mut b = B { fonts, items: Vec::new(), y: (M as f32 * s).round() as i32, s };
    let c = Align::Center;
    let left = Align::Left;

    b.text("Министерство уюта и котиков · Ветеринарно-компьютерная служба", 13, false, INK, c, 2);
    b.text("ПОЛИКЛИНИКА «АПТЕЧКА» №1", 21, true, INK, c, 2);
    b.text(&format!("отделение диагностики {}-пациентов · приём круглосуточно, без записи", os_family()), 13, false, GREY, c, 8);
    b.rule(2, 4);
    b.rule(1, 18);
    b.text(&format!("СПРАВКА № {}", doc.number), 26, true, INK, c, 2);
    b.text("о состоянии здоровья компьютера (форма 086/мяу)", 14, false, GREY, c, 16);

    b.field("Выдана", &doc.owner);
    b.field("Пациент", &doc.patient);
    b.field("Порода (ОС)", &doc.os);
    b.field("Возраст без перезагрузки", &doc.age);
    b.field("Дежурный ассистент Claude", doc.claude);
    b.y += b.l(10);

    b.text("ЗАКЛЮЧЕНИЕ", 17, true, INK, left, 4);
    if doc.healthy {
        b.text(
            "Пациент осмотрен. В выполненных проверках проблем не найдено: сеть дышит ровно, VPN бьётся ритмично, темы на месте.",
            15,
            false,
            INK,
            left,
            4,
        );
        b.text(
            "Годен к работе, учёбе и просмотру аниме. Справка — о проверенном на момент выдачи, а не обо всём на свете.",
            16,
            true,
            BLUE_INK,
            left,
            14,
        );
    } else if doc.findings.is_empty() && doc.unchecked > 0 {
        b.text(
            "Пациент осмотрен не полностью: проблем в выполненных проверках не найдено, но часть проверок выполнить не удалось. Заключения «здоров» не даю.",
            15,
            false,
            INK,
            left,
            4,
        );
        b.text(&format!("Не удалось проверить: {}", doc.unchecked), 15, true, INK, left, 12);
    } else if doc.findings.is_empty() {
        b.text("Осмотр не закончен: проверки ещё идут или не выполнены. Данных для заключения недостаточно.", 15, false, INK, left, 12);
    } else {
        let nb = doc.findings.iter().filter(|f| f.status == "bad").count();
        let nw = doc.findings.len() - nb;
        b.text(&format!("Пациент осмотрен. Выявлено: диагнозов — {nb}, замечаний — {nw}. Подробности и назначения ниже."), 15, false, INK, left, 12);
        for (i, f) in doc.findings.iter().enumerate() {
            let col = if f.status == "bad" { RED } else { AMBER };
            b.flow(
                &[
                    sp(&format!("{}. {}: {}", i + 1, crate::i18n::tr(f.severity), crate::i18n::tr(&f.title)), INK, true),
                    sp(" — ", INK, false),
                    sp(&f.value, col, false),
                ],
                M,
                W - 2 * M,
                15,
                left,
                3,
            );
            if !f.norm.is_empty() {
                b.detail("Норма:", &f.norm, INK);
            }
            if !f.procedure.is_empty() {
                b.detail("Процедура:", &format!("{}.", f.procedure), INK);
            }
            if !f.advice.is_empty() {
                b.detail("Если не помогло:", &f.advice, INK);
            }
            if doc.referral.contains(&f.title) {
                b.flow(
                    &[
                        sp(&format!("Направление к {SENIOR}"), BLUE_INK, true),
                        sp(" — случай непростой, откройте Claude в терминале и опишите симптомы.", BLUE_INK, false),
                    ],
                    M + 22,
                    W - 2 * M - 22,
                    14,
                    left,
                    3,
                );
            }
            b.y += b.l(8);
        }
    }

    // показатели
    if !doc.vitals.is_empty() {
        b.y += b.l(4);
        b.text("ОСНОВНЫЕ ПОКАЗАТЕЛИ", 17, true, INK, left, 6);
        let tw = W - 2 * M;
        let cols = [(M, 165), (M + 172, 222), (M + 402, 200), (M + 610, tw - 610)];
        let row = |b: &mut B, cells: [&str; 4], pxs: [i32; 4], colors: [u32; 4], bolds: [bool; 4], gap: i32| {
            let y0 = b.y;
            let mut hmax = 0;
            for i in 0..4 {
                b.y = y0;
                let (x, w) = cols[i];
                b.flow(&[sp(cells[i], colors[i], bolds[i])], x, w, pxs[i], Align::Left, 0);
                hmax = hmax.max(b.y - y0);
            }
            b.y = y0 + hmax + b.l(gap);
        };
        row(&mut b, ["Показатель", "Результат", "Норма", "Оценка"], [13; 4], [GREY; 4], [false; 4], 4);
        for (name, val, mark, norm) in &doc.vitals {
            let mcol = match *mark {
                "в норме" => GREEN,
                "погранично" => AMBER,
                "вне нормы" => RED,
                _ => INK,
            };
            row(&mut b, [name, val, norm, mark], [14, 13, 13, 14], [INK, BLUE_INK, GREY, mcol], [false; 4], 5);
        }
        b.y += b.l(10);
    }

    b.text("Справка дана для предъявления по месту требования. Действительна до следующей проверки.", 13, false, GREY, left, 26);

    // подпись, дата, лапка
    let sig_y = b.y;
    b.text(&format!("Дата выдачи: {}, {}", doc.date, doc.time), 15, false, INK, left, 18);
    let doctor_line_y = b.y;
    b.text("Лечащий врач: ____________________", 15, false, INK, left, 4);
    b.flow(&[sp(DOCTOR, INK, false)], M + 108, 300, 14, left, 10);
    let en = crate::i18n::lang() == crate::i18n::Lang::En;
    // подпись — над чертой, сразу за «Лечащий врач:» (на английском надпись длиннее)
    let line = crate::i18n::t("Лечащий врач: ____________________");
    let label = line.split('_').next().unwrap_or("");
    let lx = b.l(M) + b.fonts.width(label, b.l(15) as u16, false);
    b.items.push(Item::Sign { x: lx + b.l(4), y: doctor_line_y - b.l(22), h: b.l(40), en });
    b.items.push(Item::Paw { x: b.l(W - M - 110), y: sig_y + b.l(40), size: 1.25 * s });
    let label = "М.П. (место лапки)";
    let (lpx, lw) = (b.l(12) as u16, b.l(160));
    let tw = b.fonts.width(label, lpx, false);
    let lbase = b.fonts.center_base(sig_y + b.l(92), b.l(16), lpx);
    b.items.push(Item::Text { x: b.l(W - M - 190) + (lw - tw) / 2, base: lbase, s: label.into(), col: GREY, px: lpx, bold: false });
    b.y += b.l(30);
    let h = b.y + b.l(M / 2);
    let items = std::mem::take(&mut b.items);
    Page { items, w: pw, h, scale: s }
}

// ───────────────────────────── медкарта расхода ИИ ─────────────────────────────

/// Данные для «Аналитики расхода»: по агенту — лимиты и расход по дням и моделям.
#[derive(Clone, Debug, Default)]
pub struct Analytics {
    pub agents: Vec<(&'static str, Option<crate::limits::Limits>, crate::limits::Usage, crate::limits::Projects)>,
    pub now: i64,
    /// false — ещё считаем (журналы читаются в фоне)
    pub ready: bool,
}

impl Analytics {
    /// Прочитать журналы (первый раз — около секунды: звать не из потока окна).
    pub fn load() -> Analytics {
        use crate::limits::*;
        let agents =
            vec![("Claude Code", claude_limits(), claude_usage(), claude_projects()), ("Codex", codex_limits(), codex_usage(), codex_projects())]
                .into_iter()
                .filter(|(n, l, u, _)| crate::tray::shown(n) && (l.is_some() || !u.is_empty()))
                .collect();
        Analytics { agents, now: crate::ui::now_secs(), ready: true }
    }
}

/// Цвет столбиков агента.
fn agent_col(i: usize) -> u32 {
    [0xC2652A, 0x2A6FC2, 0x3C9D5D, 0x8A4FBF][i % 4]
}

/// Раскладка «Медкарты расхода ИИ» шириной pw.
pub fn analytics_layout(fonts: &mut Fonts, a: &Analytics, pw: i32) -> Page {
    use crate::limits::{fmt_tokens, local_day, Tok};
    let s = pw as f32 / W as f32;
    let mut b = B { fonts, items: Vec::new(), y: (M as f32 * s).round() as i32, s };
    let (c, left) = (Align::Center, Align::Left);
    b.text("Министерство уюта и котиков · Ветеринарно-компьютерная служба", 13, false, INK, c, 2);
    b.text("МЕДКАРТА РАСХОДА ИИ", 24, true, INK, c, 2);
    b.text("за 30 дней · по журналам агентов на этом компьютере", 13, false, GREY, c, 8);
    b.rule(2, 4);
    b.rule(1, 18);
    if !a.ready {
        b.text("Считаю… (читаю журналы агентов)", 15, false, GREY, left, 10);
    } else if a.agents.is_empty() {
        b.text("Нет данных: Claude Code и Codex на этом компьютере ещё не работали.", 15, false, GREY, left, 10);
    }
    // дни: 30 последних, по местному времени
    let days: Vec<i32> = (0..crate::limits::DAYS).rev().map(|i| local_day(a.now - i * 86_400)).collect();
    let (today, week0) = (local_day(a.now), local_day(a.now - 6 * 86_400));
    for (ai, (name, lim, us, projects)) in a.agents.iter().enumerate() {
        b.text(name, 19, true, agent_col(ai), left, 6);
        // лимиты — полосками: слева что за лимит, справа процент и через сколько сброс, под ними полоса
        if let Some(l) = lim.filter(|l| l.five.is_some() || l.week.is_some()) {
            for (what, w) in [("Лимит на 5 ч", l.five), ("Лимит на неделю", l.week)] {
                let Some(w) = w else { continue };
                let pct = w.now_pct(a.now);
                let col = if pct >= 90.0 {
                    0xD03B3B
                } else if pct >= 70.0 {
                    0xEDA100
                } else {
                    0x0CA30C
                };
                let (x0, x1) = (b.l(M), b.l(W - M));
                let y0 = b.y;
                let p14 = b.l(14).max(1) as u16;
                let base = b.fonts.center_base(y0, b.l(20), p14);
                b.items.push(Item::Text { x: x0, base, s: crate::i18n::t(what), col: INK, px: p14, bold: false });
                let reset = if w.resets_at > a.now {
                    crate::i18n::t(&format!("сброс {}", crate::limits::until(w.resets_at, a.now)))
                } else if w.resets_at > 0 {
                    crate::i18n::t("уже сброшен")
                } else {
                    String::new()
                };
                let p12 = b.l(12).max(1) as u16;
                let rw = b.fonts.width(&reset, p12, false);
                b.items.push(Item::Text { x: x1 - rw, base, s: reset, col: GREY, px: p12, bold: false });
                let t = format!("{pct:.0}%");
                let p16 = b.l(16).max(1) as u16;
                let tw = b.fonts.width(&t, p16, true);
                b.items.push(Item::Text { x: x1 - rw - b.l(12) - tw, base, s: t, col: INK, px: p16, bold: true });
                let by = y0 + b.l(26);
                let bh = b.l(8).max(2);
                b.items.push(Item::Box { x: x0, y: by, w: x1 - x0, h: bh, col: 0xE6E3DA });
                let fw = ((x1 - x0) as f64 * pct / 100.0).round() as i32;
                if fw > 0 {
                    b.items.push(Item::Box { x: x0, y: by, w: fw, h: bh, col });
                }
                b.y = by + bh + b.l(16);
            }
            b.y += b.l(4);
        }
        let mut per_day: BTreeMap<i32, Tok> = BTreeMap::new();
        let mut per_model: BTreeMap<&str, (Tok, Tok)> = BTreeMap::new();
        for ((d, m), t) in us {
            per_day.entry(*d).or_default().add(t);
            let e = per_model.entry(m.as_str()).or_default();
            e.1.add(t);
            if *d >= week0 {
                e.0.add(t);
            }
        }
        let sum = |from: i32| per_day.range(from..).fold(0u64, |acc, (_, t)| acc + t.total());
        b.field("Сегодня", &fmt_tokens(sum(today)));
        b.field("За 7 дней", &fmt_tokens(sum(week0)));
        b.field("За 30 дней", &fmt_tokens(sum(days[0])));
        b.y += b.l(10);
        // столбики по дням
        let max = days.iter().map(|d| per_day.get(d).map_or(0, |t| t.total())).max().unwrap_or(0).max(1);
        let (cx, cw, ch) = (b.l(M), b.l(W - 2 * M), b.l(110));
        let step = cw as f32 / days.len() as f32;
        b.text(&format!("в день, токенов (макс. {})", fmt_tokens(max)), 12, false, GREY, left, 4);
        let top = b.y;
        b.items.push(Item::Rule { x: cx, y: top + ch, w: cw, thick: b.l(1).max(1) });
        for (i, d) in days.iter().enumerate() {
            let v = per_day.get(d).map_or(0, |t| t.total());
            let h = ((v as f64 / max as f64) * ch as f64).round() as i32;
            if h > 0 {
                let x = cx + (i as f32 * step).round() as i32;
                let w = (step * 0.72).round().max(1.0) as i32;
                b.items.push(Item::Box { x, y: top + ch - h, w, h, col: agent_col(ai) });
            }
        }
        b.y = top + ch + b.l(4);
        // подписи дат: первая, середина, последняя
        let lpx = b.l(11).max(1) as u16;
        let base = b.fonts.center_base(b.y, b.l(14), lpx);
        for i in [0, days.len() / 2, days.len() - 1] {
            let d = days[i];
            let t = format!("{:02}.{:02}", d % 100, d / 100 % 100);
            let tw = b.fonts.width(&t, lpx, false);
            let x = (cx + (i as f32 * step + step * 0.36).round() as i32 - tw / 2).clamp(cx, cx + cw - tw);
            b.items.push(Item::Text { x, base, s: t, col: GREY, px: lpx, bold: false });
        }
        b.y += b.l(26);
        // по моделям
        let cols = [(M, 300), (M + 300, 150), (M + 450, 150), (M + 600, W - 2 * M - 600)];
        let row = |b: &mut B, cells: [String; 4], col: u32| {
            let y0 = b.y;
            let mut hmax = 0;
            for (i, t) in cells.iter().enumerate() {
                b.y = y0;
                b.flow(&[sp(t, col, false)], cols[i].0, cols[i].1, 13, Align::Left, 0);
                hmax = hmax.max(b.y - y0);
            }
            b.y = y0 + hmax + b.l(4);
        };
        row(&mut b, ["Модель".into(), "7 дней".into(), "30 дней".into(), "из них ответы".into()], GREY);
        let mut models: Vec<_> = per_model.into_iter().collect();
        models.sort_by_key(|(_, (_, m))| std::cmp::Reverse(m.total()));
        for (m, (w7, m30)) in models {
            row(&mut b, [m.to_string(), fmt_tokens(w7.total()), fmt_tokens(m30.total()), fmt_tokens(m30.output)], INK);
        }
        // по проектам (папкам, где работал агент): самые затратные за 30 дней
        let mut pr: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
        for ((d, p), n) in projects {
            let e = pr.entry(p.as_str()).or_default();
            e.1 += n;
            if *d >= week0 {
                e.0 += n;
            }
        }
        if !pr.is_empty() {
            b.y += b.l(10);
            row(&mut b, ["Проект".into(), "7 дней".into(), "30 дней".into(), "доля".into()], GREY);
            let total: u64 = pr.values().map(|v| v.1).sum::<u64>().max(1);
            let mut list: Vec<_> = pr.into_iter().collect();
            list.sort_by_key(|(_, (_, m))| std::cmp::Reverse(*m));
            let rest: u64 = list.iter().skip(8).map(|(_, (_, m))| m).sum();
            for (p, (w7, m30)) in list.into_iter().take(8) {
                row(&mut b, [p.to_string(), fmt_tokens(w7), fmt_tokens(m30), format!("{:.0}%", m30 as f64 * 100.0 / total as f64)], INK);
            }
            if rest > 0 {
                row(&mut b, ["остальные".into(), String::new(), fmt_tokens(rest), format!("{:.0}%", rest as f64 * 100.0 / total as f64)], GREY);
            }
        }
        b.y += b.l(8);
        b.rule(1, 16);
    }
    if a.ready && !a.agents.is_empty() {
        b.text(
            "Почти весь объём — чтение из кэша: на каждом ответе агент заново читает начало разговора, и это считается \
             токенами, хотя стоит намного дешевле новых. Сколько написала сама модель — колонка «из них ответы».",
            13,
            false,
            GREY,
            left,
            6,
        );
    }
    let h = b.y + b.l(M / 2);
    let items = std::mem::take(&mut b.items);
    Page { items, w: pw, h, scale: s }
}

// ───────────────────────────── рисование ─────────────────────────────

struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.next()
    }
}

/// Эллипс с поворотом, закрашенный полупрозрачным цветом.
fn ellipse(c: &mut Canvas, cx: f32, cy: f32, rx: f32, ry: f32, ang: f32, col: u32, alpha: u32) {
    let (sn, cs) = ang.sin_cos();
    let r = rx.max(ry).ceil() as i32 + 1;
    for dy in -r..=r {
        for dx in -r..=r {
            let (x, y) = (dx as f32, dy as f32);
            let lx = x * cs + y * sn;
            let ly = -x * sn + y * cs;
            if (lx / rx).powi(2) + (ly / ry).powi(2) <= 1.0 {
                c.blend(cx.round() as i32 + dx, cy.round() as i32 + dy, col, alpha);
            }
        }
    }
}

/// Розовый отпечаток лапки вместо печати (слегка «смазанный», как штамп).
fn paw(c: &mut Canvas, x: i32, y: i32, size: f32) {
    if c.clip.is_empty() {
        return;
    }
    let ang = -0.35f32;
    let (sn, cs) = ang.sin_cos();
    let mut rnd = Lcg(7);
    let pads: [(f32, f32, f32, f32); 5] =
        [(0.0, 18.0, 26.0, 22.0), (-28.0, -14.0, 10.0, 13.0), (-11.0, -30.0, 10.0, 13.0), (11.0, -30.0, 10.0, 13.0), (28.0, -14.0, 10.0, 13.0)];
    for (px, py, rx, ry) in pads {
        for _ in 0..3 {
            // несколько полупрозрачных слоёв — фактура штемпельной краски
            let (jx, jy) = (px + rnd.range(-1.0, 1.0), py + rnd.range(-1.0, 1.0));
            let (ox, oy) = ((jx * cs - jy * sn) * size, (jx * sn + jy * cs) * size);
            ellipse(c, x as f32 + ox, y as f32 + oy, rx * size, ry * size, ang, PINK, 90);
        }
    }
    // «непропечатанные» точки
    for _ in 0..40 {
        let (px, py, rx, ry) = pads[(rnd.next() * 5.0) as usize % 5];
        let (a, d) = (rnd.range(0.0, std::f32::consts::TAU), rnd.range(0.0, 0.9));
        let (jx, jy) = (px + a.cos() * rx * d, py + a.sin() * ry * d);
        let (ox, oy) = ((jx * cs - jy * sn) * size, (jx * sn + jy * cs) * size);
        let rr = rnd.range(0.6, 1.6) * size;
        ellipse(c, x as f32 + ox, y as f32 + oy, rr, rr, 0.0, 0xFFFFFF, 140);
    }
}

/// Нарисовать лист; (ox, oy) — где стоит его левый верхний угол (oy может быть выше окна — прокрутка).
pub fn draw(page: &Page, c: &mut Canvas, fonts: &mut Fonts, ox: i32, oy: i32) {
    let sheet = Rect::new(ox, oy, page.w, page.h);
    if c.clip.intersect(sheet).is_empty() {
        return;
    }
    c.fill(sheet, PAPER);
    // «волокна» бумаги
    let mut rnd = Lcg(1);
    for _ in 0..160 {
        let (x, y) = (rnd.range(0.0, page.w as f32), rnd.range(0.0, page.h as f32));
        let (dx, dy) = (rnd.range(-20.0, 20.0) * page.scale, rnd.range(-3.0, 3.0) * page.scale);
        let steps = dx.abs().max(1.0) as i32;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            c.blend(ox + (x + dx * t) as i32, oy + (y + dy * t) as i32, 0x998C73, 15);
        }
    }
    for it in &page.items {
        match it {
            Item::Text { x, base, s, col, px, bold } => fonts.draw(c, ox + x, oy + base, s, *col, *px, *bold),
            Item::Rule { x, y, w, thick } => c.fill(Rect::new(ox + x, oy + y, *w, *thick), INK),
            Item::Paw { x, y, size } => paw(c, ox + x, oy + y, *size),
            Item::Sign { x, y, h, en } => sign(c, ox + x, oy + y, *h, *en),
            Item::Box { x, y, w, h, col } => c.fill(Rect::new(ox + x, oy + y, *w, *h), *col),
        }
    }
}

/// Подписи врача (нарисованы ChatGPT): маска чернил в оттенках серого, белое — чернила. Обе в одном
/// масштабе, у «Danya» хвост «y» уходит ниже, поэтому она выше.
const SIGN_RU: &[u8] = include_bytes!("../assets/подпись_ru.png");
const SIGN_EN: &[u8] = include_bytes!("../assets/подпись_en.png");

/// Маска подписи: (ширина, высота, покрытие 0..255). Разбирается один раз.
fn sign_mask(en: bool) -> Option<&'static (usize, usize, Vec<u8>)> {
    static M: [std::sync::OnceLock<Option<(usize, usize, Vec<u8>)>>; 2] = [std::sync::OnceLock::new(), std::sync::OnceLock::new()];
    M[en as usize]
        .get_or_init(|| {
            let pm = crate::gfx::decode_png(if en { SIGN_EN } else { SIGN_RU })?;
            Some((pm.width() as usize, pm.height() as usize, pm.data().chunks(4).map(|p| p[0]).collect()))
        })
        .as_ref()
}

/// Подпись синими чернилами: масштаб — по высоте русской (h), сглаживание — средним по клетке.
fn sign(c: &mut Canvas, x: i32, y: i32, h: i32, en: bool) {
    let (Some(ru), Some((sw, sh, m))) = (sign_mask(false), sign_mask(en)) else { return };
    let f = h.max(1) as f32 / ru.1 as f32;
    let (dw, dh) = ((*sw as f32 * f).round() as i32, (*sh as f32 * f).round() as i32);
    let n = ((1.0 / f).ceil() as usize).clamp(1, 8);
    for dy in 0..dh {
        for dx in 0..dw {
            let mut sum = 0u32;
            for j in 0..n {
                for i in 0..n {
                    let sx = (((dx as f32 + (i as f32 + 0.5) / n as f32) / f) as usize).min(sw - 1);
                    let sy = (((dy as f32 + (j as f32 + 0.5) / n as f32) / f) as usize).min(sh - 1);
                    sum += m[sy * sw + sx] as u32;
                }
            }
            let a = sum / (n * n) as u32;
            if a > 0 {
                c.blend(x + dx, y + dy, BLUE_INK, a);
            }
        }
    }
}

/// Лист целиком в RGB (для «Сохранить как картинку»).
pub fn render_rgb(fonts: &mut Fonts, doc: &Doc, pw: i32) -> (usize, usize, Vec<u8>) {
    let page = layout(fonts, doc, pw);
    page_rgb(fonts, &page)
}

/// Готовый лист в RGB.
pub fn page_rgb(fonts: &mut Fonts, page: &Page) -> (usize, usize, Vec<u8>) {
    let (w, h) = (page.w.max(1) as usize, page.h.max(1) as usize);
    let mut buf = vec![0u32; w * h];
    {
        let mut c = Canvas::new(&mut buf, w, w as i32, h as i32);
        draw(page, &mut c, fonts, 0, 0);
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for p in buf {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
    }
    (w, h, rgb)
}

#[cfg(test)]
mod unchecked_tests {
    use super::*;

    #[test]
    fn not_healthy_if_unchecked() {
        let ok = Check::new("sys", "Память", "ok", "40%");
        assert!(build(std::slice::from_ref(&ok), &[]).healthy);
        let d = build(&[ok, Check::new("sys", "Диск", "info", "не проверить").unknown()], &[]);
        assert!(!d.healthy);
        assert_eq!(d.unchecked, 1);
    }

    /// Подпись врача на месте на обоих языках: синие чернила у строки «Лечащий врач».
    /// SPRAVKA_SHOT=папка — заодно сохранить листы картинками (посмотреть глазами).
    #[test]
    fn signature_drawn_in_both_languages() {
        for lang in [crate::i18n::Lang::Ru, crate::i18n::Lang::En] {
            crate::i18n::TEST_LANG.with(|l| l.set(lang));
            let doc = build(&[Check::new("sys", "Память", "ok", "40%")], &[]);
            let mut fonts = Fonts::new();
            let (w, h, rgb) = render_rgb(&mut fonts, &doc, 760);
            let ink = rgb.chunks(3).filter(|p| p[2] > 120 && p[0] < 90 && p[1] < 110).count();
            assert!(ink > 300, "{lang:?}: подписи не видно ({ink})");
            if let Some(dir) = std::env::var_os("SPRAVKA_SHOT") {
                let f = std::fs::File::create(std::path::Path::new(&dir).join(format!("{lang:?}.png"))).unwrap();
                let mut e = png::Encoder::new(f, w as u32, h as u32);
                e.set_color(png::ColorType::Rgb);
                e.write_header().unwrap().write_image_data(&rgb).unwrap();
            }
        }
        crate::i18n::TEST_LANG.with(|l| l.set(crate::i18n::Lang::Ru));
    }
}
