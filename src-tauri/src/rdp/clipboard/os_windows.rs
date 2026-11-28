//! Windows OS clipboard access: raw wire-format reads/writes, HDROP file
//! lists, and a hidden-window listener that broadcasts local changes.
//!
//! Data operations run on any worker thread (with retry while another app
//! holds the clipboard open). The listener window lives on its own pump
//! thread; clipboard reads never use delayed rendering, so there is no
//! `WM_RENDERFORMAT` interaction at all.

use std::{
    cell::RefCell,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Duration,
};

use windows::Win32::{
    Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM},
    System::{
        DataExchange::{
            AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
            IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSW,
    },
};

use super::{CliprdrEvent, LocalFile, LocalSnapshot, MAX_FILES_PER_COPY, Sinks};
use crate::rdp::clipboard::OsClipboard;

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
const CF_HDROP: u32 = 15;
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
const DROPFILES_SIZE: usize = 20;
const OPEN_ATTEMPTS: usize = 10;
const RETRY_DELAY: Duration = Duration::from_millis(100);

/// The listener window handle (set once by the pump thread).
static LISTENER_HWND: OnceLock<isize> = OnceLock::new();

pub(crate) struct WindowsOs;

impl std::fmt::Debug for WindowsOs {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WindowsOs")
    }
}

impl OsClipboard for WindowsOs {
    fn is_supported(&self) -> bool {
        true
    }

    fn read(&self) -> Result<LocalSnapshot, String> {
        with_clipboard(|| {
            let mut snapshot = LocalSnapshot::default();
            if available(CF_UNICODETEXT) {
                snapshot.text = Some(read_raw(CF_UNICODETEXT)?);
            }
            if available(CF_DIB) {
                snapshot.dib = Some(read_raw(CF_DIB)?);
            }
            if available(CF_HDROP) {
                // Dropped folders expand recursively: each file keeps a
                // backslash-separated prefix so the remote paste recreates
                // the directory structure.
                let mut files = Vec::new();
                for path in read_hdrop()? {
                    let metadata = match std::fs::metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(_) => continue,
                    };
                    if metadata.is_dir() {
                        let prefix = path.file_name().map_or_else(
                            || path.to_string_lossy().into_owned(),
                            |name| name.to_string_lossy().into_owned(),
                        );
                        walk_files(&path, &prefix, &mut files);
                    } else {
                        files.push(LocalFile {
                            name: file_name(&path),
                            relative_path: None,
                            path,
                            size: metadata.len(),
                        });
                    }
                    if files.len() >= MAX_FILES_PER_COPY {
                        break;
                    }
                }
                snapshot.files = files.into_iter().take(MAX_FILES_PER_COPY).collect();
            }
            Ok(snapshot)
        })
    }

    fn write(&self, items: &[(u32, Vec<u8>)], files: &[PathBuf]) -> Result<(), String> {
        with_clipboard(|| {
            unsafe { EmptyClipboard() }.map_err(|error| format!("EmptyClipboard failed: {error}"))?;
            for (format, data) in items {
                set_raw(*format, data)?;
            }
            if !files.is_empty() {
                set_raw(CF_HDROP, &hdrop_wire(files))?;
            }
            Ok(())
        })
    }
}

// ---------- Raw clipboard data helpers ----------

fn with_clipboard<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let hwnd = LISTENER_HWND.get().map(|handle| HWND(*handle as *mut core::ffi::c_void));
    let mut last_error = "clipboard unavailable".to_owned();
    for _ in 0..OPEN_ATTEMPTS {
        if unsafe { OpenClipboard(hwnd.as_ref().map(|window| *window).map_or(None, Some)) }.is_ok() {
            let result = operation();
            unsafe {
                let _ = CloseClipboard();
            }
            return result;
        }
        last_error = "OpenClipboard failed (clipboard busy)".into();
        std::thread::sleep(RETRY_DELAY);
    }
    Err(last_error)
}

fn available(format: u32) -> bool {
    unsafe { IsClipboardFormatAvailable(format).is_ok() }
}

/// Reads the raw HGLOBAL bytes for one clipboard format. For CF_UNICODETEXT /
/// CF_DIB / CF_HDROP these bytes are exactly the wire format we exchange.
fn read_raw(format: u32) -> Result<Vec<u8>, String> {
    let handle = unsafe { GetClipboardData(format) }
        .map_err(|error| format!("GetClipboardData({format}) failed: {error}"))?;
    let global = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(global) };
    let locked = unsafe { GlobalLock(global) };
    if locked.is_null() {
        return Err(format!("GlobalLock({format}) failed"));
    }
    let bytes = unsafe { std::slice::from_raw_parts(locked.cast::<u8>(), size) }.to_vec();
    let _ = unsafe { GlobalUnlock(global) };
    Ok(bytes)
}

