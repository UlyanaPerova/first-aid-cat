//! Лицензии и уведомления, вшитые в программу (build.rs → OUT_DIR/notices): «Справка → Лицензии» кладёт их
//! в папку сеанса и открывает её системным файловым менеджером — на любой системе и в любой сборке, даже если
//! рядом с программой этих файлов нет (переносной архив, собранная из исходников).

pub const FILES: [(&str, &str); 4] = [
    ("LICENSE", include_str!(concat!(env!("OUT_DIR"), "/notices/LICENSE"))),
    ("NOTICE", include_str!(concat!(env!("OUT_DIR"), "/notices/NOTICE"))),
    ("LICENSE-ART.md", include_str!(concat!(env!("OUT_DIR"), "/notices/LICENSE-ART.md"))),
    ("THIRD_PARTY_NOTICES.md", include_str!(concat!(env!("OUT_DIR"), "/notices/THIRD_PARTY_NOTICES.md"))),
];

/// Записать файлы в папку сеанса (licenses/) и открыть её. Ok — папка (открылась или нет — второе в паре).
pub fn open() -> Result<(std::path::PathBuf, bool), String> {
    let dir = crate::store::runtime_dir().join("licenses");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, text) in FILES {
        crate::store::write_private(&dir.join(name), text.as_bytes()).map_err(|e| e.to_string())?;
    }
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut c = std::process::Command::new(opener);
    c.arg(&dir);
    crate::cmd::scrub_env(&mut c);
    let shown = crate::procs::spawn_detached(&mut c);
    Ok((dir, shown))
}

/// Инструкция «Свои проверки и исправления» (CUSTOM.md / CUSTOM.ru.md): на языке интерфейса — в папку сеанса и
/// открыть программой для текстов. Ok — путь (открылась или нет — второе в паре).
pub fn open_guide() -> Result<(std::path::PathBuf, bool), String> {
    let (name, text) = if crate::i18n::lang() == crate::i18n::Lang::Ru {
        ("CUSTOM.ru.md", include_str!(concat!(env!("OUT_DIR"), "/notices/CUSTOM.ru.md")))
    } else {
        ("CUSTOM.md", include_str!(concat!(env!("OUT_DIR"), "/notices/CUSTOM.md")))
    };
    let p = crate::store::runtime_dir().join(name);
    crate::store::write_private(&p, text.as_bytes()).map_err(|e| e.to_string())?;
    let opener = if cfg!(windows) {
        "notepad"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let mut c = std::process::Command::new(opener);
    c.arg(&p);
    crate::cmd::scrub_env(&mut c);
    let shown = crate::procs::spawn_detached(&mut c);
    Ok((p, shown))
}

#[cfg(test)]
mod tests {
    /// Все четыре файла вшиты и не пустые; лицензия — Apache 2.0.
    #[test]
    fn notices_are_embedded() {
        assert!(super::FILES.iter().all(|(_, t)| t.len() > 100));
        assert!(super::FILES[0].1.contains("Apache License"));
    }
}
