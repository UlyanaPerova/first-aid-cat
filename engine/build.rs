//! Вшивает в программу картинки и данные слоёв из art/layers: только то, что читает движок
//! (layers.json, слои, позы из позы.json, дыхание.json, экраны.*) — ≈4 МБ, а не вся папка с черновиками.
use std::path::Path;

fn main() {
    i18n_tables();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../art/layers");
    let dir = dir.canonicalize().expect("нет art/layers");
    let read_json = |rel: &str| -> serde_json::Value {
        let p = dir.join(rel);
        println!("cargo:rerun-if-changed={}", p.display());
        serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).expect("json")
    };
    let mut files: Vec<String> = vec![
        "layers.json".into(),
        "дыхание.json".into(),
        "экраны.json".into(),
        "экраны.png".into(),
        "позы/позы.json".into(),
        "потолок.png".into(),
        "виджет/кружка_ok.png".into(),
        "виджет/кружка_warn.png".into(),
        "виджет/кружка_bad.png".into(),
    ];
    for e in read_json("layers.json")["order"].as_array().expect("order") {
        files.push(format!("{}.png", e["name"].as_str().expect("name")));
    }
    if let Some(p) = read_json("позы/позы.json").as_object() {
        for (frame, ls) in p {
            for l in ls.as_array().into_iter().flatten() {
                files.push(format!("позы/{}@{frame}.png", l.as_str().expect("слой")));
            }
        }
    }
    // всё для виджета: кот с кружкой, шаблоны видов (картинка + разметка полей)
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                // старые шаблоны-вырезки из концептов не вшиваем: виджеты рисуются по пикселям (widget_px.rs)
                if p.file_name().is_some_and(|n| n == "шаблоны") {
                    continue;
                }
                walk(&p, base, out);
            } else if matches!(p.extension().and_then(|x| x.to_str()), Some("png" | "json")) {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    println!("cargo:rerun-if-changed={}", dir.join("виджет").display());
    println!("cargo:rerun-if-changed={}", dir.join("виджет/шаблоны").display());
    walk(&dir.join("виджет"), &dir, &mut files);
    files.sort();
    files.dedup();
    let mut out =
        String::from("/// Вшитые файлы art/layers: (путь внутри папки, содержимое). Создаётся build.rs.\npub static FILES: &[(&str, &[u8])] = &[\n");
    for f in &files {
        let p = dir.join(f);
        assert!(p.is_file(), "нет файла {}", p.display());
        println!("cargo:rerun-if-changed={}", p.display());
        out.push_str(&format!("    ({f:?}, include_bytes!({:?})),\n", p.to_string_lossy()));
    }
    out.push_str("];\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("art_files.rs"), out).unwrap();
    windows_icon();
}

/// Windows: значок программы (assets/aptechka.ico) — ресурсом внутри aptechka.exe (виден в проводнике и на панели).
fn windows_icon() {
    if !std::env::var("TARGET").unwrap_or_default().contains("windows") {
        return;
    }
    let ico = Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/aptechka.ico").canonicalize().expect("нет assets/aptechka.ico");
    println!("cargo:rerun-if-changed={}", ico.display());
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let rc = out.join("icon.rc");
    std::fs::write(&rc, format!("1 ICON \"{}\"\n", ico.to_string_lossy().replace('\\', "/"))).unwrap();
    let obj = out.join("icon.o");
    // GNU-сборка (MinGW): windres → объектный файл, который компоновщик добавит в exe
    let windres = std::env::var("WINDRES").unwrap_or_else(|_| "x86_64-w64-mingw32-windres".into());
    let ok = std::process::Command::new(&windres).arg(&rc).args(["-O", "coff", "-o"]).arg(&obj).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg-bins={}", obj.display());
    } else {
        println!("cargo:warning=значок для Windows не вшит: нет {windres}");
    }
}

/// Таблицы перевода engine/i18n/en/*.txt — одним файлом для i18n.rs (порядок по имени, чтобы сборка была повторяемой).
fn i18n_tables() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n/en");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<_> = std::fs::read_dir(&dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    names.retain(|p: &std::path::PathBuf| p.extension().is_some_and(|x| x == "txt"));
    names.sort();
    let mut all = String::new();
    for p in &names {
        println!("cargo:rerun-if-changed={}", p.display());
        all.push_str(&std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display())));
        all.push('\n');
    }
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("i18n_en.txt");
    std::fs::write(out, all).expect("i18n_en.txt");
}
