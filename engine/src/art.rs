//! Картинки и данные слоёв: вшиты в программу (build.rs). С переменной ART=папка — читаются с диска
//! (удобно, когда правишь позы: пересобирать не нужно).

include!(concat!(env!("OUT_DIR"), "/art_files.rs"));

/// Файл `rel` (путь внутри art/layers): с диска, если `dir` задан, иначе вшитый.
pub fn read(dir: &str, rel: &str) -> Option<Vec<u8>> {
    if !dir.is_empty() {
        return std::fs::read(format!("{dir}/{rel}")).ok();
    }
    FILES.iter().find(|(p, _)| *p == rel).map(|(_, b)| b.to_vec())
}

pub fn read_string(dir: &str, rel: &str) -> Option<String> {
    read(dir, rel).and_then(|b| String::from_utf8(b).ok())
}
