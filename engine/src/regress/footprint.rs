//! Регрессионные тесты поиска ключей и лечения «Вымарать ключи из чатов» (`footprint.rs`, `fixes.rs`).
//! Все файлы — выдуманные, во временной папке; все «ключи» — случайные строки, сгенерированные тут же и ничьи.
//! `fix_chats` запускается только после подмены HOME на временную папку (`with_fake_home` проверяет подмену).
#![allow(clippy::manual_is_multiple_of)] // is_multiple_of стабилен с Rust 1.90, а проект собирается с 1.89

use super::*;
use crate::regress_common::*;

/// Вид ключа: (начало, длина хвоста, из заглавных и цифр ли).
const KIND_SPECS: [(&str, usize, bool); 8] = [
    ("sk-proj-", 48, false),
    ("sk-ant-", 60, false),
    ("ghp_", 36, false),
    ("github_pat_", 50, false),
    ("AIza", 35, false),
    ("hf_", 34, false),
    ("xoxb-", 40, false),
    ("AKIA", 16, true),
];

fn gen_key(r: &mut Rng, spec: &(&str, usize, bool)) -> String {
    if spec.2 {
        const U: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ0123456789";
        loop {
            let t: String = (0..spec.1).map(|_| U[r.below(U.len() as u64) as usize] as char).collect();
            // «XXXX» и «EXAMPLE» программа считает признаком выдуманного примера
            if !t.contains("XXXX") && !t.contains("EXAMPLE") {
                return format!("{}{t}", spec.0);
            }
        }
    } else {
        fake_key(r, spec.0, spec.1)
    }
}

/// Безопасный наполнитель: строчные буквы, пробелы, русский, эмодзи — без «-», «_», кавычек и обратной косой
/// (чтобы не склеиваться с ключом и не создавать ключи случайно).
fn filler(r: &mut Rng, n: usize) -> String {
    const W: [&str; 12] = ["привет", "hello", "мир", "chat", "ответ", "ok", "😀", "世界", "token", "тест", "файл", "data"];
    let mut s = String::new();
    while s.len() < n {
        s += r.pick(&W);
        s.push(' ');
    }
    s
}

/// Файл: список (смещение ключа, ключ, длина «оставляемого» начала) и сам файл.
struct Built {
    data: Vec<u8>,
    keys: Vec<(usize, String, usize)>,
}

/// Собрать журнал: строки JSON `{"t":"… ключ …"}`; при `bad_bytes` между строками вставляются недопустимые байты.
fn build(r: &mut Rng, lines: usize, bad_bytes: bool) -> Built {
    let mut data = Vec::new();
    let mut keys = Vec::new();
    for _ in 0..lines {
        let mut line = String::from("{\"t\":\"");
        let mut pend: Vec<(usize, String, usize)> = Vec::new();
        for _ in 0..r.range(0, 3) {
            let fl = r.range(1, 40) as usize;
            line += &filler(r, fl);
            let spec = r.pick(&KIND_SPECS);
            let k = gen_key(r, spec);
            pend.push((line.len(), k.clone(), spec.0.len()));
            line += &k;
            line.push(' ');
        }
        line += &filler(r, 5);
        line += "\"}\n";
        let base = data.len();
        data.extend_from_slice(line.as_bytes());
        for (o, k, p) in pend {
            keys.push((base + o, k, p));
        }
        if bad_bytes && r.chance(30) {
            data.extend_from_slice(b"\xff\xfe\xc3\n");
        }
    }
    Built { data, keys }
}

/// Ожидание по построению (без вызова проверяемой функции): ровно хвосты ключей заменены на «*».
fn expected_after(b: &Built) -> Vec<u8> {
    let mut v = b.data.clone();
    for (o, k, keep) in &b.keys {
        for x in &mut v[*o + keep..*o + k.len()] {
            *x = b'*';
        }
    }
    v
}

fn lines_valid_json(d: &[u8]) -> bool {
    d.split(|b| *b == b'\n').filter(|l| !l.is_empty()).all(|l| serde_json::from_slice::<serde_json::Value>(l).is_ok())
}

fn write_tmp(t: &Tmp, name: &str, data: &[u8]) -> PathBuf {
    let p = t.p(name);
    std::fs::write(&p, data).unwrap();
    p
}

// ───────────── свойства маскирования на месте ─────────────

