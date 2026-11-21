//! Clipboard bridge with two-way text/image/file sync.
//!
//! Architecture:
//! - One Win32 message-pump thread per process listens for local clipboard
//!   changes (`WM_CLIPBOARDUPDATE`) and broadcasts [`CliprdrEvent::OsUpdated`].
//! - The worker drives an *eager* sync: remote copies are fetched immediately
//!   and written to the OS clipboard; local copies are announced to the remote
//!   as soon as they happen. Data transfers to/from the OS clipboard run on
//!   worker threads with retry, never delayed rendering.
//! - [`SessionBackend`] implements the protocol side (`CliprdrBackend`):
//!   announcing formats, serving text/image/file data from the local snapshot,
//!   and streaming file ranges for uploads. `ironrdp-cliprdr` handles FDW
//!   encoding/decoding, locking and remote file-name sanitization.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, Once},
    thread,
};

use ironrdp::cliprdr::{
    backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage, ClipboardMessageProxy},
    pdu::{
        ClipboardFormat, ClipboardFormatId, ClipboardFormatName, FileDescriptor, FormatDataRequest,
        FormatDataResponse, FileContentsRequest, FileContentsResponse, LockDataId,
        ClipboardGeneralCapabilityFlags, PackedFileList,
    },
};
use tokio::sync::mpsc;

mod os_windows;
mod worker;

#[cfg(windows)]
pub(crate) use os_windows::WindowsOs;
pub(crate) use worker::{STAGING_DIR_NAME, Worker};

/// OS clipboard operations (raw wire formats + file lists). Implemented by
/// the Windows backend; tests substitute an in-memory backend.
pub(crate) trait OsClipboard: std::fmt::Debug {
    /// Whether this OS actually provides a clipboard.
    fn is_supported(&self) -> bool;

    /// Reads the current snapshot (text/image wire bytes + file list).
    fn read(&self) -> Result<LocalSnapshot, String>;

    /// Replaces the OS clipboard contents; `files` become a CF_HDROP.
    fn write(&self, items: &[(u32, Vec<u8>)], files: &[PathBuf]) -> Result<(), String>;
}

#[cfg(not(windows))]
#[derive(Debug, Default)]
pub(crate) struct UnsupportedOs;

#[cfg(not(windows))]
impl OsClipboard for UnsupportedOs {
    fn is_supported(&self) -> bool {
        false
    }
    fn read(&self) -> Result<LocalSnapshot, String> {
        Err("clipboard unsupported on this platform".into())
    }
    fn write(&self, _: &[(u32, Vec<u8>)], _: &[PathBuf]) -> Result<(), String> {
        Err("clipboard unsupported on this platform".into())
    }
}

/// Registry of per-session clipboard event channels; every entry lives until
/// its session's `Session` is dropped.
pub(crate) type Sinks = Arc<Mutex<Vec<(u32, mpsc::UnboundedSender<CliprdrEvent>)>>>;

/// Per-connection backend factory. The trait object only carries `Send`, so it
/// is shared behind a mutex.
type Factory = Box<dyn CliprdrBackendFactory + Send>;
pub(crate) type SharedCliprdrFactory = Arc<Mutex<Factory>>;

/// Events consumed by a session worker (see `transport::run`).
pub(crate) enum CliprdrEvent {
    /// The OS clipboard changed; re-read the local snapshot and re-announce.
    OsUpdated,
    /// The CLIPRDR channel finished its handshake and is Ready.
    Ready,
    /// The remote announced a copy with these formats.
    RemoteCopy(Vec<ClipboardFormat>),
    /// A protocol action for the CLIPRDR channel (announce/paste/submit).
    Protocol(ClipboardMessage),
    /// The remote answered a paste with format data (`None` = error response).
    PasteData(Option<Vec<u8>>),
    /// The library parsed a remote FileGroupDescriptorW announcement.
    RemoteFileList(Vec<FileDescriptor>, Option<u32>),
    /// The remote answered a file-contents request (`None` = error response).
    FileContentsResult {
        stream_id: u32,
        data: Option<Vec<u8>>,
    },
}

