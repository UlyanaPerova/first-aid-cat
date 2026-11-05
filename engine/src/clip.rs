//! Буфер обмена. Linux — X11 без GTK; Windows — clipboard-win; macOS — pbcopy.
//! Linux: поток занимает выделение CLIPBOARD и отвечает тем, кто вставляет.
//! Поток спит в ожидании событий (нагрузки нет) и сам завершается, когда выделение забрал другой.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::*;
#[cfg(target_os = "linux")]
use x11rb::protocol::Event;
#[cfg(target_os = "linux")]
use x11rb::rust_connection::RustConnection;
#[cfg(target_os = "linux")]
use x11rb::wrapper::ConnectionExt as _;

#[cfg(target_os = "linux")]
fn serve(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = RustConnection::connect(None)?;
    let root = conn.setup().roots.get(screen_num).ok_or("нет экрана X11")?.root;
    let win = conn.generate_id()?;
    conn.create_window(0, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?;
    let atom = |n: &str| -> Result<u32, Box<dyn std::error::Error>> { Ok(conn.intern_atom(false, n.as_bytes())?.reply()?.atom) };
    let (clipboard, utf8, targets) = (atom("CLIPBOARD")?, atom("UTF8_STRING")?, atom("TARGETS")?);
    conn.set_selection_owner(win, clipboard, Time::CURRENT_TIME)?;
    conn.flush()?;
    if conn.get_selection_owner(clipboard)?.reply()?.owner != win {
        return Err("не удалось занять буфер обмена".into());
    }
    loop {
        match conn.wait_for_event()? {
            Event::SelectionRequest(req) => {
                let prop = if req.property == 0 { req.target } else { req.property };
                let mut ok = true;
                if req.target == targets {
                    conn.change_property32(PropMode::REPLACE, req.requestor, prop, AtomEnum::ATOM, &[targets, utf8, AtomEnum::STRING.into()])?;
                } else if req.target == utf8 {
                    conn.change_property8(PropMode::REPLACE, req.requestor, prop, utf8, text.as_bytes())?;
                } else if req.target == u32::from(AtomEnum::STRING) {
                    conn.change_property8(PropMode::REPLACE, req.requestor, prop, AtomEnum::STRING, text.as_bytes())?;
                } else {
                    ok = false;
                }
                let note = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: req.time,
                    requestor: req.requestor,
                    selection: req.selection,
                    target: req.target,
                    property: if ok { prop } else { 0 },
                };
                conn.send_event(false, req.requestor, EventMask::NO_EVENT, note)?;
                conn.flush()?;
            }
            Event::SelectionClear(_) => return Ok(()),
            _ => {}
        }
    }
}

/// Скопировать текст в буфер обмена (в фоновом потоке).
#[cfg(target_os = "linux")]
pub fn copy(text: String) {
    std::thread::spawn(move || match serve(&text) {
        Ok(()) => eprintln!("[окно] буфер обмена забрало другое приложение"),
        Err(e) => eprintln!("[окно] буфер обмена: {e}"),
    });
}

#[cfg(windows)]
pub fn copy(text: String) {
    if let Err(e) = clipboard_win::set_clipboard_string(&text) {
        eprintln!("[окно] буфер обмена: {e}");
    }
}

#[cfg(target_os = "macos")]
pub fn copy(text: String) {
    use std::io::Write;
    let r = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().and_then(|mut c| {
        c.stdin.take().map(|mut i| i.write_all(text.as_bytes())).transpose()?;
        c.wait().map(|_| ())
    });
    if let Err(e) = r {
        eprintln!("[окно] буфер обмена: {e}");
    }
}