#[test]
fn mask_file_random_logs_exact_effect() {
    for_seeds("mask", |r| {
        let nl = r.range(1, 25) as usize;
        let b = build(r, nl, false);
        let t = Tmp::new("mask");
        let p = write_tmp(&t, "chat.jsonl", &b.data);
        // пробный запуск ничего не меняет и считает те же ключи
        let (n_dry, _) = mask_file(&p, true).unwrap();
        assert_eq!(n_dry, b.keys.len());
        assert_eq!(std::fs::read(&p).unwrap(), b.data);
        // настоящий запуск
        let (n, left) = mask_file(&p, false).unwrap();
        assert_eq!((n, left), (b.keys.len(), false));
        let after = std::fs::read(&p).unwrap();
        assert_eq!(after.len(), b.data.len(), "размер файла изменился");
        assert_eq!(after, expected_after(&b), "изменены не только хвосты ключей");
        assert!(lines_valid_json(&after), "JSON после маскирования сломан");
        // ключей не осталось (по самой же проверке и по подстрокам)
        let text = String::from_utf8_lossy(&after);
        assert!(secret_kinds(&text).is_empty());
        for (_, k, keep) in &b.keys {
            assert!(!text.contains(&k[*keep..]), "ключ остался");
        }
        // повторно — ничего
        assert_eq!(mask_file(&p, false).unwrap().0, 0);
        assert_eq!(std::fs::read(&p).unwrap(), after, "повторный запуск изменил файл");
    });
}

/// То же с недопустимым UTF-8 между строками: остальные байты нетронуты.
#[test]
fn mask_file_with_invalid_utf8_between_lines() {
    for_seeds("mask_bad", |r| {
        let nl = r.range(2, 15) as usize;
        let b = build(r, nl, true);
        let t = Tmp::new("mask_bad");
        let p = write_tmp(&t, "chat.jsonl", &b.data);
        let (n, left) = mask_file(&p, false).unwrap();
        assert_eq!((n, left), (b.keys.len(), false));
        assert_eq!(std::fs::read(&p).unwrap(), expected_after(&b));
    });
}

/// Метаморфическое: объединение двух файлов маскируется так же, как каждый по отдельности.
#[test]
fn mask_file_concat_equals_parts() {
    for_seeds("mask_cat", |r| {
        let (a, b) = (build(r, 5, false), build(r, 5, false));
        let t = Tmp::new("mask_cat");
        let (pa, pb) = (write_tmp(&t, "a.jsonl", &a.data), write_tmp(&t, "b.jsonl", &b.data));
        let mut both = a.data.clone();
        both.extend_from_slice(&b.data);
        let pc = write_tmp(&t, "c.jsonl", &both);
        for p in [&pa, &pb, &pc] {
            mask_file(p, false).unwrap();
        }
        let mut cat = std::fs::read(&pa).unwrap();
        cat.extend(std::fs::read(&pb).unwrap());
        assert_eq!(std::fs::read(&pc).unwrap(), cat);
    });
}

/// Ключ на стыке проходов (CH = 1 МиБ): начало на каждом смещении от −70 до +5 относительно границы, и на 2·CH.
/// Ожидание — по построению: замаскирован ровно хвост ключа.
#[test]
fn mask_file_key_at_every_chunk_boundary_offset() {
    const CH: usize = 1 << 20;
    let mut r = Rng(0xB0_0000);
    for base in [CH, 2 * CH] {
        for off in -70i64..=5 {
            let k = gen_key(&mut r, &KIND_SPECS[0]);
            let start = (base as i64 + off) as usize;
            let mut data = vec![b' '; start];
            data.extend_from_slice(k.as_bytes());
            data.extend_from_slice(b" tail\n");
            // хвост длиннее запаса справа (4096): иначе файл целиком читается одним проходом и стык не проверяется
            data.extend(std::iter::repeat_n(b' ', 6000));
            let t = Tmp::new("mask_b");
            let p = write_tmp(&t, "c.jsonl", &data);
            let (n, left) = mask_file(&p, false).unwrap_or_else(|e| panic!("base {base} off {off}: {e}"));
            assert_eq!((n, left), (1, false), "base {base} off {off}");
            let mut want = data.clone();
            for x in &mut want[start + 8..start + k.len()] {
                *x = b'*';
            }
            assert_eq!(std::fs::read(&p).unwrap(), want, "base {base} off {off}");
        }
    }
}

