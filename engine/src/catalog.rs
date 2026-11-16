//! Каталог показателей: что проверяет Аптечка, по категориям.
//!
//! Показатель — одна проверка (функция), которая даёт одну или несколько строк в таблице.
//! У каждого есть код (для настроек), категория (вкладка), название и пояснение для человека.
//! Выключенные в «Показателях» не запускаются вовсе — экономят время и процессор.
//! Список свой для каждой системы (Linux, Windows, macOS).

use crate::diag::Check;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Net,
    Ai,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Gpu,
    Dev,
    Look,
    Sys,
}

/// Категории в порядке вкладок. «Графика» — только в Linux: на macOS и Windows в ней нет проверок.
#[cfg(target_os = "linux")]
pub const CATS: [Cat; 6] = [Cat::Net, Cat::Ai, Cat::Gpu, Cat::Dev, Cat::Look, Cat::Sys];
#[cfg(not(target_os = "linux"))]
pub const CATS: [Cat; 5] = [Cat::Net, Cat::Ai, Cat::Dev, Cat::Look, Cat::Sys];

impl Cat {
    /// Ключ группы в строках проверок (Check::key).
    pub fn key(self) -> &'static str {
        match self {
            Cat::Net => "net",
            Cat::Ai => "ai",
            Cat::Gpu => "gpu",
            Cat::Dev => "dev",
            Cat::Look => "theme",
            Cat::Sys => "sys",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Cat::Net => "Сеть",
            Cat::Ai => "ИИ",
            Cat::Gpu => "Графика",
            Cat::Dev => "Устройства",
            Cat::Look => "Темы",
            Cat::Sys => "Система",
        }
    }
    /// Пояснение категории для вкладки «Показатели».
    pub fn about(self) -> &'static str {
        match self {
            Cat::Net => "Интернет, DNS, прокси и VPN",
            Cat::Ai => "Связь с ИИ-помощниками и их сессии в терминале",
            Cat::Gpu => "Видеокарта, драйвер, кто рисует экран",
            Cat::Dev if cfg!(target_os = "macos") => "Звук, принтер",
            Cat::Dev if cfg!(windows) => "Звук, печать, Bluetooth, диск",
            Cat::Dev => "Звук, Bluetooth, принтер, клавиатура",
            Cat::Look if cfg!(target_os = "macos") => "Оформление, цвета, Dock",
            Cat::Look if cfg!(windows) => "Тема, цвет акцента, обои, панель задач",
            Cat::Look => "Темы, значки, расширения, док",
            Cat::Sys => "Время, диск, память, обновления, службы",
        }
    }
    /// Имя иконки (assets/xp/<имя>.png).
    pub fn icon(self) -> &'static str {
        match self {
            Cat::Net => "net",
            Cat::Ai => "ai",
            Cat::Gpu => "gpu",
            Cat::Dev => "dev",
            Cat::Look => "theme",
            Cat::Sys => "sys",
        }
    }
}

#[derive(Clone)]
pub struct Unit {
    /// Код для настроек (не менять: по нему запоминается «выключено»).
    pub id: &'static str,
    pub cat: Cat,
    pub title: &'static str,
    /// Что проверяет — одной фразой, для вкладки «Показатели».
    pub about: &'static str,
    pub run: fn() -> Vec<Check>,
    /// Включён ли, пока человек сам не решил иначе.
    pub default_on: bool,
}

