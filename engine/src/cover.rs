//! Закрыт ли виджет другими окнами (Linux, X11). С композитным менеджером (GNOME, KDE…) X-сервер считает
//! каждое окно видимым, и winit не присылает Occluded — виджет крутил бы оживление часами, хотя его не видно.
//! Поэтому спрашиваем у оконного менеджера сами: окна по порядку наложения (_NET_CLIENT_LIST_STACKING), на
//! текущем рабочем столе, не свёрнутые; если они целиком накрыли прямоугольник виджета — он спрятан.
#![cfg(all(unix, not(target_os = "macos")))]

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

pub struct Cover {
    conn: RustConnection,
    root: Window,
    stacking: Atom,
    state: Atom,
    hidden: Atom,
    desktop: Atom,
    current: Atom,
    wtype: Atom,
    normal_types: [Atom; 3],
    pid: Atom,
}

/// Прямоугольник (x, y, w, h) в точках экрана.
pub type R = (i32, i32, i32, i32);

impl Cover {
    pub fn new() -> Option<Cover> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atom = |n: &str| -> Option<Atom> { Some(conn.intern_atom(false, n.as_bytes()).ok()?.reply().ok()?.atom) };
        Some(Cover {
            stacking: atom("_NET_CLIENT_LIST_STACKING")?,
            state: atom("_NET_WM_STATE")?,
            hidden: atom("_NET_WM_STATE_HIDDEN")?,
            desktop: atom("_NET_WM_DESKTOP")?,
            current: atom("_NET_CURRENT_DESKTOP")?,
            wtype: atom("_NET_WM_WINDOW_TYPE")?,
            normal_types: [atom("_NET_WM_WINDOW_TYPE_NORMAL")?, atom("_NET_WM_WINDOW_TYPE_DIALOG")?, atom("_NET_WM_WINDOW_TYPE_UTILITY")?],
            pid: atom("_NET_WM_PID")?,
            root,
            conn,
        })
    }

    /// Свойство окна — числами (запрос уже отправлен, тут — ответ).
    fn nums(c: Option<x11rb::cookie::Cookie<'_, RustConnection, x11rb::protocol::xproto::GetPropertyReply>>) -> Vec<u32> {
        c.and_then(|c| c.reply().ok()).and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
    }

    fn prop(&self, w: Window, prop: Atom) -> Option<x11rb::cookie::Cookie<'_, RustConnection, x11rb::protocol::xproto::GetPropertyReply>> {
        self.conn.get_property(false, w, prop, AtomEnum::ANY, 0, 1024).ok()
    }

    /// Окна, что лежат поверх рабочего стола и видны сейчас: их прямоугольники. None — оконный менеджер не
    /// отвечает по EWMH (тогда о закрытости не судим). Запросы уходят пачкой, ответы читаются потом: два
    /// обмена с X-сервером на всю проверку, а не по шесть на каждое окно.
    fn covering(&self) -> Option<Vec<R>> {
        let (lc, cc) = (self.prop(self.root, self.stacking), self.prop(self.root, self.current));
        let (list, cur) = (Self::nums(lc), Self::nums(cc).first().copied());
        if list.is_empty() {
            return None;
        }
        let asks: Vec<_> = list
            .iter()
            .map(|&w| {
                (
                    self.prop(w, self.pid),
                    self.prop(w, self.state),
                    self.prop(w, self.desktop),
                    self.prop(w, self.wtype),
                    self.conn.get_geometry(w).ok(),
                    self.conn.translate_coordinates(w, self.root, 0, 0).ok(),
                )
            })
            .collect();
        let me = std::process::id();
        let mut rows = Vec::new();
        for (pid, state, desk, wtype, geo, pos) in asks {
            let (pid, state, desk, wtype) = (Self::nums(pid), Self::nums(state), Self::nums(desk), Self::nums(wtype));
            let geo = geo.and_then(|c| c.reply().ok());
            let pos = pos.and_then(|c| c.reply().ok());
            rows.push((pid.first() == Some(&me), state, desk.first().copied(), wtype, geo.zip(pos)));
        }
        // считаются только окна выше нашего (список — снизу вверх); своё не нашлось — все
        let from = rows.iter().rposition(|r| r.0).map_or(0, |i| i + 1);
        let mut out = Vec::new();
        for (mine, state, desk, types, gp) in rows.into_iter().skip(from) {
            if mine || state.contains(&self.hidden) {
                continue;
            }
            // окна с других рабочих столов не видны (0xFFFFFFFF — на всех)
            if let (Some(d), Some(c)) = (desk, cur) {
                if d != c && d != u32::MAX {
                    continue;
                }
            }
            // рабочий стол, панели, всплывающие подсказки — не «окна поверх»; без типа — обычное окно
            if !types.is_empty() && !types.iter().any(|t| self.normal_types.contains(t)) {
                continue;
            }
            let Some((g, t)) = gp else { continue };
            out.push((t.dst_x as i32, t.dst_y as i32, g.width as i32, g.height as i32));
        }
        Some(out)
    }

    /// Целиком ли закрыт прямоугольник `me` (точки экрана). None — не узнать.
    pub fn covered(&self, me: R) -> Option<bool> {
        Some(covered_by(me, &self.covering()?))
    }
}

/// Закрыт ли `me` прямоугольниками `ws` целиком: проверяются 9×9 точек по сетке (краешек в пару точек
/// между ними не в счёт — такой виджет всё равно не разглядеть).
fn covered_by(me: R, ws: &[R]) -> bool {
    let (x, y, w, h) = me;
    if w <= 0 || h <= 0 {
        return false;
    }
    (0..9).all(|i| {
        (0..9).all(|j| {
            let px = x + (w - 1) * i / 8;
            let py = y + (h - 1) * j / 8;
            ws.iter().any(|&(wx, wy, ww, wh)| px >= wx && px < wx + ww && py >= wy && py < wy + wh)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::covered_by;

    #[test]
    fn covered_only_when_whole() {
        let me = (100, 100, 200, 150);
        assert!(covered_by(me, &[(0, 0, 1920, 1080)]));
        assert!(!covered_by(me, &[(0, 0, 250, 1080)]));
        // два окна вместе закрывают целиком
        assert!(covered_by(me, &[(0, 0, 200, 1080), (200, 0, 400, 1080)]));
        assert!(!covered_by(me, &[]));
    }
}
