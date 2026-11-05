//! Цвета под профиль монитора (управление цветом).
//!
//! Картинки и цвета Аптечки — в sRGB. Мониторы с широким охватом (P3 и шире) показывают значения sRGB
//! «как есть» перенасыщенными: красный и зелёный у них сочнее. Программы просмотра картинок пересчитывают
//! цвета под профиль монитора — поэтому концепт в просмотрщике спокойнее, чем тот же рисунок в Аптечке.
//! Здесь — тот же пересчёт: sRGB → XYZ (D50) → цвета монитора по его ICC-профилю (матрица + кривые).
//! Профиль: Linux (X11) — свойство _ICC_PROFILE корневого окна (его ставит GNOME/colord по данным монитора);
//! Windows — GetICMProfileW. macOS пересчитывает сама (окно отдаётся в пространстве экрана, см. vendor/softbuffer).
//! Нет профиля, он не матричный или почти как sRGB — ничего не делаем.

use std::sync::OnceLock;

pub struct Xform {
    /// sRGB 0..255 → линейная 0..1
    dec: [f32; 256],
    /// линейные sRGB → линейные цвета монитора
    m: [[f32; 3]; 3],
    /// линейная 0..1 (4096 ступеней) → значение монитора 0..255
    enc: Vec<u8>,
}

static XF: OnceLock<Option<Xform>> = OnceLock::new();

/// Пересчёт для этого монитора (один раз за запуск; APTECHKA_NO_CM=1 — выключить).
pub fn get() -> Option<&'static Xform> {
    XF.get_or_init(|| {
        if std::env::var_os("APTECHKA_NO_CM").is_some() {
            return None;
        }
        let icc = load_profile()?;
        let x = Xform::from_icc(&icc);
        if let Some(x) = &x {
            eprintln!("[цвет] профиль монитора: пересчёт sRGB → монитор (матрица {:?})", x.m.map(|r| r.map(|v| (v * 1000.0).round() / 1000.0)));
        }
        x
    })
    .as_ref()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn load_profile() -> Option<Vec<u8>> {
    // xprop -root -notype _ICC_PROFILE → «_ICC_PROFILE = 0, 0, 6, 140, …»
    // не дольше 3 с: зависший X-сервер (ssh -X) не должен держать первый кадр окна
    let r = crate::cmd::exec(&["xprop", "-root", "-notype", "_ICC_PROFILE"], std::time::Duration::from_secs(3), crate::cmd::CHECK);
    let out = std::process::Output { status: Default::default(), stdout: r.text().as_bytes().to_vec(), stderr: Vec::new() };
    if r.code() != 0 {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let list = s.split_once('=')?.1;
    let b: Vec<u8> = list.split(',').filter_map(|x| x.trim().parse::<i64>().ok()).map(|v| v as u8).collect();
    (b.len() > 132).then_some(b)
}

#[cfg(windows)]
fn load_profile() -> Option<Vec<u8>> {
    use core::ffi::c_void;
    #[link(name = "user32")]
    extern "system" {
        fn GetDC(h: *mut c_void) -> *mut c_void;
        fn ReleaseDC(h: *mut c_void, dc: *mut c_void) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn GetICMProfileW(dc: *mut c_void, len: *mut u32, name: *mut u16) -> i32;
    }
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let r = GetICMProfileW(dc, &mut len, buf.as_mut_ptr());
        ReleaseDC(std::ptr::null_mut(), dc);
        r
    };
    if ok == 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    std::fs::read(String::from_utf16_lossy(&buf[..n])).ok()
}