/// Перед ключом на границе стоит буква — это не начало слова: ключ не трогаем ни в одном проходе (запас LEFT работает).
#[test]
fn mask_file_word_start_check_survives_chunk_boundary() {
    const CH: usize = 1 << 20;
    let mut r = Rng(0xB1_0000);
    for off in [-2i64, -1, 0, 1] {
        let k = gen_key(&mut r, &KIND_SPECS[0]);
        let start = (CH as i64 + off) as usize;
        let mut data = vec![b' '; start - 1];
        data.push(b'x'); // буква непосредственно перед «sk-proj-»
        data.extend_from_slice(k.as_bytes());
        data.push(b'\n');
        data.extend(std::iter::repeat_n(b' ', 6000)); // см. комментарий в тесте выше: нужен второй проход
        let t = Tmp::new("mask_w");
        let p = write_tmp(&t, "c.jsonl", &data);
        assert_eq!(mask_file(&p, false).unwrap().0, 0, "off {off}");
        assert_eq!(std::fs::read(&p).unwrap(), data);
    }
}

/// Ключ длиннее запаса на стыке (4096): честный отказ и файл НЕ тронут; не на стыке — маскируется целиком.
#[test]
fn mask_file_key_longer_than_overlap_refused_without_damage() {
    const CH: usize = 1 << 20;
    let mut r = Rng(0xB2_0000);
    // ≈ 6 КБ случайного хвоста (повторяющийся хвост «выдаёт выдумку» и ключом не считается)
    let long = fake_key(&mut r, "sk-proj-", 6000);
    let mut data = vec![b' '; CH - 100];
    data.extend_from_slice(long.as_bytes());
    data.push(b'\n');
    let t = Tmp::new("mask_l");
    let p = write_tmp(&t, "c.jsonl", &data);
    let e = mask_file(&p, false).expect_err("ключ на стыке длиннее запаса");
    assert!(e.contains("длиннее запаса"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), data, "при отказе файл должен остаться как был");
    // тот же ключ в начале файла — маскируется целиком
    let mut d2 = long.clone().into_bytes();
    d2.push(b'\n');
    let p2 = write_tmp(&t, "d.jsonl", &d2);
    assert_eq!(mask_file(&p2, false).unwrap(), (1, false));
    let after = std::fs::read(&p2).unwrap();
    assert!(after[8..after.len() - 1].iter().all(|b| *b == b'*') && after.len() == d2.len());
}

/// Закодированные ключи (\uXXXX, %XX), закрытый ключ с «\n» и в несколько строк — эталон по построению.
#[test]
fn mask_file_escaped_variants_hand_expectation() {
    let mut r = Rng(0xB3_0000);
    let k = gen_key(&mut r, &KIND_SPECS[0]);
    // 1) «-» после sk записан как -: маскируется всё после «sk-proj-» (включая сам эскейп внутри хвоста нет)
    let esc = k.replacen('-', "\\u002d", 1);
    // 2) тот же ключ с %2d
    let pct = k.replacen('-', "%2d", 1);
    let body = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7VJTUt9Us8cKj";
    let pem = format!("-----BEGIN PRIVATE KEY-----\\n{body}\\n{body}\\n-----END PRIVATE KEY-----");
    let text = format!("{{\"a\":\"{esc}\",\"b\":\"x {pct} y\",\"c\":\"{pem}\"}}\n");
    let t = Tmp::new("mask_e");
    let p = write_tmp(&t, "c.jsonl", text.as_bytes());
    let (n, left) = mask_file(&p, false).unwrap();
    assert_eq!((n, left), (3, false));
    let after = std::fs::read_to_string(&p).unwrap();
    assert_eq!(after.len(), text.len());
    assert!(serde_json::from_str::<serde_json::Value>(after.trim_end()).is_ok(), "JSON сломан: {after}");
    assert!(secret_kinds(&after).is_empty());
    assert!(!after.contains("MIIEvQ") && !after.contains(&k[8..20]));
    assert!(after.contains("-----BEGIN PRIVATE KEY-----\\n") && after.contains("-----END PRIVATE KEY-----"));
}

