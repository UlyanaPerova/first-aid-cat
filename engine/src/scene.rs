//! Слои картинки: загрузка из art/layers (layers.json + PNG) и сборка кадра.

/// Один слой: на весь холст, прозрачное = нет пикселя.
pub struct Layer {
    pub name: String,
    /// Пиксели только внутри bbox (строка шириной x1−x0) — пустоту вокруг не храним: так картинки
    /// занимают в памяти в разы меньше. 0xAARRGGBB, AA = 0 или 255.
    pub px: Vec<u32>,
    /// рамка непрозрачной части (x0, y0, x1, y1) — по ней и рисуем, чтобы не гонять пустоту
    pub bbox: (usize, usize, usize, usize),
    /// позы слоя (кадры анимации): имя кадра, пиксели внутри рамки, рамка — рисуются вместо px
    pub poses: Vec<(String, Vec<u32>, (usize, usize, usize, usize))>,
}

impl Layer {
    /// Пиксель слоя в клетке (x, y) холста (0 — пусто).
    pub fn at(&self, x: usize, y: usize) -> u32 {
        let (x0, y0, x1, y1) = self.bbox;
        if x < x0 || x >= x1 || y < y0 || y >= y1 {
            return 0;
        }
        self.px[(y - y0) * (x1 - x0) + (x - x0)]
    }
}

/// Вырезать рамку из картинки на весь холст.
fn crop(px: &[u32], w: usize, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> Vec<u32> {
    let mut v = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        v.extend_from_slice(&px[y * w + x0..y * w + x1]);
    }
    v
}

fn bbox_of(px: &[u32], w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if px[y * w + x] >> 24 != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 == 0 {
        (0, 0, 0, 0)
    } else {
        (x0, y0, x1, y1)
    }
}

pub struct Scene {
    pub w: usize,
    pub h: usize,
    pub layers: Vec<Layer>, // от дальнего к ближнему
}

pub(crate) fn load_png(dir: &str, rel: &str) -> (usize, usize, Vec<u32>) {
    let bytes = crate::art::read(dir, rel).unwrap_or_else(|| panic!("нет картинки {rel}"));
    let dec = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut rd = dec.read_info().expect("png");
    let mut buf = vec![0; rd.output_buffer_size()];
    let info = rd.next_frame(&mut buf).expect("кадр png");
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = info.color_type.samples();
    let mut px = Vec::with_capacity(w * h);
    for p in buf[..w * h * ch].chunks(ch) {
        let a = if ch == 4 { p[3] } else { 255 };
        let a = if a > 127 { 255u32 } else { 0 };
        px.push(a << 24 | (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32);
    }
    (w, h, px)
}

impl Scene {
    pub fn load(dir: &str) -> Scene {
        let meta: serde_json::Value =
            serde_json::from_str(&crate::art::read_string(dir, "layers.json").expect("layers.json")).expect("layers.json: не JSON");
        let w = meta["size"][0].as_u64().unwrap() as usize;
        let h = meta["size"][1].as_u64().unwrap() as usize;
        let mut layers = Vec::new();
        for e in meta["order"].as_array().expect("order") {
            let name = e["name"].as_str().unwrap().to_string();
            let (lw, lh, px) = load_png(dir, &format!("{name}.png"));
            assert!(lw == w && lh == h, "слой {name}: {lw}×{lh}, а холст {w}×{h}");
            let bbox = bbox_of(&px, w, h);
            let px = crop(&px, w, bbox);
            layers.push(Layer { name, px, bbox, poses: Vec::new() });
        }
        // позы: позы/позы.json — {кадр: [слои]}, картинки позы/<слой>@<кадр>.png
        if let Some(txt) = crate::art::read_string(dir, "позы/позы.json") {
            let idx: serde_json::Value = serde_json::from_str(&txt).expect("позы.json");
            for (frame, ls) in idx.as_object().unwrap() {
                for l in ls.as_array().unwrap() {
                    let l = l.as_str().unwrap();
                    let Some(layer) = layers.iter_mut().find(|x| x.name == l) else { continue };
                    let (_, _, px) = load_png(dir, &format!("позы/{l}@{frame}.png"));
                    let bb = bbox_of(&px, w, h);
                    layer.poses.push((frame.clone(), crop(&px, w, bb), bb));
                }
            }
        }
        Scene { w, h, layers }
    }

    pub fn index(&self, name: &str) -> usize {
        self.layers.iter().position(|l| l.name == name).unwrap_or_else(|| panic!("нет слоя {name}"))
    }
}

/// Как рисовать слой в этом кадре.
#[derive(Clone, Copy)]
pub struct Draw {
    pub dx: i32,
    pub dy: i32,
    pub visible: bool,
    /// подъём по шву дыхания: всё, что выше шва в своём столбце, поднимается на lift
    /// клеток, а сама строка шва остаётся на месте (повторяется) — свитер «растягивается»
    pub lift: i32,
    /// номер позы слоя (кадра анимации) или None — обычный вид
    pub pose: Option<usize>,
    /// покачивание (растение): верх слоя сдвигается вбок на sway клеток, низ стоит
    pub sway: f32,
}

impl Default for Draw {
    fn default() -> Self {
        Draw { dx: 0, dy: 0, visible: true, lift: 0, pose: None, sway: 0.0 }
    }
}

/// Собрать кадр: слои по порядку, каждый со своим сдвигом и подъёмом по шву.
/// `seam[x]` — строка шва дыхания в столбце x (−1 — столбец не поднимается).
/// `owner` — кто сверху в каждой клетке (номер слоя), для кликов.
pub fn compose(sc: &Scene, draws: &[Draw], seam: &[i32], out: &mut [u32], owner: &mut [u16]) {
    out.fill(0);
    owner.fill(u16::MAX);
    let (w, h) = (sc.w as i32, sc.h as i32);
    for (i, l) in sc.layers.iter().enumerate() {
        let d = draws[i];
        if !d.visible {
            continue;
        }
        let (px, (x0, y0, x1, y1)) = match d.pose {
            Some(k) => (&l.poses[k].1, l.poses[k].2),
            None => (&l.px, l.bbox),
        };
        let height = (y1 - y0).max(1) as f32;
        for y in y0..y1 {
            let ty = y as i32 + d.dy;
            // покачивание: сдвиг строки пропорционален высоте над основанием
            let sdx = if d.sway != 0.0 { (d.sway * (y1 - y) as f32 / height).round() as i32 } else { 0 };
            if ty < 0 || ty >= h {
                continue;
            }
            let bw = x1 - x0;
            let row = &px[(y - y0) * bw..(y - y0 + 1) * bw];
            for x in x0..x1 {
                let p = row[x - x0];
                if p >> 24 == 0 {
                    continue;
                }
                let tx = x as i32 + d.dx + sdx;
                if tx < 0 || tx >= w {
                    continue;
                }
                let px = p & 0x00FF_FFFF;
                let s = if d.lift != 0 { seam[x] } else { -1 };
                if s >= 0 && (y as i32) <= s {
                    let uy = ty - d.lift;
                    if uy >= 0 {
                        out[uy as usize * sc.w + tx as usize] = px;
                        owner[uy as usize * sc.w + tx as usize] = i as u16;
                    }
                    if (y as i32) < s {
                        continue; // выше шва — только поднятая копия
                    }
                }
                out[ty as usize * sc.w + tx as usize] = px;
                owner[ty as usize * sc.w + tx as usize] = i as u16;
            }
        }
    }
}