#[cfg(target_os = "linux")]
fn base_units() -> &'static [Unit] {
    use crate::diag::*;
    static U: &[Unit] = &[
        Unit {
            id: "clock",
            cat: Cat::Sys,
            title: "Часы",
            about: "Не врут ли часы и включена ли автосверка времени",
            run: check_clock,
            default_on: true,
        },
        Unit {
            id: "network",
            cat: Cat::Net,
            title: "Подключение и DNS",
            about: "Есть ли сеть, интернет, отвечает ли DNS",
            run: check_network,
            default_on: true,
        },
        Unit {
            id: "proxy",
            cat: Cat::Net,
            title: "Системный прокси",
            about: "Не остался ли прокси от выключенного VPN",
            run: check_proxy,
            default_on: true,
        },
        Unit {
            id: "proxy_other",
            cat: Cat::Net,
            title: "Прокси в браузерах и программах",
            about: "Прокси-расширения браузеров (BlancVPN и т. п.), прокси в Firefox, http_proxy в терминале, git/npm/pip/apt",
            run: crate::netproxy::check_other_proxies,
            default_on: true,
        },
        Unit {
            id: "vpn", cat: Cat::Net, title: "VPN", about: "Каналы Happ, Outline, Proton и внешний IP", run: check_vpn, default_on: true
        },
        Unit {
            id: "latency",
            cat: Cat::Net,
            title: "Задержка сети",
            about: "Пинг и джиттер до сервера замера раз в 15 минут (без скачивания: трафика почти нет)",
            run: crate::diag::check_latency,
            default_on: true,
        },
        Unit {
            id: "captive",
            cat: Cat::Net,
            title: "Вход в сеть",
            about: "Не просит ли сеть войти на своей странице (кафе, гостиница, вокзал)",
            run: crate::netproxy::check_captive,
            default_on: true,
        },
        Unit {
            id: "battery",
            cat: Cat::Sys,
            title: "Батарея",
            about: "Заряд, заряжается ли от сети, износ (у ноутбуков)",
            run: crate::netproxy::check_battery,
            default_on: true,
        },
        Unit {
            id: "diskhealth",
            cat: Cat::Sys,
            title: "Здоровье дисков",
            about: "Не сообщают ли сами диски о скором отказе (S.M.A.R.T.)",
            run: crate::disks::check_health,
            default_on: true,
        },
        Unit {
            id: "footprint",
            cat: Cat::Sys,
            title: "Цифровой след",
            about: "Ключи API и доступы VPN открытым текстом: в истории команд, чатах ИИ, настройках; ключи SSH без пароля; шифрование диска",
            run: crate::footprint::check,
            default_on: true,
        },
        Unit {
            id: "wifi",
            cat: Cat::Net,
            title: "Сигнал Wi-Fi",
            about: "Не слабый ли сигнал Wi-Fi (частая причина «тормозит интернет»)",
            run: crate::netproxy::check_wifi_signal,
            default_on: true,
        },
        Unit {
            id: "claude",
            cat: Cat::Ai,
            title: "Claude",
            about: "Связь с Claude API, claude.ai и статус серверов Anthropic",
            run: check_claude_reach,
            default_on: true,
        },
        Unit {
            id: "codex",
            cat: Cat::Ai,
            title: "Codex",
            about: "OpenAI Codex CLI: связь, статус OpenAI, сессии",
            run: crate::agents::check_codex,
            default_on: true,
        },
        Unit {
            id: "ai_limits",
            cat: Cat::Ai,
            title: "Лимиты и расход ИИ",
            about: "Сколько занято лимитов подписки Claude Code и Codex, когда сброс, сколько токенов ушло",
            run: crate::limits::check,
            default_on: true,
        },
        Unit {
            id: "qwen",
            cat: Cat::Ai,
            title: "Qwen Code",
            about: "Qwen Code: связь с DashScope, сессии",
            run: crate::agents::check_qwen,
            default_on: true,
        },
        Unit {
            id: "kimi",
            cat: Cat::Ai,
            title: "Kimi",
            about: "Kimi Code: связь с Kimi и Moonshot, сессии",
            run: crate::agents::check_kimi,
            default_on: true,
        },
        Unit {
            id: "gemini",
            cat: Cat::Ai,
            title: "Gemini CLI",
            about: "Gemini CLI: связь с Gemini API, сессии",
            run: crate::agents::check_gemini,
            default_on: true,
        },
        Unit {
            id: "cursor",
            cat: Cat::Ai,
            title: "Cursor",
            about: "Cursor CLI (cursor-agent): связь, статус Cursor, сессии",
            run: crate::agents::check_cursor,
            default_on: true,
        },
        Unit {
            id: "opencode",
            cat: Cat::Ai,
            title: "OpenCode",
            about: "OpenCode: связь с opencode.ai, сессии",
            run: crate::agents::check_opencode,
            default_on: true,
        },
        Unit {
            id: "agy",
            cat: Cat::Ai,
            title: "Antigravity",
            about: "Google Antigravity: связь с Google, разговоры",
            run: crate::agents::check_agy,
            default_on: true,
        },
        Unit {
            id: "dock",
            cat: Cat::Look,
            title: "Док",
            about: "На месте ли док и не пропал ли он после полноэкранных окон",
            run: check_dock,
            default_on: true,
        },
        Unit {
            id: "repos", cat: Cat::Sys, title: "Репозитории", about: "Доступны ли источники обновлений apt", run: check_repos, default_on: true
        },
        Unit {
            id: "gpu",
            cat: Cat::Gpu,
            title: "Видеокарта",
            about: "Кто рисует экран, драйвер NVIDIA, температура, режим PRIME",
            run: check_gpu,
            default_on: true,
        },
        Unit {
            id: "themes",
            cat: Cat::Look,
            title: "Темы и расширения",
            about: "Не сбились ли темы и работают ли расширения GNOME",
            run: check_themes,
            default_on: true,
        },
        Unit {
            id: "sound",
            cat: Cat::Dev,
            title: "Звук",
            about: "Работает ли звук, на каком устройстве, не выключен ли, микрофон",
            run: crate::more::check_sound,
            default_on: true,
        },
        Unit {
            id: "radio",
            cat: Cat::Dev,
            title: "Bluetooth и Wi-Fi",
            about: "Не заблокированы ли, работает ли служба Bluetooth",
            run: crate::more::check_radio,
            default_on: true,
        },
        Unit {
            id: "printing",
            cat: Cat::Dev,
            title: "Печать",
            about: "Принтер не на паузе, очередь не застряла",
            run: crate::more::check_printing,
            default_on: true,
        },
        Unit {
            id: "keyboard",
            cat: Cat::Dev,
            title: "Раскладка",
            about: "Есть ли чем переключать раскладку",
            run: crate::more::check_keyboard,
            default_on: true,
        },
        Unit {
            id: "packages",
            cat: Cat::Sys,
            title: "Пакеты и /boot",
            about: "Прерванная установка, сломанные зависимости, нужна ли перезагрузка, место в /boot",
            run: crate::more::check_packages,
            default_on: true,
        },
        Unit {
            id: "cleanup",
            cat: Cat::Sys,
            title: "Что занимает место",
            about: "Корзина, кэш миниатюр, системный журнал",
            run: crate::more::check_cleanup,
            default_on: true,
        },
        Unit {
            id: "dupes",
            cat: Cat::Sys,
            title: "Копии файлов",
            about: "Одинаковые файлы в Загрузках, Документах, Изображениях и т. п. (ищет по кнопке)",
            run: crate::dupes::check_dupes,
            default_on: true,
        },
        Unit {
            id: "memory",
            cat: Cat::Sys,
            title: "Нехватка памяти",
            about: "Закрывала ли система программы из-за памяти, есть ли подкачка",
            run: crate::more::check_memory,
            default_on: true,
        },
        Unit {
            id: "hogs",
            cat: Cat::Sys,
            title: "Прожорливые программы",
            about: "Кто сейчас грузит процессор",
            run: crate::more::check_hogs,
            default_on: true,
        },
        Unit {
            id: "secureboot",
            cat: Cat::Gpu,
            title: "Secure Boot и NVIDIA",
            about: "Не мешает ли Secure Boot драйверу NVIDIA",
            run: crate::more::check_secureboot,
            default_on: true,
        },
        Unit {
            id: "system",
            cat: Cat::Sys,
            title: "Нагрузка и диск",
            about: "Процессор, память, диск, упавшие службы",
            run: check_system,
            default_on: true,
        },
    ];
    U
}