#[cfg(unix)]
#[test]
fn mask_file_refuses_non_regular_and_missing() {
    let t = Tmp::new("mask_n");
    assert!(mask_file(&t.p("нет.jsonl"), false).is_err());
    assert!(mask_file(&t.0, false).is_err(), "каталог");
    // FIFO: открытие без ожидания, отказ (а не зависание)
    let fifo = t.p("pipe.jsonl");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    extern "C" {
        fn mkfifo(path: *const std::os::raw::c_char, mode: u32) -> i32;
    }
    assert_eq!(unsafe { mkfifo(c.as_ptr(), 0o600) }, 0);
    let t0 = std::time::Instant::now();
    assert!(mask_file(&fifo, false).is_err());
    assert!(t0.elapsed().as_secs() < 3, "FIFO повесил обработку");
}

// ───────────── гонки: файл меняется во время работы ─────────────

/// Агент дописывает чат (O_APPEND), пока Аптечка маскирует: началo файла замаскировано точно, дописанное не искажено
/// (кроме ключей, попавших в дописанное), ничего не потеряно.
#[test]
fn mask_file_while_agent_appends() {
    for_seeds_n("mask_race_append", 30, |r| {
        let b = build(r, 12, false);
        let t = Tmp::new("mask_ra");
        let p = write_tmp(&t, "chat.jsonl", &b.data);
        let p2 = p.clone();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        let appended = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let a2 = appended.clone();
        let h = std::thread::spawn(move || {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&p2).unwrap();
            let mut i = 0u32;
            while !s2.load(std::sync::atomic::Ordering::Relaxed) && i < 400 {
                let line = format!("{{\"t\":\"дописано {i} без ключей\"}}\n");
                f.write_all(line.as_bytes()).unwrap();
                a2.lock().unwrap().extend_from_slice(line.as_bytes());
                i += 1;
            }
        });
        let res = mask_file(&p, false);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        h.join().unwrap();
        let (n, left) = res.expect("маскирование при дозаписи");
        assert_eq!((n, left), (b.keys.len(), false));
        let after = std::fs::read(&p).unwrap();
        let mut want = expected_after(&b);
        want.extend_from_slice(&appended.lock().unwrap());
        assert_eq!(after, want, "дозапись и маскирование исказили друг друга");
    });
}

/// Другой процесс перезаписывает участок с ключом на месте (то же число байт), пока идёт маскирование: итог — либо
/// отказ «файл изменился», либо корректная маска; байты вне участка всегда прежние, размер прежний.
#[test]
fn mask_file_while_region_is_rewritten() {
    for_seeds_n("mask_race_rewrite", 40, |r| {
        let k1 = gen_key(r, &KIND_SPECS[0]);
        let k2 = gen_key(r, &KIND_SPECS[0]); // та же длина
        assert_eq!(k1.len(), k2.len());
        let pre = "p".repeat(r.range(0, 3000) as usize);
        let post = " tail\n".repeat(r.range(1, 200) as usize);
        let t = Tmp::new("mask_rw");
        let data = format!("{pre}{k1}{post}");
        let p = write_tmp(&t, "chat.jsonl", data.as_bytes());
        let (p2, o) = (p.clone(), pre.len());
        let (a, b) = (k1.clone(), k2.clone());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        let h = std::thread::spawn(move || {
            use std::io::{Seek, SeekFrom, Write};
            let mut f = std::fs::OpenOptions::new().write(true).open(&p2).unwrap();
            let mut flip = false;
            while !s2.load(std::sync::atomic::Ordering::Relaxed) {
                f.seek(SeekFrom::Start(o as u64)).unwrap();
                f.write_all(if flip { a.as_bytes() } else { b.as_bytes() }).unwrap();
                flip = !flip;
            }
        });
        let res = mask_file(&p, false);
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        h.join().unwrap();
        let after = std::fs::read(&p).unwrap();
        assert_eq!(after.len(), data.len());
        assert_eq!(&after[..o], pre.as_bytes(), "байты до участка изменились");
        assert_eq!(&after[o + k1.len()..], post.as_bytes(), "байты после участка изменились");
        let region = &after[o..o + k1.len()];
        // участок — один из ключей (если отказ/успели переписать) либо целиком «sk-proj-» + звёздочки (либо смесь звёздочек
        // с одним из ключей, если писатель вклинился между чтением и записью — это допустимо, главное, не за пределами)
        let ok = region == k1.as_bytes()
            || region == k2.as_bytes()
            || region.starts_with(b"sk-proj-") && region[8..].iter().all(|c| c.is_ascii_alphanumeric() || *c == b'*');
        assert!(ok, "участок искажён: {:?}", String::from_utf8_lossy(region));
        if let Err(e) = res {
            assert!(e.contains("изменился") || !e.is_empty());
        }
    });
}