fn set_raw(format: u32, data: &[u8]) -> Result<(), String> {
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, data.len().max(1)) }
        .map_err(|error| format!("GlobalAlloc failed: {error}"))?;
    let locked = unsafe { GlobalLock(global) };
    if locked.is_null() {
        // The allocated handle leaks on this rare path; freeing it risks
        // use-after-free confusion with SetClipboardData ownership.
        return Err("GlobalLock failed while writing".into());
    }
    unsafe {
        locked.cast::<u8>().copy_from_nonoverlapping(data.as_ptr(), data.len());
        let _ = GlobalUnlock(global);
    }
    // SetClipboardData takes ownership of the handle on success.
    unsafe { SetClipboardData(format, Some(HANDLE(global.0))) }
        .map_err(|error| format!("SetClipboardData({format}) failed: {error}"))
        .map(|_| ())
}

/// Recursively collects files under `dir`, tagging each with the accumulated
/// backslash-separated prefix so a remote Explorer paste rebuilds the tree.
fn walk_files(dir: &std::path::Path, prefix: &str, out: &mut Vec<LocalFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES_PER_COPY {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() {
            walk_files(&path, &format!("{prefix}\\{name}"), out);
        } else if let Ok(metadata) = entry.metadata() {
            out.push(LocalFile {
                name,
                relative_path: Some(prefix.to_owned()),
                path,
                size: metadata.len(),
            });
        }
    }
}

/// Parses a DROPFILES/CF_HDROP payload (UTF-16 path list, double-null ended).
fn read_hdrop() -> Result<Vec<PathBuf>, String> {
    let raw = read_raw(CF_HDROP)?;
    if raw.len() < DROPFILES_SIZE + 2 {
        return Err("HDROP payload too small".into());
    }
    let start = u32::from_le_bytes(raw[0..4].try_into().expect("fixed size")) as usize;
    let body = raw.get(start..).ok_or("HDROP offset out of range")?;
    let mut paths = Vec::new();
    let mut current = String::new();
    for unit in body.chunks_exact(2) {
        let unit = u16::from_le_bytes(unit.try_into().expect("fixed size"));
        if unit == 0 {
            if current.is_empty() {
                break; // terminating double-null
            }
            paths.push(PathBuf::from(current));
            current = String::new();
        } else {
            current.push(char::from_u32(u32::from(unit)).unwrap_or('\u{FFFD}'));
        }
    }
    Ok(paths)
}

/// Builds a DROPFILES/CF_HDROP payload pointing at the staged files.
fn hdrop_wire(files: &[PathBuf]) -> Vec<u8> {
    let mut wide: Vec<u16> = Vec::new();
    for path in files {
        wide.extend(path.as_os_str().encode_wide().chain(std::iter::once(0)));
    }
    wide.push(0); // terminating double-null
    let mut wire = Vec::with_capacity(DROPFILES_SIZE + wide.len() * 2);
    wire.extend_from_slice(&(DROPFILES_SIZE as u32).to_le_bytes()); // pFiles
    wire.extend_from_slice(&[0; 8]); // pt
    wire.extend_from_slice(&0u32.to_le_bytes()); // fNC
    wire.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for unit in wide {
        wire.extend_from_slice(&unit.to_le_bytes());
    }
    wire
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into())
}

// ---------- Listener pump thread ----------

thread_local! {
    static PUMP_SINKS: RefCell<Option<Sinks>> = const { RefCell::new(None) };
}

/// Runs the listener message loop on the calling thread. Never returns under
/// normal operation.
pub(crate) fn run_clipboard_listener(sinks: Sinks) -> ! {
    use windows::core::PCWSTR;

    PUMP_SINKS.with(|cell| *cell.borrow_mut() = Some(sinks));

    let class_name: Vec<u16> = "AuuRemoteClipboardListener\0".encode_utf16().collect();
    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        eprintln!("cliprdr: RegisterClassW failed");
        park_forever();
    }
    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
    };
    let Ok(hwnd) = created else {
        eprintln!("cliprdr: listener window creation failed");
        park_forever();
    };
    if unsafe { AddClipboardFormatListener(hwnd) }.is_err() {
        eprintln!("cliprdr: AddClipboardFormatListener failed");
        park_forever();
    }
    let _ = LISTENER_HWND.set(hwnd.0 as isize);

    let mut message = MSG::default();
    loop {
        if unsafe { GetMessageW(&mut message, Some(hwnd), 0, 0) }.0 <= 0 {
            park_forever();
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        PUMP_SINKS.with(|cell| {
            if let Some(sinks) = cell.borrow().as_ref() {
                let sinks = sinks.lock().expect("clipboard sinks");
                if let Some((_, sink)) = sinks.last() {
                    let _ = sink.send(CliprdrEvent::OsUpdated);
                }
            }
        });
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn park_forever() -> ! {
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