#[cfg(windows)]
fn base_units() -> &'static [Unit] {
    use crate::win::*;
    static U: &[Unit] = &[
        Unit { id: "win_os", cat: Cat::Sys, title: "Windows", about: "Версия Windows", run: check_os, default_on: true },
        Unit {
            id: "themes",
            cat: Cat::Look,
            title: "Темы",
            about: "Не сбились ли тема, цвет акцента, обои и панель задач",
            run: crate::looksnap::check,
            default_on: true,
        },
        Unit {
            id: "network", cat: Cat::Net, title: "Интернет и DNS", about: "Есть ли интернет, отвечает ли DNS", run: check_net, default_on: true
        },
        Unit {
            id: "proxy",
            cat: Cat::Net,
            title: "Системный прокси",
            about: "Не остался ли прокси от закрытого VPN (Happ, v2rayN, Hiddify…), WinHTTP",
            run: check_proxy,
            default_on: true,
        },
        Unit {
            id: "vpn",
            cat: Cat::Net,
            title: "VPN",
            about: "Happ, Outline, Proton, Amnezia, Hiddify, v2rayN, WireGuard: запущен ли, не остался ли туннель",
            run: check_vpn,
            default_on: true,
        },
        Unit {
            id: "latency",
            cat: Cat::Net,
            title: "Задержка сети",
            about: "Пинг и джиттер до сервера замера раз в 15 минут (без скачивания: трафика почти нет)",
            run: crate::diag::check_latency,
            default_on: true,
        },
        Unit {
            id: "wifi", cat: Cat::Net, title: "Wi-Fi", about: "Подключён ли Wi-Fi и какой сигнал", run: check_wifi, default_on: true
        },
        Unit {
            id: "proxy_other",
            cat: Cat::Net,
            title: "Прокси в браузерах и программах",
            about: "Прокси-расширения браузеров, переменные http_proxy (Windows и терминал), git/npm/pip",
            run: crate::netproxy::check_other_proxies,
            default_on: true,
        },
        Unit {
            id: "captive",
            cat: Cat::Net,
            title: "Вход в сеть",
            about: "Не просит ли сеть войти на своей странице (кафе, гостиница, вокзал)",
            run: crate::netproxy::check_captive,
            default_on: true,
        },
        Unit {
            id: "battery",
            cat: Cat::Sys,
            title: "Батарея",
            about: "Заряд, заряжается ли от сети, износ (у ноутбуков)",
            run: crate::netproxy::check_battery,
            default_on: true,
        },
        Unit {
            id: "diskhealth",
            cat: Cat::Sys,
            title: "Здоровье дисков",
            about: "Не сообщают ли сами диски о скором отказе (S.M.A.R.T.)",
            run: crate::disks::check_health,
            default_on: true,
        },
        Unit {
            id: "footprint",
            cat: Cat::Sys,
            title: "Цифровой след",
            about: "Ключи API и доступы VPN открытым текстом: в истории команд, чатах ИИ, настройках; ключи SSH без пароля; шифрование диска",
            run: crate::footprint::check,
            default_on: true,
        },
        Unit {
            id: "claude",
            cat: Cat::Ai,
            title: "Claude",
            about: "Связь с Claude API, claude.ai и статус серверов Anthropic",
            run: crate::diag::check_claude_reach,
            default_on: true,
        },
        Unit {
            id: "codex",
            cat: Cat::Ai,
            title: "Codex",
            about: "OpenAI Codex CLI: связь, статус OpenAI, сессии",
            run: crate::agents::check_codex,
            default_on: true,
        },
        Unit {
            id: "ai_limits",
            cat: Cat::Ai,
            title: "Лимиты и расход ИИ",
            about: "Сколько занято лимитов подписки Claude Code и Codex, когда сброс, сколько токенов ушло",
            run: crate::limits::check,
            default_on: true,
        },
        Unit {
            id: "qwen",
            cat: Cat::Ai,
            title: "Qwen Code",
            about: "Qwen Code: связь с DashScope, сессии",
            run: crate::agents::check_qwen,
            default_on: true,
        },
        Unit {
            id: "kimi",
            cat: Cat::Ai,
            title: "Kimi",
            about: "Kimi Code: связь с Kimi и Moonshot, сессии",
            run: crate::agents::check_kimi,
            default_on: true,
        },
        Unit {
            id: "gemini",
            cat: Cat::Ai,
            title: "Gemini CLI",
            about: "Gemini CLI: связь с Gemini API, сессии",
            run: crate::agents::check_gemini,
            default_on: true,
        },
        Unit {
            id: "cursor",
            cat: Cat::Ai,
            title: "Cursor",
            about: "Cursor CLI (cursor-agent): связь, статус Cursor, сессии",
            run: crate::agents::check_cursor,
            default_on: true,
        },
        Unit {
            id: "opencode",
            cat: Cat::Ai,
            title: "OpenCode",
            about: "OpenCode: связь с opencode.ai, сессии",
            run: crate::agents::check_opencode,
            default_on: true,
        },
        Unit {
            id: "agy",
            cat: Cat::Ai,
            title: "Antigravity",
            about: "Google Antigravity: связь с Google, разговоры",
            run: crate::agents::check_agy,
            default_on: true,
        },
        Unit {
            id: "updates",
            cat: Cat::Sys,
            title: "Обновления Windows",
            about: "Ошибки установки обновлений за 2 недели и что они значат",
            run: check_updates,
            default_on: true,
        },
        Unit {
            id: "stability",
            cat: Cat::Sys,
            title: "Сбои и зависания",
            about: "Внезапные выключения, падающие и зависшие программы, проводник",
            run: check_stability,
            default_on: true,
        },
        Unit {
            id: "system",
            cat: Cat::Sys,
            title: "Нагрузка и память",
            about: "Занят ли процессор, хватает ли памяти",
            run: check_system,
            default_on: true,
        },
        Unit {
            id: "hogs", cat: Cat::Sys, title: "Прожорливые программы", about: "Кто сейчас грузит процессор", run: check_hogs, default_on: true
        },
        Unit { id: "clock", cat: Cat::Sys, title: "Часы", about: "Не врут ли часы", run: check_time, default_on: true },
        Unit {
            id: "services",
            cat: Cat::Dev,
            title: "Звук, печать, Bluetooth, диск",
            about: "Службы звука, печати и Bluetooth; место на диске C:",
            run: check_services,
            default_on: true,
        },
        Unit {
            id: "dupes",
            cat: Cat::Sys,
            title: "Копии файлов",
            about: "Одинаковые файлы в Загрузках, Документах, Изображениях и т. п. (ищет по кнопке)",
            run: crate::dupes::check_dupes,
            default_on: true,
        },
    ];
    U
}