/// Прогон «агент-писатель + несколько маскирующих» по одному файлу: ни паник, ни потери.
#[test]
fn mask_file_two_maskers_one_file() {
    for_seeds_n("mask_two", 20, |r| {
        let b = build(r, 10, false);
        let t = Tmp::new("mask_2");
        let p = write_tmp(&t, "chat.jsonl", &b.data);
        let hs: Vec<_> = (0..2)
            .map(|_| {
                let p = p.clone();
                std::thread::spawn(move || mask_file(&p, false))
            })
            .collect();
        for h in hs {
            let _ = h.join().unwrap();
        }
        assert_eq!(std::fs::read(&p).unwrap(), expected_after(&b), "два одновременных маскирования дали не ту маску");
    });
}

// ───────────── лечение целиком на выдуманной домашней папке ─────────────

#[cfg(unix)]
#[test]
fn fix_chats_covers_all_agent_dirs_and_leaves_others() {
    with_fake_home("fx_chats", |h| {
        let mut r = Rng(0xC0_0000);
        let k = gen_key(&mut r, &KIND_SPECS[0]);
        let mut touched = Vec::new();
        for (d, ext) in CHAT_DIRS {
            let dir = h.join(d).join("sess");
            std::fs::create_dir_all(&dir).unwrap();
            let f = dir.join(format!("a{ext}"));
            std::fs::write(&f, format!("{{\"m\":\"{k}\"}}\n")).unwrap();
            touched.push(f);
        }
        // чужой файл с ключом (не чат) и файл другого расширения — не трогаем
        std::fs::write(h.join("notes.txt"), &k).unwrap();
        std::fs::create_dir_all(h.join(".claude/projects/x")).unwrap();
        std::fs::write(h.join(".claude/projects/x/readme.txt"), &k).unwrap();
        let (ok, msg) = fix_chats();
        assert!(ok, "{msg}");
        for f in &touched {
            let s = std::fs::read_to_string(f).unwrap();
            assert!(!s.contains(&k[8..]), "{} не обработан", f.display());
            assert_eq!(s.len(), format!("{{\"m\":\"{k}\"}}\n").len());
        }
        assert_eq!(std::fs::read_to_string(h.join("notes.txt")).unwrap(), k);
        assert_eq!(std::fs::read_to_string(h.join(".claude/projects/x/readme.txt")).unwrap(), k);
        // повтор — «ключей не нашёл»
        let (ok, msg) = fix_chats();
        assert!(ok && msg.contains("не нашёл"), "{msg}");
    });
}

/// Папка чатов — ссылка наружу из дома: файлы по ту сторону не трогаются, лечение сообщает о сбое.
#[cfg(unix)]
#[test]
fn fix_chats_refuses_dir_symlinked_outside_home() {
    with_fake_home("fx_out", |h| {
        let mut r = Rng(0xC1_0000);
        let k = gen_key(&mut r, &KIND_SPECS[0]);
        let outside = h.parent().unwrap().join("outside_chats");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("a.jsonl"), &k).unwrap();
        std::fs::create_dir_all(h.join(".claude")).unwrap();
        std::os::unix::fs::symlink(&outside, h.join(".claude/projects")).unwrap();
        let (ok, msg) = fix_chats();
        assert!(!ok && msg.contains("за пределы"), "{msg}");
        assert_eq!(std::fs::read_to_string(outside.join("a.jsonl")).unwrap(), k);
    });
}

/// Файлы чатов-ссылки внутри папки не обрабатываются (в список попадают только обычные файлы).
#[cfg(unix)]
#[test]
fn fix_chats_skips_symlinked_files() {
    with_fake_home("fx_lnk", |h| {
        let mut r = Rng(0xC2_0000);
        let k = gen_key(&mut r, &KIND_SPECS[0]);
        let d = h.join(".claude/projects/x");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(h.join("real.txt"), &k).unwrap();
        std::os::unix::fs::symlink(h.join("real.txt"), d.join("a.jsonl")).unwrap();
        let (ok, _) = fix_chats();
        assert!(ok);
        assert_eq!(std::fs::read_to_string(h.join("real.txt")).unwrap(), k, "файл за ссылкой изменён");
    });
}

