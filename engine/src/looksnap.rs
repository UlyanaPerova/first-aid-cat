//! «Темы» на Windows и macOS: запомнить, как выглядит система («Запомнить текущее как норму»), и заметить,
//! что сбилось (обновление, чужая программа, случайный щелчок) — с кнопкой «Вернуть как было».
//! Эталон — тот же файл, что на Linux (snapshot.json в папке настроек), ключи вида «reg|раздел|имя»
//! (Windows, реестр текущего пользователя) и «defaults|домен|ключ» (macOS).
//! Только чтение при проверке; запись — только по кнопке и только наших ключей (validate в fixes.rs).
#![cfg(any(windows, target_os = "macos"))]

use crate::diag::{fx, Check, BAD, INFO, OK, WARN};

/// Ключ эталона: (id, название строки, вид значения).
/// Вид: dword (число реестра), text (строка), bool/int/float/string — тип `defaults write`.
pub struct Key {
    pub id: &'static str,
    pub title: &'static str,
    pub kind: &'static str,
}

#[cfg(windows)]
pub const KEYS: [Key; 7] = [
    Key {
        id: r"reg|HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize|SystemUsesLightTheme", title: "Тема Windows", kind: "dword"
    },
    Key {
        id: r"reg|HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize|AppsUseLightTheme", title: "Тема приложений", kind: "dword"
    },
    Key {
        id: r"reg|HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize|EnableTransparency", title: "Прозрачность", kind: "dword"
    },
    Key { id: r"reg|HKCU:\Software\Microsoft\Windows\DWM|AccentColor", title: "Цвет акцента", kind: "dword" },
    Key { id: r"reg|HKCU:\Software\Microsoft\Windows\DWM|ColorPrevalence", title: "Акцент на заголовках окон", kind: "dword" },
    Key { id: r"reg|HKCU:\Control Panel\Desktop|WallPaper", title: "Обои", kind: "text" },
    Key {
        id: r"reg|HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced|TaskbarAl", title: "Панель задач: значки", kind: "dword"
    },
];

#[cfg(target_os = "macos")]
pub const KEYS: [Key; 8] = [
    Key { id: "defaults|-g|AppleInterfaceStyle", title: "Оформление", kind: "string" },
    Key { id: "defaults|-g|AppleAccentColor", title: "Цвет акцента", kind: "int" },
    Key { id: "defaults|-g|AppleHighlightColor", title: "Цвет выделения", kind: "string" },
    Key { id: "defaults|-g|AppleShowScrollBars", title: "Полосы прокрутки", kind: "string" },
    Key { id: "defaults|com.apple.dock|autohide", title: "Dock: автоскрытие", kind: "bool" },
    Key { id: "defaults|com.apple.dock|orientation", title: "Dock: положение", kind: "string" },
    Key { id: "defaults|com.apple.dock|tilesize", title: "Dock: размер значков", kind: "float" },
    Key { id: "defaults|com.apple.dock|magnification", title: "Dock: увеличение", kind: "bool" },
];

/// Ключ из нашего списка (для «Вернуть как было»: чужой ключ не пишем).
pub fn known(id: &str) -> bool {
    KEYS.iter().any(|k| k.id == id)
}

fn key(id: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.id == id)
}

fn split(id: &str) -> Option<(&str, &str)> {
    let mut p = id.splitn(3, '|');
    p.next()?;
    Some((p.next()?, p.next()?))
}