#[cfg(target_os = "macos")]
fn base_units() -> &'static [Unit] {
    use crate::mac::*;
    static U: &[Unit] = &[
        Unit { id: "mac_os", cat: Cat::Sys, title: "macOS", about: "Версия macOS", run: check_os, default_on: true },
        Unit {
            id: "themes",
            cat: Cat::Look,
            title: "Темы",
            about: "Не сбились ли оформление, цвета и Dock",
            run: crate::looksnap::check,
            default_on: true,
        },
        Unit {
            id: "network", cat: Cat::Net, title: "Интернет и DNS", about: "Есть ли интернет, отвечает ли DNS", run: check_net, default_on: true
        },
        Unit {
            id: "proxy",
            cat: Cat::Net,
            title: "Системный прокси",
            about: "Не остался ли прокси от закрытого VPN",
            run: check_proxy,
            default_on: true,
        },
        Unit {
            id: "vpn",
            cat: Cat::Net,
            title: "VPN",
            about: "VPN-службы macOS (Happ, Outline, ProtonVPN, BlancVPN…): подключены ли",
            run: check_vpn,
            default_on: true,
        },
        Unit {
            id: "latency",
            cat: Cat::Net,
            title: "Задержка сети",
            about: "Пинг и джиттер до сервера замера раз в 15 минут (без скачивания: трафика почти нет)",
            run: crate::diag::check_latency,
            default_on: true,
        },
        Unit {
            id: "wifi", cat: Cat::Net, title: "Wi-Fi", about: "Подключён ли Wi-Fi и какой сигнал", run: check_wifi, default_on: true
        },
        Unit {
            id: "captive",
            cat: Cat::Net,
            title: "Вход в сеть",
            about: "Не просит ли сеть войти на своей странице (кафе, гостиница, вокзал)",
            run: crate::netproxy::check_captive,
            default_on: true,
        },
        Unit {
            id: "battery",
            cat: Cat::Sys,
            title: "Батарея",
            about: "Заряд, заряжается ли от сети, износ (у ноутбуков)",
            run: crate::netproxy::check_battery,
            default_on: true,
        },
        Unit {
            id: "diskhealth",
            cat: Cat::Sys,
            title: "Здоровье дисков",
            about: "Не сообщают ли сами диски о скором отказе (S.M.A.R.T.)",
            run: crate::disks::check_health,
            default_on: true,
        },
        Unit {
            id: "footprint",
            cat: Cat::Sys,
            title: "Цифровой след",
            about: "Ключи API и доступы VPN открытым текстом: в истории команд, чатах ИИ, настройках; ключи SSH без пароля; шифрование диска",
            run: crate::footprint::check,
            default_on: true,
        },
        Unit {
            id: "proxy_other",
            cat: Cat::Net,
            title: "Прокси в браузерах и программах",
            about: "Прокси-расширения браузеров, http_proxy в терминале, git/npm/pip",
            run: crate::netproxy::check_other_proxies,
            default_on: true,
        },
        Unit {
            id: "claude",
            cat: Cat::Ai,
            title: "Claude",
            about: "Связь с Claude API, claude.ai и статус серверов Anthropic",
            run: crate::diag::check_claude_reach,
            default_on: true,
        },
        Unit {
            id: "codex",
            cat: Cat::Ai,
            title: "Codex",
            about: "OpenAI Codex CLI: связь, статус OpenAI, сессии",
            run: crate::agents::check_codex,
            default_on: true,
        },
        Unit {
            id: "ai_limits",
            cat: Cat::Ai,
            title: "Лимиты и расход ИИ",
            about: "Сколько занято лимитов подписки Claude Code и Codex, когда сброс, сколько токенов ушло",
            run: crate::limits::check,
            default_on: true,
        },
        Unit {
            id: "qwen",
            cat: Cat::Ai,
            title: "Qwen Code",
            about: "Qwen Code: связь с DashScope, сессии",
            run: crate::agents::check_qwen,
            default_on: true,
        },
        Unit {
            id: "kimi",
            cat: Cat::Ai,
            title: "Kimi",
            about: "Kimi Code: связь с Kimi и Moonshot, сессии",
            run: crate::agents::check_kimi,
            default_on: true,
        },
        Unit {
            id: "gemini",
            cat: Cat::Ai,
            title: "Gemini CLI",
            about: "Gemini CLI: связь с Gemini API, сессии",
            run: crate::agents::check_gemini,
            default_on: true,
        },
        Unit {
            id: "cursor",
            cat: Cat::Ai,
            title: "Cursor",
            about: "Cursor CLI (cursor-agent): связь, статус Cursor, сессии",
            run: crate::agents::check_cursor,
            default_on: true,
        },
        Unit {
            id: "opencode",
            cat: Cat::Ai,
            title: "OpenCode",
            about: "OpenCode: связь с opencode.ai, сессии",
            run: crate::agents::check_opencode,
            default_on: true,
        },
        Unit {
            id: "agy",
            cat: Cat::Ai,
            title: "Antigravity",
            about: "Google Antigravity: связь с Google, разговоры",
            run: crate::agents::check_agy,
            default_on: true,
        },
        Unit {
            id: "audio", cat: Cat::Dev, title: "Звук", about: "Работает ли звуковая служба coreaudiod", run: check_audio, default_on: true
        },
        Unit {
            id: "printing",
            cat: Cat::Dev,
            title: "Печать",
            about: "Принтер не на паузе, очередь не застряла",
            run: crate::more::check_printing_mac,
            default_on: true,
        },
        Unit {
            id: "system",
            cat: Cat::Sys,
            title: "Нагрузка и память",
            about: "Занят ли процессор, хватает ли памяти, много ли подкачки",
            run: check_system,
            default_on: true,
        },
        Unit {
            id: "disk",
            cat: Cat::Sys,
            title: "Диск и снимки",
            about: "Свободное место, локальные снимки Time Machine",
            run: check_disk,
            default_on: true,
        },
        Unit {
            id: "hogs",
            cat: Cat::Sys,
            title: "Прожорливые программы",
            about: "Кто грузит процессор; индексация Spotlight",
            run: check_hogs,
            default_on: true,
        },
        Unit {
            id: "apps",
            cat: Cat::Sys,
            title: "Открытые и зависшие программы",
            about: "Закрыть зависшую программу, которая не закрывается; зависания и падения за неделю",
            run: check_apps,
            default_on: true,
        },
        Unit { id: "clock", cat: Cat::Sys, title: "Часы", about: "Не врут ли часы", run: check_time, default_on: true },
        Unit {
            id: "dupes",
            cat: Cat::Sys,
            title: "Копии файлов",
            about: "Одинаковые файлы в Загрузках, Документах, Изображениях и т. п. (ищет по кнопке)",
            run: crate::dupes::check_dupes,
            default_on: true,
        },
    ];
    U
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn base_units() -> &'static [Unit] {
    &[]
}

/// Все показатели этой системы. Своя сборка владелицы (папка custom/ рядом с engine/, её нет в git и в
/// публичном репозитории — build.rs включает cfg aptechka_custom) добавляет свои в конец.
pub fn units() -> &'static [Unit] {
    #[cfg(aptechka_custom)]
    {
        static ALL: std::sync::OnceLock<Vec<Unit>> = std::sync::OnceLock::new();
        return ALL.get_or_init(|| base_units().iter().cloned().chain(crate::custom::units()).collect());
    }
    #[allow(unreachable_code)]
    base_units()
}

pub fn find(id: &str) -> Option<&'static Unit> {
    units().iter().find(|u| u.id == id)
}