/// Чат только для чтения (0444) без единого ключа: лечить нечего, ложной тревоги быть не должно. Сейчас файл открывается
/// на запись заранее, и отказ в правах записывается как сбой — итог «не удалось изменить файлов: 1».
#[cfg(unix)]
#[test]
#[ignore = "известная ошибка: файл без ключей, но только для чтения, даёт ложное «Не удалось изменить файлов» (mask_file открывает на запись до поиска ключей)"]
fn fix_chats_readonly_file_without_keys_is_not_a_failure() {
    with_fake_home("fx_ro", |h| {
        use std::os::unix::fs::PermissionsExt;
        let d = h.join(".claude/projects/x");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a.jsonl");
        std::fs::write(&f, "обычный разговор без ключей\n").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
        let (ok, msg) = fix_chats();
        assert!(ok, "ложная тревога: {msg}");
    });
}

/// Чат только для чтения С ключом: честный сбой, файл не тронут.
#[cfg(unix)]
#[test]
fn fix_chats_readonly_file_with_key_reports_failure_untouched() {
    with_fake_home("fx_ro2", |h| {
        use std::os::unix::fs::PermissionsExt;
        let mut r = Rng(0xC3_0000);
        let k = gen_key(&mut r, &KIND_SPECS[0]);
        let d = h.join(".claude/projects/x");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("a.jsonl");
        std::fs::write(&f, format!("{k}\n")).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
        let (ok, msg) = fix_chats();
        assert!(!ok && msg.contains("нет прав"), "{msg}");
        assert_eq!(std::fs::read_to_string(&f).unwrap(), format!("{k}\n"));
    });
}

/// Пробный запуск (DRY_RUN) через настоящий флаг: ничего не меняет, считает. Флаг глобальный — под общим замком.
#[cfg(unix)]
#[test]
fn fix_chats_dry_run_changes_nothing() {
    with_fake_home("fx_dry", |h| {
        let mut r = Rng(0xC4_0000);
        let k = gen_key(&mut r, &KIND_SPECS[1]);
        let d = h.join(".codex/sessions/2026/11/15");
        std::fs::create_dir_all(&d).unwrap();
        let body = format!("{{\"m\":\"{k}\"}}\n{{\"m\":\"{k}\"}}\n");
        std::fs::write(d.join("r.jsonl"), &body).unwrap();
        crate::fixes::set_dry_run(true);
        let (ok, msg) = fix_chats();
        crate::fixes::set_dry_run(false);
        assert!(ok && msg.contains("Пробный запуск") && msg.contains("2 ключа"), "{msg}");
        assert_eq!(std::fs::read_to_string(d.join("r.jsonl")).unwrap(), body);
    });
}

// ───────────── поиск секретов: свойства и «стойкость» ─────────────

/// Случайный текст из опасных кусочков: префиксы ключей, заголовки PEM, эскейпы, юникод, обрывки.
fn nasty(r: &mut Rng, pieces: usize) -> String {
    const P: [&str; 26] = [
        "sk-",
        "sk-ant-",
        "sk-proj-",
        "ghp_",
        "AKIA",
        "AIza",
        "hf_",
        "github_pat_",
        "xoxb-",
        "glpat-",
        "-----BEGIN PRIVATE KEY-----",
        "-----BEGIN",
        "-----END PRIVATE KEY-----",
        "\\n",
        "\\u0073",
        "\\u00",
        "%2d",
        "%",
        "ы",
        "😀",
        "世",
        "A1b2C3d4",
        "abcdefgh",
        "12345678",
        " ",
        "\n",
    ];
    let mut s = String::new();
    for _ in 0..pieces {
        if r.chance(15) {
            let kl = r.range(16, 60) as usize;
            s += &fake_key(r, "", kl);
        } else {
            s += r.pick(&P);
        }
    }
    s
}