/// Текущие значения всех ключей и время. Значение "" — настройки нет (у системы — как по умолчанию);
/// None у ключа — прочитать не удалось; None целиком — система не ответила.
#[cfg(windows)]
pub fn read_all() -> Option<(Vec<(String, Option<String>)>, String)> {
    let mut items = Vec::new();
    for k in &KEYS {
        let (p, n) = split(k.id)?;
        items.push(format!("@({},{},{})", crate::agents::ps_quote(k.id), crate::agents::ps_quote(p), crate::agents::ps_quote(n)));
    }
    let script = format!(
        "$o=[ordered]@{{}}; foreach($k in @({})) {{ try {{ $v=(Get-ItemProperty -LiteralPath $k[1] -Name $k[2] -ErrorAction Stop).($k[2]); $o[$k[0]]=[string]$v }} \
         catch [System.Management.Automation.PSArgumentException] {{ $o[$k[0]]='' }} catch [System.Management.Automation.ItemNotFoundException] {{ $o[$k[0]]='' }} catch {{ $o[$k[0]]=$null }} }}; \
         $o['time']=(Get-Date).ToString('ddd MMM d HH:mm:ss yyyy',[Globalization.CultureInfo]::InvariantCulture); [pscustomobject]$o",
        items.join(",")
    );
    let v = crate::win::ps_try(&script, 20)?.into_iter().next()?;
    let vals = KEYS.iter().map(|k| (k.id.to_string(), v.get(k.id).and_then(|x| x.as_str()).map(String::from))).collect();
    Some((vals, v.get("time").and_then(|t| t.as_str()).unwrap_or("?").to_string()))
}

#[cfg(target_os = "macos")]
pub fn read_all() -> Option<(Vec<(String, Option<String>)>, String)> {
    let t = std::time::Duration::from_secs(6);
    let mut vals = Vec::new();
    let mut any = false;
    for k in &KEYS {
        let (dom, name) = split(k.id)?;
        let r = crate::cmd::exec(&["defaults", "read", dom, name], t, crate::cmd::FIX);
        let v = match r.code() {
            0 => Some(r.text().trim().to_string()),
            // ключа нет — у системы значение по умолчанию (светлое оформление, синий акцент…)
            _ if r.text().contains("does not exist") => Some(String::new()),
            _ => None,
        };
        any |= v.is_some();
        vals.push((k.id.to_string(), v));
    }
    if !any {
        return None;
    }
    let time = crate::cmd::exec(&["date", "+%a %b %e %H:%M:%S %Y"], t, crate::cmd::CHECK).text().trim().to_string();
    Some((vals, time))
}

/// Эталон: значения, которые удалось прочитать, и время.
pub fn capture() -> (Vec<(String, String)>, String) {
    match read_all() {
        Some((vals, time)) => (vals.into_iter().filter_map(|(k, v)| Some((k, v?))).collect(), time),
        None => (Vec::new(), "?".into()),
    }
}

/// Значение — так, как его поймёт человек.
pub fn shown(id: &str, v: &str) -> String {
    let name = split(id).map_or("", |(_, n)| n);
    let onoff = |v: &str| if v == "1" || v.eq_ignore_ascii_case("true") { "вкл" } else { "выкл" };
    let s = match name {
        "SystemUsesLightTheme" | "AppsUseLightTheme" => if v == "0" { "тёмная" } else { "светлая" }.to_string(),
        "EnableTransparency" | "ColorPrevalence" | "autohide" | "magnification" => {
            if v.is_empty() {
                "выкл".into()
            } else {
                onoff(v).into()
            }
        }
        "AccentColor" => match v.parse::<i64>() {
            // ABGR в реестре → #RRGGBB
            Ok(n) => {
                let n = n as u32;
                format!("#{:02X}{:02X}{:02X}", n & 0xFF, (n >> 8) & 0xFF, (n >> 16) & 0xFF)
            }
            Err(_) => "как в системе".into(),
        },
        "WallPaper" => {
            if v.is_empty() {
                "нет (сплошной цвет)".into()
            } else {
                v.rsplit(['\\', '/']).next().unwrap_or(v).to_string()
            }
        }
        "TaskbarAl" => if v == "0" { "слева" } else { "по центру" }.to_string(),
        "AppleInterfaceStyle" => if v == "Dark" { "тёмное" } else { "светлое" }.to_string(),
        "AppleAccentColor" => match v {
            "-1" => "графит",
            "0" => "красный",
            "1" => "оранжевый",
            "2" => "жёлтый",
            "3" => "зелёный",
            "5" => "фиолетовый",
            "6" => "розовый",
            _ => "синий",
        }
        .to_string(),
        "AppleHighlightColor" => {
            if v.is_empty() {
                "как акцент".into()
            } else {
                v.split_whitespace().last().unwrap_or(v).to_string()
            }
        }
        "AppleShowScrollBars" => match v {
            "Always" => "всегда",
            "WhenScrolling" => "при прокрутке",
            _ => "автоматически",
        }
        .to_string(),
        "orientation" => match v {
            "left" => "слева",
            "right" => "справа",
            _ => "снизу",
        }
        .to_string(),
        "tilesize" => {
            if v.is_empty() {
                "как в системе".into()
            } else {
                v.split('.').next().unwrap_or(v).to_string()
            }
        }
        _ => v.to_string(),
    };
    s
}