impl std::fmt::Debug for CliprdrEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OsUpdated => formatter.write_str("OsUpdated"),
            Self::Ready => formatter.write_str("Ready"),
            Self::RemoteCopy(formats) => {
                formatter.debug_tuple("RemoteCopy").field(&formats.len()).finish()
            }
            Self::Protocol(_) => formatter.write_str("Protocol(..)"),
            Self::PasteData(data) => formatter.debug_tuple("PasteData").field(data).finish(),
            Self::RemoteFileList(files, id) => formatter
                .debug_tuple("RemoteFileList")
                .field(&files.len())
                .field(id)
                .finish(),
            Self::FileContentsResult { stream_id, data } => formatter
                .debug_tuple("FileContentsResult")
                .field(stream_id)
                .field(data)
                .finish(),
        }
    }
}

/// FileGroupDescriptorW / Preferred DropEffect are Windows-registered formats
/// with fixed IDs by convention (MS-RDPECLIP 1.3.1.2).
pub(crate) const FDW_FORMAT_ID: u32 = 0xC0FE;
const DROP_EFFECT_FORMAT_ID: u32 = 0xC0FD;
// DropEffect payload: DROPEFFECT_COPY as a little-endian DWORD.
const DROP_EFFECT_COPY: [u8; 4] = 1u32.to_le_bytes();

/// A file on the local machine that is announced to the remote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
}

/// What the local OS clipboard currently holds (raw wire bytes + file paths).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LocalSnapshot {
    pub text: Option<Vec<u8>>, // CF_UNICODETEXT wire bytes (UTF-16LE + null)
    pub dib: Option<Vec<u8>>,  // CF_DIB wire bytes
    pub files: Vec<LocalFile>,
}

impl LocalSnapshot {
    fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    /// Formats for the handshake announcement: text/image only. FileGroup
    /// formats cannot be backed by local_file_list during Initialization and
    /// the real file announce goes through initiate_file_copy once Ready.
    fn handshake_formats(&self) -> Vec<ClipboardFormat> {
        let mut formats = Vec::new();
        if self.text.is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        }
        if self.dib.is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
        }
        formats
    }

    /// The format list announced to the remote for this snapshot.
    fn announce_formats(&self) -> Vec<ClipboardFormat> {
        let mut formats = Vec::new();
        if self.text.is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        }
        if self.dib.is_some() {
            formats.push(ClipboardFormat::new(ClipboardFormatId::CF_DIB));
        }
        if self.has_files() {
            formats.push(
                ClipboardFormat::new(ClipboardFormatId::new(FDW_FORMAT_ID))
                    .with_name(ClipboardFormatName::FILE_LIST),
            );
            formats.push(
                ClipboardFormat::new(ClipboardFormatId::new(DROP_EFFECT_FORMAT_ID))
                    .with_name(ClipboardFormatName::PREFERRED_DROP_EFFECT),
            );
        }
        formats
    }
}

/// Per-session clipboard state shared between the backend (protocol callbacks)
/// and the worker loop (sync state machine).
#[derive(Debug, Default)]
pub(crate) struct SessionState {
    snapshot: LocalSnapshot,
    /// Snapshots taken when the remote locks our file announcement, keyed by
    /// clipDataId, so later file-contents requests stay consistent.
    locked: std::collections::HashMap<u32, LocalSnapshot>,
}

impl SessionState {
    fn snapshot_for(&self, data_id: Option<u32>) -> &LocalSnapshot {
        match data_id.and_then(|id| self.locked.get(&id)) {
            Some(snapshot) => snapshot,
            None => &self.snapshot,
        }
    }
}

/// Serializes the snapshot's files into FileGroupDescriptorW wire bytes.
fn file_list_wire(files: &[LocalFile]) -> Vec<u8> {
    let descriptors: Vec<FileDescriptor> = files
        .iter()
        .map(|file| {
            FileDescriptor::new(file.name.clone())
                .with_attributes(ClipboardFileAttributes::ARCHIVE)
                .with_file_size(file.size)
        })
        .collect();
    let packed = PackedFileList { files: descriptors };
    // 4-byte cItems + one 592-byte FILEDESCRIPTORW per file (attributes +
    // timestamps + size + 260-char name + 14-char alternate name).
    const FILE_DESCRIPTOR_WIRE_SIZE: usize = 592;
    let mut wire = vec![0u8; 4 + packed.files.len() * FILE_DESCRIPTOR_WIRE_SIZE];
    let mut cursor = WriteCursor::new(&mut wire);
    packed
        .encode(&mut cursor)
        .expect("PackedFileList encode into sized buffer");
    wire
}