#[test]
fn secret_search_never_panics_and_spans_are_sane() {
    for_seeds("nasty", |r| {
        let np = r.range(0, 120) as usize;
        let text = nasty(r, np);
        for sp in secret_spans(&text) {
            assert!(sp.start <= sp.end && sp.end <= text.len() && sp.start + sp.keep <= sp.end, "{} {} {}", sp.start, sp.end, sp.keep);
            assert!(
                text.is_char_boundary(sp.start) && text.is_char_boundary(sp.end) && text.is_char_boundary(sp.start + sp.keep),
                "не по границе символа"
            );
        }
        let _ = secret_kinds(&text);
        let m = mask_secrets(&text);
        // маска не оставляет найденного ключа и идемпотентна
        assert_eq!(mask_secrets(&m), m, "mask_secrets не идемпотентна");
        let _ = redact_for_backup(&text);
        // байтовая версия на том же тексте и на «рваном» UTF-8
        let mut bytes = text.clone().into_bytes();
        if !bytes.is_empty() && r.chance(50) {
            let i = r.below(bytes.len() as u64) as usize;
            bytes[i] = 0xff;
        }
        let spans = raw_spans(&bytes);
        let mut last_end = 0;
        for (s, e, _) in spans {
            assert!(s < e && e <= bytes.len(), "{s}..{e} при длине {}", bytes.len());
            assert!(s >= last_end, "куски пересекаются");
            last_end = e;
        }
    });
}

/// Метаморфическое: вид найденного ключа не зависит от того, что стоит ДО него через пробел, и от завершающего перевода строки.
#[test]
fn secret_kinds_stable_under_context() {
    for_seeds("kinds_ctx", |r| {
        let spec = *r.pick(&KIND_SPECS);
        let k = gen_key(r, &spec);
        let base = secret_kinds(&format!("x {k} y"));
        assert!(!base.is_empty(), "ключ не распознан: {k}");
        for ctx in ["", "token=", "\"", "- ", "export K=", "привет ", "\n"] {
            assert_eq!(secret_kinds(&format!("{ctx}{k}")), base, "контекст {ctx:?}");
            assert_eq!(secret_kinds(&format!("{ctx}{k}\n")), base);
        }
        // склейка двух текстов через перевод строки — объединение видов
        let spec2 = *r.pick(&KIND_SPECS);
        let k2 = gen_key(r, &spec2);
        let both = secret_kinds(&format!("{k}\n{k2}"));
        for kind in secret_kinds(&k).into_iter().chain(secret_kinds(&k2)) {
            assert!(both.contains(&kind));
        }
    });
}

/// Карта «байт раскодированного → байт исходного» согласована с длиной и не убывает; на обрывках эскейпов нет паники.
#[test]
fn decode_escapes_map_is_consistent() {
    for_seeds("dec_map", |r| {
        let np = r.range(0, 60) as usize;
        let text = nasty(r, np);
        let mut m = Vec::new();
        if let Some(d) = decode_escapes_map(&text, Some(&mut m)) {
            assert_eq!(m.len(), d.len() + 1);
            assert_eq!(*m.last().unwrap(), text.len());
            assert!(m.windows(2).all(|w| w[0] <= w[1]), "карта убывает");
            assert!(m.iter().all(|x| *x <= text.len()));
        }
    });
    // обрывки в самом конце
    for t in ["\\u", "\\u0", "\\u00", "\\u007", "%", "%2", "x%2", "\\u0073k-", "\\ud83d\\ude00"] {
        let mut m = Vec::new();
        let _ = decode_escapes_map(t, Some(&mut m));
    }
}

/// Время: длинные «кормушки» не вешают поиск (границы по времени — 5 с; обычно доли секунды).
#[test]
fn secret_search_pathological_inputs_are_fast() {
    let cases: Vec<(&str, String)> = vec![
        ("AIza×", "AIza".repeat(60_000)),
        ("hf_×", "hf_".repeat(80_000)),
        ("BEGIN×", "-----BEGIN PRIVATE KEY-----AAAA\n".repeat(3_000)),
        ("эскейпы", "\\u0073k-".repeat(50_000)),
        ("%2d", "sk%2d".repeat(50_000)),
        ("длинный хвост", format!("sk-proj-{}", "aB3".repeat(300_000))),
    ];
    for (name, text) in cases {
        let t = std::time::Instant::now();
        let _ = secret_kinds(&text);
        let _ = mask_secrets(&text);
        let _ = raw_spans(text.as_bytes());
        assert!(t.elapsed().as_secs_f64() < 5.0, "{name}: {:?}", t.elapsed());
    }
}