/// Проверка «Темы»: что сбилось по сравнению с эталоном.
pub fn check() -> Vec<Check> {
    let snap = crate::diag::load_snapshot();
    let mut out = Vec::new();
    let Some((vals, _)) = read_all() else {
        out.push(Check::new("theme", "Темы", INFO, "не проверить").unknown().hint("Система не ответила — не знаю, как сейчас выглядят темы."));
        return out;
    };
    for (id, now) in vals {
        let Some(k) = key(&id) else { continue };
        let Some(now) = now else {
            out.push(Check::new("theme", k.title, INFO, "не проверить").unknown().hint("Настройка не прочиталась."));
            continue;
        };
        let want = snap.key(&id).unwrap_or(&now).to_string();
        let shown_now = shown(&id, &now);
        if now != want {
            out.push(Check::new("theme", k.title, WARN, &shown_now).hint(&format!("Сбилось! Было: {}", shown(&id, &want))).fixes(vec![fx(
                RESTORE,
                "Вернуть как было",
                Some(&id),
            )]));
            continue;
        }
        // обои выбраны, а файла нет (удалили, отключили диск) — на экране сплошной цвет
        #[cfg(windows)]
        if k.id.ends_with("|WallPaper") && !now.is_empty() && !std::path::Path::new(&now).exists() {
            out.push(
                Check::new("theme", k.title, BAD, &shown_now).hint("Файла обоев больше нет — Windows показывает сплошной цвет. Выбери обои заново."),
            );
            continue;
        }
        out.push(Check::new("theme", k.title, OK, &shown_now));
    }
    out.push(Check::new("theme", "Эталон", INFO, &snap.time).hint("С этим состоянием сравниваются темы.").fixes(vec![fx(
        "snapshot",
        "Запомнить текущее как норму",
        None,
    )]));
    out
}

#[cfg(windows)]
pub const RESTORE: &str = "win_theme_restore";
#[cfg(target_os = "macos")]
pub const RESTORE: &str = "mac_theme_restore";

/// Команда, которая вернёт ключ к значению `val` (её же показываем как ручной способ).
#[cfg(windows)]
pub fn restore_cmd(id: &str, val: &str) -> Option<String> {
    let k = key(id)?;
    let (path, name) = split(id)?;
    let (p, n) = (crate::agents::ps_quote(path), crate::agents::ps_quote(name));
    let set = if val.is_empty() {
        format!("Remove-ItemProperty -LiteralPath {p} -Name {n} -ErrorAction SilentlyContinue")
    } else if k.kind == "dword" {
        let num: i64 = val.parse().ok()?;
        format!("Set-ItemProperty -LiteralPath {p} -Name {n} -Type DWord -Value ([int32]{})", num as i32)
    } else {
        format!("Set-ItemProperty -LiteralPath {p} -Name {n} -Type String -Value {}", crate::agents::ps_quote(val))
    };
    // обои применяются вызовом SystemParametersInfo, тема и цвета — рассылкой «настройки изменились»
    let apply = if name == "WallPaper" {
        format!(
            "Add-Type -Namespace A -Name W -MemberDefinition '[DllImport(\"user32.dll\",CharSet=CharSet.Unicode)] public static extern bool SystemParametersInfo(int a,int b,string c,int d);'; [void][A.W]::SystemParametersInfo(20,0,{},3)",
            crate::agents::ps_quote(val)
        )
    } else {
        "Add-Type -Namespace A -Name M -MemberDefinition '[DllImport(\"user32.dll\",CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr h,int m,IntPtr w,string l,int f,int t,out IntPtr r);'; $r=[IntPtr]::Zero; [void][A.M]::SendMessageTimeout([IntPtr]0xffff,0x1A,[IntPtr]::Zero,'ImmersiveColorSet',2,3000,[ref]$r)".to_string()
    };
    Some(format!("{set}; {apply}"))
}

