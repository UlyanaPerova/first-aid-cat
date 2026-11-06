//! Виды и размеры виджета, выбор в настройках, места щелчков и облачко с репликой кота.
//! Рисует виды widget_px.rs (всё по пикселям).

use crate::gfx::{Canvas, Fonts, Rect, BLACK};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Window,
    Monitor,
    Glass,
}

// («Тамагочи» убран по просьбе владельца: сохранённый в настройках «tama» откроется «Окном»)
pub const STYLES: [Style; 3] = [Style::Window, Style::Monitor, Style::Glass];

impl Style {
    pub fn id(self) -> &'static str {
        match self {
            Style::Window => "window",
            Style::Monitor => "monitor",
            Style::Glass => "glass",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Style::Window => "Окно",
            Style::Monitor => "Монитор",
            Style::Glass => "Стекло",
        }
    }
    pub fn from_id(s: &str) -> Style {
        STYLES.into_iter().find(|x| x.id() == s).unwrap_or(Style::Window)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Size {
    Small,
    Medium,
}

impl Size {
    pub fn id(self) -> &'static str {
        if self == Size::Small {
            "small"
        } else {
            "medium"
        }
    }
    pub fn from_id(s: &str) -> Size {
        if s == "small" {
            Size::Small
        } else {
            Size::Medium
        }
    }
}

/// Вид и размер из настроек (ui.json: widget_style, widget_size).
pub fn chosen() -> (Style, Size) {
    let v = crate::prefs::load();
    let style = Style::from_id(v.get("widget_style").and_then(|x| x.as_str()).unwrap_or("window"));
    let size = Size::from_id(v.get("widget_size").and_then(|x| x.as_str()).unwrap_or("medium"));
    (style, fit_size(style, size))
}

/// Есть ли у вида этот размер на этой системе: среднее «Стекло» — только на macOS (показ для WidgetKit),
/// в Linux и Windows «Стекло» всегда маленькое.
pub fn fit_size(style: Style, size: Size) -> Size {
    if style == Style::Glass && !cfg!(target_os = "macos") {
        Size::Small
    } else {
        size
    }
}

/// Оживление виджета включено (ui.json widget_anim, по умолчанию да; WIDGET_ANIM=0 — выключить).
pub fn anim_enabled() -> bool {
    std::env::var("WIDGET_ANIM").map_or(true, |v| v != "0") && crate::prefs::load().get("widget_anim").and_then(|x| x.as_bool()).unwrap_or(true)
}

/// Куда можно щёлкнуть: крестик, кот (тычок), список (открыть Аптечку).
#[derive(Clone, Copy, Default)]
pub struct Hits {
    pub close: Rect,
    pub cat: Rect,
    pub list: Rect,
}

/// Облачко с репликой кота (жёлтое, как подсказка) внизу прямоугольника a.
pub fn quip_box(c: &mut Canvas, fonts: &mut Fonts, k: i32, a: Rect, q: &str) {
    quip(c, fonts, k, a, q)
}

fn quip(c: &mut Canvas, fonts: &mut Fonts, k: i32, a: Rect, q: &str) {
    let px = (12 * k) as u16;
    let lines = fonts.wrap(q, a.w - 16 * k, px, false, 3);
    let bh = lines.len() as i32 * 15 * k + 8 * k;
    let b = Rect::new(a.x + 4 * k, a.bottom() - bh - 4 * k, a.w - 8 * k, bh);
    c.fill(b, 0xFFFFE1);
    c.bevel(b, BLACK, BLACK);
    for (i, l) in lines.iter().enumerate() {
        let base = fonts.center_base(b.y + 4 * k + i as i32 * 15 * k, 15 * k, px);
        fonts.draw(c, b.x + 6 * k, base, l, BLACK, px, false);
    }
}