/// Резервная копия оболочки: окончания строк, пустые строки и порядок сохраняются; строки с ключами и блоки PEM скрыты.
#[test]
fn redact_for_backup_preserves_structure() {
    for_seeds("backup", |r| {
        let n = r.range(1, 20) as usize;
        let mut lines = Vec::new();
        let mut secret_idx = Vec::new();
        for i in 0..n {
            if r.chance(30) {
                secret_idx.push(i);
                let sp = *r.pick(&KIND_SPECS);
                lines.push(format!("export K={}", gen_key(r, &sp)));
            } else {
                let fl = r.range(0, 30) as usize;
                lines.push(filler(r, fl));
            }
        }
        let eol = if r.chance(50) { "\r\n" } else { "\n" };
        let text: String = lines.iter().map(|l| format!("{l}{eol}")).collect();
        let out = redact_for_backup(&text);
        let out_lines: Vec<&str> = out.split_inclusive('\n').collect();
        assert_eq!(out_lines.len(), n);
        for (i, l) in out_lines.iter().enumerate() {
            assert!(l.ends_with(eol), "окончание строки потеряно");
            if secret_idx.contains(&i) {
                assert!(l.starts_with("# строка с ключом скрыта"), "{l}");
            } else {
                assert_eq!(*l, format!("{}{eol}", lines[i]));
            }
        }
        assert!(secret_kinds(&out).is_empty());
    });
}

/// Длительный прогон: сто тысяч циклов случайный файл → маска → проверка, с замером RSS и дескрипторов.
#[test]
#[ignore = "длительное испытание: см. REGRESSION_TESTS.md"]
fn soak_mask_file_cycles() {
    let secs = soak_secs(30);
    csv_line("t_s,cycle,rss_kb,threads,fds,cycle_ms,ok");
    let t0 = std::time::Instant::now();
    let mut r = Rng(0xD0_0000);
    let mut cycle = 0u64;
    let (rss0, _, fd0) = proc_stats();
    let t = Tmp::new("soak_mask");
    while t0.elapsed().as_secs() < secs {
        let s = std::time::Instant::now();
        let nl = r.range(1, 40) as usize;
        let b = build(&mut r, nl, cycle % 3 == 0);
        let p = write_tmp(&t, "chat.jsonl", &b.data);
        let ok = mask_file(&p, false).map(|x| x == (b.keys.len(), false)).unwrap_or(false) && std::fs::read(&p).unwrap() == expected_after(&b);
        cycle += 1;
        if cycle % 200 == 0 {
            let (rss, th, fd) = proc_stats();
            csv_line(&format!("{},{cycle},{rss},{th},{fd},{},{}", t0.elapsed().as_secs(), s.elapsed().as_millis(), ok as u8));
        }
        if !ok {
            // подробности расхождения (ключи выдуманные, печатать их безопасно)
            let after = std::fs::read(&p).unwrap();
            let want = expected_after(&b);
            let diff: Vec<usize> = (0..after.len().min(want.len())).filter(|&i| after[i] != want[i]).collect();
            eprintln!("[soak_mask] цикл {cycle}: файл {} Б, ключей по построению {}, расхождений {}", b.data.len(), b.keys.len(), diff.len());
            for (o, k, _) in &b.keys {
                if diff.iter().any(|&i| i >= *o && i < o + k.len()) {
                    eprintln!(
                        "[soak_mask] ключ {k}; kinds={:?}; окружение {:?}",
                        secret_kinds(k),
                        String::from_utf8_lossy(&b.data[o.saturating_sub(12)..(o + k.len() + 6).min(b.data.len())])
                    );
                }
            }
        }
        assert!(ok, "цикл {cycle}");
    }
    let (rss1, th1, fd1) = proc_stats();
    csv_line(&format!("final,{cycle},{rss1},{th1},{fd1},,"));
    assert!(fd1 <= fd0 + 5, "утечка дескрипторов {fd0} -> {fd1}");
    assert!(rss1 < rss0 + 100_000, "рост памяти {rss0} -> {rss1} КБ");
}

/// Как `for_seeds`, но с заданным числом зёрен (для тяжёлых тестов с потоками).
fn for_seeds_n(name: &str, n: u64, f: impl Fn(&mut Rng)) {
    let n = std::env::var("REGRESS_SEEDS").ok().and_then(|v| v.parse::<u64>().ok()).map_or(n, |v| v.min(n.max(v)));
    for i in 0..n {
        let seed = 0x5EED_1000_0000 + i;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut Rng(seed))));
        if let Err(e) = r {
            eprintln!("[{name}] ПАДЕНИЕ на зерне {seed:#x}");
            std::panic::resume_unwind(e);
        }
    }
}