/// macOS: профиль того пространства, в котором softbuffer помечает кадр (главный экран, у MacBook — Display P3).
/// Кадр рисуется в sRGB — пересчитываем его в это пространство сами (так система не перекодирует каждый кадр
/// на процессоре, а цвета не перенасыщены, как было, когда sRGB-числа просто помечались как P3).
#[cfg(target_os = "macos")]
fn load_profile() -> Option<Vec<u8>> {
    softbuffer::macos_display_icc()
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn s15(b: &[u8], o: usize) -> Option<f64> {
    Some(be32(b, o)? as i32 as f64 / 65536.0)
}

/// Кривая тона профиля (curv или para): значение 0..1 → линейная 0..1.
fn trc(b: &[u8], o: usize) -> Option<Box<dyn Fn(f64) -> f64>> {
    match b.get(o..o + 4)? {
        b"curv" => {
            let n = be32(b, o + 8)? as usize;
            if n == 0 {
                return Some(Box::new(|x| x));
            }
            if n == 1 {
                let g = u16::from_be_bytes([*b.get(o + 12)?, *b.get(o + 13)?]) as f64 / 256.0;
                return Some(Box::new(move |x: f64| x.powf(g)));
            }
            // таблица должна целиком лежать в профиле (испорченный профиль не должен ронять программу)
            if n > 65536 || b.len() < o + 12 + 2 * n {
                return None;
            }
            let t: Vec<f64> = (0..n).map(|i| u16::from_be_bytes([b[o + 12 + 2 * i], b[o + 13 + 2 * i]]) as f64 / 65535.0).collect();
            Some(Box::new(move |x: f64| {
                let p = x.clamp(0.0, 1.0) * (t.len() - 1) as f64;
                let i = (p.floor() as usize).min(t.len() - 2);
                t[i] + (t[i + 1] - t[i]) * (p - i as f64)
            }))
        }
        b"para" => {
            let kind = u16::from_be_bytes([*b.get(o + 8)?, *b.get(o + 9)?]);
            let n = [1, 3, 4, 5, 7].get(kind as usize).copied()?;
            let p: Vec<f64> = (0..n).map(|i| s15(b, o + 12 + 4 * i)).collect::<Option<_>>()?;
            let g = p[0];
            Some(match kind {
                0 => Box::new(move |x: f64| x.powf(g)),
                1 => Box::new(move |x: f64| if x >= -p[2] / p[1] { (p[1] * x + p[2]).max(0.0).powf(g) } else { 0.0 }),
                2 => Box::new(move |x: f64| if x >= -p[2] / p[1] { (p[1] * x + p[2]).max(0.0).powf(g) + p[3] } else { p[3] }),
                3 => Box::new(move |x: f64| if x >= p[4] { (p[1] * x + p[2]).max(0.0).powf(g) } else { p[3] * x }),
                _ => Box::new(move |x: f64| if x >= p[4] { (p[1] * x + p[2]).max(0.0).powf(g) + p[5] } else { p[3] * x + p[6] }),
            })
        }
        _ => None,
    }
}

fn inv3(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let d = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if d.abs() < 1e-9 {
        return None;
    }
    let c = |a: usize, b: usize, c: usize, e: usize| m[a][b] * m[c][e] - m[a][e] * m[c][b];
    Some([
        [c(1, 1, 2, 2) / d, -c(0, 1, 2, 2) / d, c(0, 1, 1, 2) / d],
        [-c(1, 0, 2, 2) / d, c(0, 0, 2, 2) / d, -c(0, 0, 1, 2) / d],
        [c(1, 0, 2, 1) / d, -c(0, 0, 2, 1) / d, c(0, 0, 1, 1) / d],
    ])
}

impl Xform {
    fn from_icc(b: &[u8]) -> Option<Xform> {
        let n = (be32(b, 128)? as usize).min(1000);
        let tag = |sig: &[u8; 4]| -> Option<usize> {
            (0..n).find_map(|i| {
                let o = 132 + 12 * i;
                (b.get(o..o + 4)? == sig).then(|| be32(b, o + 4).map(|v| v as usize)).flatten()
            })
        };
        let xyz = |sig: &[u8; 4]| -> Option<[f64; 3]> {
            let o = tag(sig)?;
            Some([s15(b, o + 8)?, s15(b, o + 12)?, s15(b, o + 16)?])
        };
        let (r, g, bl) = (xyz(b"rXYZ")?, xyz(b"gXYZ")?, xyz(b"bXYZ")?);
        // монитор: линейные RGB → XYZ (D50), столбцы — основные цвета
        let d = [[r[0], g[0], bl[0]], [r[1], g[1], bl[1]], [r[2], g[2], bl[2]]];
        let di = inv3(d)?;
        // sRGB → XYZ D50 (адаптация Брэдфорда, как в профилях ICC)
        let s = [[0.4360747, 0.3850649, 0.1430804], [0.2225045, 0.7168786, 0.0606169], [0.0139322, 0.0971045, 0.7141733]];
        let mut m = [[0f32; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = (0..3).map(|k| di[i][k] * s[k][j]).sum::<f64>() as f32;
            }
        }
        // три кривые; берём по каналу свою
        let curves = [tag(b"rTRC")?, tag(b"gTRC")?, tag(b"bTRC")?].map(|o| trc(b, o));
        let curves: Vec<Box<dyn Fn(f64) -> f64>> = curves.into_iter().collect::<Option<_>>()?;
        // кодирование: линейная → значение монитора (обратная кривая, по зелёному — каналы почти всегда одинаковы)
        let f = &curves[1];
        let fwd: Vec<f64> = (0..=1024).map(|i| f(i as f64 / 1024.0)).collect();
        let enc: Vec<u8> = (0..4096)
            .map(|i| {
                let y = i as f64 / 4095.0;
                let k = fwd.partition_point(|&v| v < y).min(1024);
                (k as f64 / 1024.0 * 255.0).round() as u8
            })
            .collect();
        let mut dec = [0f32; 256];
        for (i, v) in dec.iter_mut().enumerate() {
            let c = i as f64 / 255.0;
            *v = (if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }) as f32;
        }
        let x = Xform { dec, m, enc };
        // почти sRGB (обычный монитор) — не трогаем
        let near = (0..3).all(|i| (0..3).all(|j| (x.m[i][j] - if i == j { 1.0 } else { 0.0 }).abs() < 0.02));
        let srgb_trc = (0..=16).all(|i| {
            let c = i as f64 / 16.0;
            let lin = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
            (x.enc[(lin * 4095.0).round() as usize] as f64 - c * 255.0).abs() < 3.0
        });
        (!(near && srgb_trc)).then_some(x)
    }

    #[inline]
    fn px(&self, v: u32) -> u32 {
        let (r, g, b) = (self.dec[((v >> 16) & 255) as usize], self.dec[((v >> 8) & 255) as usize], self.dec[(v & 255) as usize]);
        let m = &self.m;
        let e = |x: f32| self.enc[(x.clamp(0.0, 1.0) * 4095.0 + 0.5) as usize] as u32;
        (v & 0xFF00_0000)
            | e(m[0][0] * r + m[0][1] * g + m[0][2] * b) << 16
            | e(m[1][0] * r + m[1][1] * g + m[1][2] * b) << 8
            | e(m[2][0] * r + m[2][1] * g + m[2][2] * b)
    }

    /// Пересчитать непрозрачный кадр (XRGB) в прямоугольнике (x, y, w, h) буфера шириной bw.
    pub fn apply_rect(&self, buf: &mut [u32], bw: usize, x: usize, y: usize, w: usize, h: usize) {
        // память последней точки — «пусто» в начале (раньше u32::MAX: белая непрозрачная точка 0xFFFFFFFF
        // первой в кадре принималась за уже пересчитанную и становилась нулём — прозрачной/чёрной)
        let mut last: Option<(u32, u32)> = None;
        for row in y..y + h {
            let Some(line) = buf.get_mut(row * bw + x..(row * bw + x + w).min((row + 1) * bw)) else { break };
            for p in line {
                // подряд одинаковые точки (фон, плашки) — один раз
                let out = match last {
                    Some((i, o)) if i == *p => o,
                    _ => {
                        let o = self.px(*p);
                        last = Some((*p, o));
                        o
                    }
                };
                *p = out;
            }
        }
    }

    /// Пересчитать кадр с предумноженной альфой (виджет): прозрачное пропускаем, край — без предумножения.
    pub fn apply_premul(&self, buf: &mut [u32]) {
        let mut last: Option<(u32, u32)> = None;
        for p in buf.iter_mut() {
            let a = *p >> 24;
            if a == 0 {
                continue;
            }
            if let Some((i, o)) = last {
                if i == *p {
                    *p = o;
                    continue;
                }
            }
            let last_in = *p;
            *p = if a == 255 {
                self.px(*p)
            } else {
                let un = |s: u32| (((*p >> s) & 255) * 255 / a).min(255);
                let c = self.px(0xFF00_0000 | un(16) << 16 | un(8) << 8 | un(0));
                let pm = |s: u32| ((c >> s) & 255) * a / 255;
                a << 24 | pm(16) << 16 | pm(8) << 8 | pm(0)
            };
            last = Some((last_in, *p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Белая точка первой в кадре остаётся белой (не становится прозрачной), и одинаковые подряд — тоже.
    #[test]
    fn first_white_survives() {
        let x = Xform { dec: [0.0; 256], m: [[0.0; 3]; 3], enc: vec![255; 4096] };
        let mut b = [0xFFFF_FFFF, 0xFFFF_FFFF];
        x.apply_premul(&mut b);
        assert_eq!(b, [0xFFFF_FFFF, 0xFFFF_FFFF]);
        let mut r = [0xFFFF_FFFFu32; 4];
        x.apply_rect(&mut r, 4, 0, 0, 4, 1);
        assert!(r.iter().all(|p| *p == 0xFFFF_FFFF));
    }
}