#[cfg(target_os = "macos")]
pub fn restore_cmd(id: &str, val: &str) -> Option<String> {
    let k = key(id)?;
    let (dom, name) = split(id)?;
    let q = crate::diag::shell_quote;
    // тёмное/светлое оформление — через System Events (сразу на экране; defaults write подействовал бы только после выхода)
    if name == "AppleInterfaceStyle" {
        return Some(format!(
            "osascript -e 'tell application \"System Events\" to tell appearance preferences to set dark mode to {}'",
            val == "Dark"
        ));
    }
    let set = if val.is_empty() {
        format!("defaults delete {dom} {name}")
    } else {
        let v = match k.kind {
            "bool" => (if val == "1" || val.eq_ignore_ascii_case("true") { "true" } else { "false" }).to_string(),
            "int" => val.parse::<i64>().ok()?.to_string(),
            "float" => val.parse::<f64>().ok()?.to_string(),
            _ => q(val),
        };
        format!("defaults write {dom} {name} -{} {v}", k.kind)
    };
    Some(if dom == "com.apple.dock" { format!("{set}; killall Dock") } else { set })
}

/// Вернуть ключ к значению из эталона.
pub fn restore(id: &str) -> (bool, String) {
    let snap = crate::diag::load_snapshot();
    let Some(val) = snap.key(id).map(String::from) else { return (false, "В эталоне нет этого значения.".into()) };
    let Some(cmd) = restore_cmd(id, &val) else {
        return (false, "Странное значение в эталоне — не трогаю.".into());
    };
    let title = key(id).map_or("", |k| k.title);
    if crate::fixes::dry_run() {
        crate::fixes::log_dry(&cmd);
        return (true, format!("Вернула как в эталоне: {title} — {}.", shown(id, &val)));
    }
    #[cfg(windows)]
    let r = crate::cmd::exec(
        &["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &format!("$ErrorActionPreference='Stop'; {cmd}")],
        std::time::Duration::from_secs(30),
        crate::cmd::FIX,
    );
    #[cfg(target_os = "macos")]
    let r = crate::cmd::exec(&["sh", "-c", &cmd], std::time::Duration::from_secs(30), crate::cmd::FIX);
    if r.code() == 0 {
        let later = cfg!(target_os = "macos") && !id.contains("com.apple.dock") && !id.ends_with("AppleInterfaceStyle");
        let v = shown(id, &val);
        if later {
            (true, format!("Вернула как в эталоне: {title} — {v}. Открытые программы подхватят это после перезапуска."))
        } else {
            (true, format!("Вернула как в эталоне: {title} — {v}."))
        }
    } else {
        (false, format!("Не вышло: {}", r.text().trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_known_and_shown() {
        assert!(KEYS.iter().all(|k| known(k.id) && split(k.id).is_some()));
        assert!(!known("reg|HKLM:\\SOFTWARE|Evil"));
        assert!(!known("defaults|-g|NSEvil"));
    }

    #[cfg(windows)]
    #[test]
    fn windows_values() {
        let accent = KEYS[3].id;
        // ABGR 0xFFD77800 → #0078D7 (синий Windows)
        assert_eq!(shown(accent, &(0xFFD77800u32 as i32).to_string()), "#0078D7");
        assert_eq!(shown(KEYS[0].id, "0"), "тёмная");
        // число из эталона — только число; строка — в кавычках PowerShell
        assert!(restore_cmd(accent, "12; rm x").is_none());
        assert!(restore_cmd(KEYS[5].id, "C:\\a'b.jpg").unwrap().contains("'C:\\a''b.jpg'"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_values() {
        assert_eq!(shown(KEYS[0].id, ""), "светлое");
        assert_eq!(restore_cmd(KEYS[4].id, "1").unwrap(), "defaults write com.apple.dock autohide -bool true; killall Dock");
        assert!(restore_cmd(KEYS[6].id, "48; rm -rf ~").is_none());
        assert_eq!(restore_cmd(KEYS[1].id, "").unwrap(), "defaults delete -g AppleAccentColor");
    }
}