/// The protocol-side backend for one connection. All OS data access goes
/// through the shared snapshot (filled by the worker) — callbacks never touch
/// the OS clipboard directly.
#[derive(Debug)]
pub(crate) struct SessionBackend {
    events: mpsc::UnboundedSender<CliprdrEvent>,
    state: Arc<Mutex<SessionState>>,
}

impl_as_any!(SessionBackend);

impl ClipboardMessageProxy for SessionBackend {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        let _ = self.events.send(CliprdrEvent::Protocol(message));
    }
}

impl SessionBackend {
    fn emit(&self, event: CliprdrEvent) {
        let _ = self.events.send(event);
    }
}

impl CliprdrBackend for SessionBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
    }

    fn on_ready(&mut self) {
        // Channel Ready: announce (or re-announce) the local snapshot now,
        // including the file copy path that requires the Ready state.
        self.emit(CliprdrEvent::Ready);
    }

    fn on_request_format_list(&mut self) {
        // MonitorReady: complete the handshake with a first announcement.
        self.emit(CliprdrEvent::OsUpdated);
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        self.emit(CliprdrEvent::RemoteCopy(formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let state = self.state.lock().expect("clipboard state");
        let snapshot = state.snapshot_for(None);
        let response = if request.format.value() == ClipboardFormatId::CF_UNICODETEXT.value() {
            snapshot
                .text
                .as_ref()
                .map(|text| FormatDataResponse::new_data(text.clone()))
        } else if request.format.value() == ClipboardFormatId::CF_DIB.value() {
            snapshot
                .dib
                .as_ref()
                .map(|dib| FormatDataResponse::new_data(dib.clone()))
        } else if request.format.value() == FDW_FORMAT_ID {
            Some(FormatDataResponse::new_data(file_list_wire(&snapshot.files)))
        } else if request.format.value() == DROP_EFFECT_FORMAT_ID {
            Some(FormatDataResponse::new_data(DROP_EFFECT_COPY.to_vec()))
        } else {
            None
        };
        drop(state);
        let response = response.unwrap_or_else(FormatDataResponse::new_error);
        self.emit(CliprdrEvent::Protocol(ClipboardMessage::SendFormatData(response)));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        let data = if response.is_error() {
            None
        } else {
            Some(response.data().to_vec())
        };
        self.emit(CliprdrEvent::PasteData(data));
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.emit(CliprdrEvent::RemoteFileList(files.to_vec(), clip_data_id));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        // Serve a byte range of one announced file. Range reads are small
        // (chunks), so a synchronous read here is acceptable.
        let state = self.state.lock().expect("clipboard state");
        let snapshot = state.snapshot_for(request.data_id);
        let served = usize::try_from(request.index)
            .ok()
            .and_then(|index| snapshot.files.get(index))
            .filter(|_| {
                request.flags.contains(FileContentsFlags::RANGE)
                    && u64::from(request.requested_size) <= MAX_TRANSFER
            })
            .and_then(|file| {
                std::fs::File::open(&file.path)
                    .and_then(|mut handle| {
                        use std::io::{Read, Seek, SeekFrom};
                        handle.seek(SeekFrom::Start(request.position))?;
                        let mut buffer = vec![0u8; request.requested_size as usize];
                        handle.read_exact(&mut buffer)?;
                        Ok(buffer)
                    })
                    .ok()
            });
        drop(state);
        let response = match served {
            Some(data) => FileContentsResponse::new_data_response(request.stream_id, data),
            None => FileContentsResponse::new_error(request.stream_id),
        };
        self.emit(CliprdrEvent::Protocol(ClipboardMessage::SendFileContentsResponse(response)));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let data = if response.is_error() {
            None
        } else {
            Some(response.data().to_vec())
        };
        self.emit(CliprdrEvent::FileContentsResult {
            stream_id: response.stream_id(),
            data,
        });
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        let mut state = self.state.lock().expect("clipboard state");
        let snapshot = state.snapshot.clone();
        state.locked.insert(data_id.0, snapshot);
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        self.state.lock().expect("clipboard state").locked.remove(&data_id.0);
    }
}

use ironrdp::cliprdr::pdu::{ClipboardFileAttributes, FileContentsFlags};
use ironrdp::core::{Encode, WriteCursor, impl_as_any};

/// Per-session backend factory (one per link; builds the session backend).
#[derive(Debug)]
pub(crate) struct SessionFactory {
    pub(crate) events: mpsc::UnboundedSender<CliprdrEvent>,
    pub(crate) state: Arc<Mutex<SessionState>>,
}

impl CliprdrBackendFactory for SessionFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(SessionBackend {
            events: self.events.clone(),
            state: self.state.clone(),
        })
    }
}

