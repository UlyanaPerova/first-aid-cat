//! «Справка от Мяудоктора» — перенос aptechka/spravka.py: шуточный бланк поликлиники по
//! результатам проверок. Текст (диагнозы, нормы, советы) собирает `build`, раскладку бланка в
//! список «что где нарисовать» — `layout`, рисует его `draw`. Раскладка считается один раз при
//! открытии справки, а не при каждой перерисовке, поэтому лист не тормозит, пока кот дышит под ним.
//!
//! Отличие от Python: нет шрифтов Liberation Serif / Marck Script (в программе вшит только DejaVu
//! Sans Condensed), поэтому курсива нет, а подпись «Даня» набрана жирным крупным шрифтом.

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
    Text { x: i32, base: i32, s: String, col: u32, px: u16, bold: bool },
    Rule { x: i32, y: i32, w: i32, thick: i32 },
    Paw { x: i32, y: i32, size: f32 },
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
        self.flow(&[sp(&format!("{label}:  "), INK, false), sp(value, BLUE_INK, true)], M, W - 2 * M, 15, Align::Left, 4);
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
    let (sx, sy) = (b.l(M + 120), doctor_line_y - b.l(16));
    let base = b.fonts.center_base(sy, b.l(30), b.l(30) as u16);
    b.items.push(Item::Text { x: sx, base, s: "Даня".into(), col: BLUE_INK, px: b.l(30) as u16, bold: true });
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
        }
    }
}

/// Лист целиком в RGB (для «Сохранить как картинку»).
pub fn render_rgb(fonts: &mut Fonts, doc: &Doc, pw: i32) -> (usize, usize, Vec<u8>) {
    let page = layout(fonts, doc, pw);
    let (w, h) = (page.w.max(1) as usize, page.h.max(1) as usize);
    let mut buf = vec![0u32; w * h];
    {
        let mut c = Canvas::new(&mut buf, w, w as i32, h as i32);
        draw(&page, &mut c, fonts, 0, 0);
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
}
