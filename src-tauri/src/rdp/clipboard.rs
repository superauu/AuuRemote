//! OS clipboard bridge. One Win32 message-pump thread owns the OS clipboard
//! listener for the whole process; the most recently opened RDP session worker
//! consumes `ClipboardMessage`s through an unbounded channel.

use std::{
    sync::{Arc, Mutex, Once, OnceLock},
    thread,
};

use ironrdp::cliprdr::backend::{CliprdrBackendFactory, ClipboardMessage, ClipboardMessageProxy};
use tokio::sync::mpsc;

#[cfg(windows)]
use ironrdp_cliprdr_native::WinClipboard;

/// Registry of per-session clipboard channels; every entry lives until its
/// session's `Session` is dropped.
pub(crate) type Sinks = Arc<Mutex<Vec<(u32, mpsc::UnboundedSender<ClipboardMessage>)>>>;

/// Per-connection backend factory. The trait object only carries `Send`, so it
/// is shared behind a mutex.
type Factory = Box<dyn CliprdrBackendFactory + Send>;
pub(crate) type SharedCliprdrFactory = Arc<Mutex<Factory>>;

/// Process-wide clipboard state: the OS backend factory (when available) plus
/// the sink registry that session workers register against.
#[derive(Clone, Default)]
pub(crate) struct ClipboardBridge {
    pub(crate) sinks: Sinks,
    factory: Option<SharedCliprdrFactory>,
}

impl ClipboardBridge {
    /// Spawns the clipboard pump thread (once per process) and hands out a
    /// bridge whose factory builds per-connection backends.
    #[must_use]
    pub(crate) fn with_pump() -> Self {
        let sinks: Sinks = Arc::default();
        let factory = start_pump(sinks.clone());
        Self { sinks, factory }
    }

    /// Per-session channel. The registered sink lives until the session's
    /// `Session` is dropped.
    pub(crate) fn link(&self, id: u32) -> ClipboardLink {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sinks
            .lock()
            .expect("clipboard sinks")
            .push((id, tx));
        ClipboardLink {
            factory: self.factory.clone(),
            rx,
        }
    }
}

impl std::fmt::Debug for ClipboardBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardBridge")
            .field("clipboard_sync", &self.factory.is_some())
            .finish()
    }
}

pub(crate) struct ClipboardLink {
    pub(crate) factory: Option<SharedCliprdrFactory>,
    pub(crate) rx: mpsc::UnboundedReceiver<ClipboardMessage>,
}

impl ClipboardLink {
    /// Direct constructor for tests and alternative backends; skips the sink
    /// registry, so the caller keeps the producer side alive.
    #[allow(dead_code)] // production uses ClipboardBridge::link; tests use raw
    pub(crate) fn raw(factory: Option<SharedCliprdrFactory>, rx: mpsc::UnboundedReceiver<ClipboardMessage>) -> Self {
        Self { factory, rx }
    }
}

#[derive(Debug)]
struct PumpProxy(Sinks);

impl ClipboardMessageProxy for PumpProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        // ClipboardMessage is not Clone, so messages go to the most recently
        // opened session — the tab the user is most likely working in.
        let sinks = self.0.lock().expect("clipboard sinks");
        match sinks.last() {
            Some((_, sink)) if sink.send(message).is_ok() => {}
            _ => eprintln!("cliprdr: dropped clipboard message with no active session"),
        }
    }
}

/// Starts the process-wide clipboard pump thread (once). The thread owns the
/// `WinClipboard` instance — which is deliberately `!Send` — and pumps its
/// hidden window's messages for the lifetime of the app, keeping the up-to-10s
/// `WM_RENDERFORMAT` wait off the UI thread. Returns the shared backend
/// factory, or `None` when the OS clipboard is unavailable.
fn start_pump(sinks: Sinks) -> Option<SharedCliprdrFactory> {
    #[cfg(windows)]
    {
        static FACTORY: OnceLock<Option<SharedCliprdrFactory>> = OnceLock::new();
        static START: Once = Once::new();
        START.call_once(|| {
            let (ready_tx, ready_rx) = std::sync::mpsc::channel();
            let spawned = thread::Builder::new()
                .name("auuremote-cliprdr".into())
                .spawn(move || match WinClipboard::new(PumpProxy(sinks)) {
                    Ok(clipboard) => {
                        let _ = ready_tx.send(Some(clipboard.backend_factory()));
                        // Blocks on the pump thread's own message queue for the
                        // rest of the process lifetime; `WinClipboard` drops on
                        // this thread when the queue dies.
                        pump_messages();
                    }
                    Err(error) => {
                        eprintln!("cliprdr: OS clipboard unavailable ({error})");
                        let _ = ready_tx.send(None);
                    }
                });
            if spawned.is_err() {
                let _ = FACTORY.set(None);
                return;
            }
            // The first caller waits for the pump to come up (fast); later
            // callers read the memoized result.
            let factory = ready_rx
                .recv()
                .ok()
                .flatten()
                .map(|factory| Arc::new(Mutex::new(factory)));
            let _ = FACTORY.set(factory);
        });
        FACTORY.get().cloned().flatten()
    }
    #[cfg(not(windows))]
    {
        let _ = sinks;
        None
    }
}

/// Standard Win32 message pump for the clipboard monitor window. Only returns
/// (ending the thread) when the message queue dies.
#[cfg(windows)]
fn pump_messages() {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, MSG, TranslateMessage};

    loop {
        let mut message = MSG::default();
        // 0 means WM_QUIT, -1 means error; either way the pump is done.
        if unsafe { GetMessageW(&mut message, None, 0, 0) }.0 <= 0 {
            return;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