/// Worker-facing per-session handle: the event channel plus everything the
/// sync state machine needs.
pub(crate) struct ClipboardLink {
    pub(crate) factory: Option<SharedCliprdrFactory>,
    pub(crate) rx: mpsc::UnboundedReceiver<CliprdrEvent>,
    pub(crate) os: Option<Arc<dyn OsClipboard + Send + Sync>>,
    pub(crate) staging_root: Option<PathBuf>,
    pub(crate) state: Arc<Mutex<SessionState>>,
}

impl ClipboardLink {
    /// A link with clipboard sync disabled (tests, unsupported platforms).
    #[allow(dead_code)] // production uses ClipboardBridge::link; tests use these
    pub(crate) fn unsupported() -> Self {
        Self {
            factory: None,
            rx: mpsc::unbounded_channel().1,
            os: None,
            staging_root: None,
            state: Arc::default(),
        }
    }

    /// Direct constructor for tests and alternative backends; skips the sink
    /// registry, so the caller keeps the producer side alive.
    #[allow(dead_code)]
    pub(crate) fn raw(
        factory: Option<SharedCliprdrFactory>,
        rx: mpsc::UnboundedReceiver<CliprdrEvent>,
        os: Option<Arc<dyn OsClipboard + Send + Sync>>,
        staging_root: Option<PathBuf>,
        state: Arc<Mutex<SessionState>>,
    ) -> Self {
        Self {
            factory,
            rx,
            os,
            staging_root,
            state,
        }
    }
}

/// Process-wide clipboard state: the OS layer (listener + data ops) and the
/// sink registry that session workers register against.
#[derive(Default)]
pub(crate) struct ClipboardBridge {
    pub(crate) sinks: Sinks,
    pub(crate) os: Option<Arc<dyn OsClipboard + Send + Sync>>,
    pub(crate) staging_root: Option<PathBuf>,
}

impl ClipboardBridge {
    /// Spawns the OS clipboard listener thread (once per process) and builds
    /// the bridge. `staging_root` receives files downloaded from the remote.
    #[must_use]
    pub(crate) fn with_pump(os: Arc<dyn OsClipboard + Send + Sync>, staging_root: PathBuf) -> Self {
        let sinks: Sinks = Arc::default();
        if os.as_ref().is_supported() {
            start_listener(sinks.clone());
        }
        Self {
            sinks,
            os: Some(os),
            staging_root: Some(staging_root),
        }
    }

    /// Per-session channel. The registered sink lives until the session's
    /// `Session` is dropped.
    pub(crate) fn link(&self, id: u32) -> ClipboardLink {
        let (tx, rx) = mpsc::unbounded_channel();
        self.sinks
            .lock()
            .expect("clipboard sinks")
            .push((id, tx.clone()));
        let state = Arc::new(Mutex::new(SessionState::default()));
        let factory = match self.os.as_ref().filter(|os| os.is_supported()) {
            Some(_) => Some(Arc::new(Mutex::new(
                Box::new(SessionFactory { events: tx, state: state.clone() }) as Factory
            ))),
            _ => None,
        };
        ClipboardLink {
            factory,
            rx,
            os: self.os.clone(),
            staging_root: self.staging_root.clone(),
            state,
        }
    }
}

impl std::fmt::Debug for ClipboardBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClipboardBridge")
            .field("clipboard_sync", &self.os.as_ref().is_some_and(|os| os.is_supported()))
            .finish()
    }
}

/// Per-file and per-copy transfer caps (bytes).
pub(crate) const MAX_TRANSFER: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) const MAX_FILES_PER_COPY: usize = 64;
pub(crate) const CHUNK_SIZE: u32 = 1024 * 1024;

/// Starts the OS clipboard listener thread (once). Broadcasts
/// [`CliprdrEvent::OsUpdated`] to the most recent session on local changes.
fn start_listener(sinks: Sinks) {
    #[cfg(windows)]
    {
        static START: Once = Once::new();
        START.call_once(|| {
            let spawned = thread::Builder::new()
                .name("auuremote-cliprdr".into())
                .spawn(move || os_windows::run_clipboard_listener(sinks));
            if spawned.is_err() {
                eprintln!("cliprdr: failed to spawn clipboard listener thread");
            }
        });
    }
    #[cfg(not(windows))]
    {
        let _ = sinks;
    }
}
