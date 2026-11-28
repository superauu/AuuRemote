use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State, async_runtime::JoinHandle, ipc::Response};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::mpsc;

use crate::errors::{ApiError, ErrorCode};
use crate::trust;

#[cfg(test)]
mod tests;
pub(crate) mod clipboard;
mod transport;

pub(crate) struct Sessions {
    next_id: AtomicU32,
    active: Mutex<HashMap<u32, Session>>,
    clipboard: clipboard::ClipboardBridge,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            next_id: AtomicU32::new(0),
            active: Mutex::new(HashMap::new()),
            clipboard: clipboard::ClipboardBridge::default(),
        }
    }
}

impl Sessions {
    /// Production constructor: spawns the OS clipboard listener thread so
    /// every session syncs with the local clipboard. `staging_root` receives
    /// files downloaded from remote sessions.
    pub(crate) fn with_clipboard_pump(
        os: Arc<dyn clipboard::OsClipboard + Send + Sync>,
        staging_root: std::path::PathBuf,
    ) -> Self {
        Self {
            clipboard: clipboard::ClipboardBridge::with_pump(os, staging_root),
            ..Self::default()
        }
    }
}

struct Session {
    id: u32,
    sinks: clipboard::Sinks,
    view: Arc<Mutex<View>>,
    input: mpsc::Sender<Vec<Input>>,
    worker: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.worker.abort();
        self.sinks
            .lock()
            .expect("clipboard sinks")
            .retain(|(id, _)| *id != self.id);
    }
}

#[derive(Default)]
struct View {
    established: bool,
    transfer: clipboard::TransferHandle,
    /** A live session dropped and the worker is retrying the connection. */
    reconnecting: bool,
    frame: Option<Arc<Frame>>,
    error: Option<ApiError>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum Status {
    Connecting,
    WaitingForFrame,
    Reconnecting,
    Ready,
    Ended { code: ErrorCode, detail: String },
}

impl View {
    fn status(&self) -> Status {
        if let Some(error) = &self.error {
            Status::Ended {
                code: error.code,
                detail: error.detail.clone(),
            }
        } else if self.reconnecting {
            Status::Reconnecting
        } else if self.frame.is_some() {
            Status::Ready
        } else if self.established {
            Status::WaitingForFrame
        } else {
            Status::Connecting
        }
    }
}

struct Frame {
    revision: u32,
    width: u16,
    height: u16,
    // Little-endian revision, width, height, then native-size RGBA pixels.
    wire: Vec<u8>,
}

impl Frame {
    fn png(&self) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width.into(), self.height.into());
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&self.wire[8..])?;
        writer.finish()?;
        Ok(bytes)
    }
}

#[derive(Clone, Deserialize)]
pub(crate) struct Connection {
    #[serde(flatten)]
    endpoint: Endpoint,
    password: String,
    #[serde(default)]
    width: u16,
    #[serde(default)]
    height: u16,
}

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Endpoint {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    #[serde(default)]
    pub(crate) fingerprint: String,
}

impl Endpoint {
    pub(crate) fn validate(&mut self) -> Result<Option<[u8; 32]>, ApiError> {
        let host = self.host.trim();
        self.host = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host)
            .to_owned();
        if self.port == 0 || !valid_host(&self.host) {
            return Err(ApiError::new(
                ErrorCode::InvalidEndpoint,
                "请输入有效的主机名或 IP，端口需单独填写。",
            ));
        }
        self.username = self.username.trim().to_owned();
        if self.username.is_empty() {
            return Err(ApiError::new(
                ErrorCode::InvalidUsername,
                "请输入远端用户名。",
            ));
        }
        let fingerprint = parse_fingerprint(&self.fingerprint)?;
        self.fingerprint = fingerprint.map_or_else(String::new, |bytes| format_fingerprint(&bytes));
        Ok(fingerprint)
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum Input {
    Move {
        x: u16,
        y: u16,
    },
    Button {
        x: u16,
        y: u16,
        button: u8,
        pressed: bool,
    },
    Wheel {
        x: u16,
        y: u16,
        units: i16,
        vertical: bool,
    },
    Key {
        scan_code: u16,
        pressed: bool,
    },
    Text {
        text: String,
    },
    RemoteRun,
    ReleaseAll,
    /// Requests a new desktop size over the Display Control channel; the
    /// server answers with a Deactivation-Reactivation Sequence.
    Resize {
        width: u16,
        height: u16,
    },
}

impl Sessions {
    fn get<R>(
        &self,
        id: u32,
        read: impl FnOnce(&Session) -> Result<R, ApiError>,
    ) -> Result<R, ApiError> {
        let active = self.active.lock().expect("session lock");
        let session = active
            .get(&id)
            .ok_or_else(|| ApiError::new(ErrorCode::SessionClosed, "此会话已关闭。"))?;
        read(session)
    }

    fn frame(&self, id: u32) -> Result<Option<Arc<Frame>>, ApiError> {
        self.get(id, |session| {
            let view = session.view.lock().expect("view lock");
            if let Some(error) = &view.error {
                return Err(error.clone());
            }
            if session.worker.inner().is_finished() {
                return Err(ApiError::new(
                    ErrorCode::SessionClosed,
                    "RDP 会话意外结束。",
                ));
            }
            Ok(view.frame.clone())
        })
    }

    fn disconnect(&self, id: u32) {
        self.active.lock().expect("session lock").remove(&id);
    }
}

#[tauri::command]
pub(crate) fn rdp_connect(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    connection: Connection,
) -> Result<u32, ApiError> {
    let directory = crate::store::config_directory(&app)?;
    // Small JSON file; reading it inline keeps the command synchronous.
    let trusted = trust::load(&directory)?;
    start_session(
        &sessions,
        connection,
        |id| sessions.clipboard.link(id),
        trusted,
    )
}

/// TOFU: remembers the certificate fingerprint the user accepted for a host,
/// so later connections skip the manual verification prompt.
#[tauri::command]
pub(crate) async fn trust_host(
    app: AppHandle,
    host: String,
    fingerprint: String,
) -> Result<(), ApiError> {
    let host = host.trim().to_owned();
    if !valid_host(&host) {
        return Err(ApiError::new(
            ErrorCode::InvalidEndpoint,
            "请输入有效的主机名或 IP。",
        ));
    }
    let digest = parse_fingerprint(&fingerprint)?;
    let fingerprint = format_fingerprint(&digest.ok_or_else(|| {
        ApiError::new(
            ErrorCode::InvalidFingerprint,
            "请输入 64 位十六进制 SHA-256 证书指纹。",
        )
    })?);
    let directory = crate::store::config_directory(&app)?;
    tauri::async_runtime::spawn_blocking(move || trust::trust(&directory, host, fingerprint))
        .await
        .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "信任存储任务失败"))?
}

fn start_session(
    sessions: &Sessions,
    mut connection: Connection,
    make_link: impl FnOnce(u32) -> clipboard::ClipboardLink,
    trusted: trust::TrustMap,
) -> Result<u32, ApiError> {
    let fingerprint = connection.endpoint.validate()?;
    if connection.password.is_empty() {
        return Err(ApiError::new(
            ErrorCode::PasswordRequired,
            "请输入远端密码。",
        ));
    }
    let mut active = sessions.active.lock().expect("session lock");
    let id = sessions.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let clipboard = make_link(id);
    let view = Arc::new(Mutex::new(View::default()));
    let (input, receiver) = mpsc::channel(32);
    let worker_view = view.clone();
    let worker = tauri::async_runtime::spawn(async move {
        let result = transport::run(
            connection,
            fingerprint,
            receiver,
            clipboard,
            trusted,
            &worker_view,
        )
        .await;
        let mut view = worker_view.lock().expect("view lock");
        view.frame = None;
        view.error = Some(match result {
            Ok(()) => ApiError::new(ErrorCode::ConnectionFailed, "远程会话已断开。"),
            Err(error) => error,
        });
    });
    active.insert(
        id,
        Session {
            id,
            sinks: sessions.clipboard.sinks.clone(),
            view,
            input,
            worker,
        },
    );
    Ok(id)
}

/// Live remote-file download progress for the session, if one is running.
#[tauri::command]
pub(crate) fn rdp_transfer(
    sessions: State<'_, Sessions>,
    id: u32,
) -> Result<Option<clipboard::TransferProgress>, ApiError> {
    sessions.get(id, |session| {
        Ok(session
            .view
            .lock()
            .expect("view lock")
            .transfer
            .lock()
            .expect("transfer lock")
            .clone())
    })
}

#[tauri::command]
pub(crate) fn rdp_status(sessions: State<'_, Sessions>, id: u32) -> Result<Status, ApiError> {
    sessions.get(id, |session| {
        let view = session.view.lock().expect("view lock");
        let status = view.status();
        if !matches!(status, Status::Ended { .. }) && session.worker.inner().is_finished() {
            return Ok(Status::Ended {
                code: ErrorCode::SessionClosed,
                detail: "RDP 会话意外结束。".into(),
            });
        }
        Ok(status)
    })
}

#[tauri::command]
pub(crate) fn rdp_frame(
    sessions: State<'_, Sessions>,
    id: u32,
    after: u32,
) -> Result<Response, ApiError> {
    let wire = sessions
        .frame(id)?
        .filter(|frame| frame.revision != after)
        .map_or_else(Vec::new, |frame| frame.wire.clone());
    Ok(Response::new(wire))
}

#[tauri::command]
pub(crate) fn rdp_input(
    sessions: State<'_, Sessions>,
    id: u32,
    events: Vec<Input>,
) -> Result<(), ApiError> {
    if events.len() > 64
        || events
            .iter()
            .any(|event| matches!(event, Input::Text { text } if text.len() > 256))
    {
        return Err(ApiError::new(ErrorCode::SendTooLarge, "输入批次过大。"));
    }
    sessions.get(id, |session| {
        if session.view.lock().expect("view lock").frame.is_none() {
            return Err(ApiError::new(
                ErrorCode::NoFrame,
                "会话尚无可操作画面。",
            ));
        }
        session
            .input
            .try_send(events)
            .map_err(|_| ApiError::new(ErrorCode::SessionClosed, "会话无法接收输入。"))
    })
}

#[tauri::command]
pub(crate) fn rdp_disconnect(sessions: State<'_, Sessions>, id: u32) {
    sessions.disconnect(id);
}

#[tauri::command]
pub(crate) async fn rdp_screenshot(
    app: AppHandle,
    sessions: State<'_, Sessions>,
    id: u32,
) -> Result<bool, ApiError> {
    let frame = sessions
        .frame(id)?
        .ok_or_else(|| ApiError::new(ErrorCode::NoFrame, "尚未收到远端画面。"))?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .add_filter("PNG", &["png"])
            .set_file_name("rdp-screenshot.png")
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = path
            .into_path()
            .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法保存截图"))?;
        let bytes = frame
            .png()
            .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法保存截图"))?;
        std::fs::write(path, bytes)
            .map_err(|error| ApiError::chain(ErrorCode::IoError, &error, "无法保存截图"))?;
        Ok(true)
    })
    .await
    .map_err(|error| ApiError::chain(ErrorCode::Internal, &error, "截图任务失败"))?
}

fn valid_host(host: &str) -> bool {
    if host.parse::<IpAddr>().is_ok() {
        return true;
    }
    let name = host.strip_suffix('.').unwrap_or(host);
    name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn parse_fingerprint(value: &str) -> Result<Option<[u8; 32]>, ApiError> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let hex: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ':')
        .collect();
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::new(
            ErrorCode::InvalidFingerprint,
            "请输入 64 位十六进制 SHA-256 证书指纹。",
        ));
    }
    let mut result = [0; 32];
    for (pos, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[pos * 2..pos * 2 + 2], 16).expect("validated hex");
    }
    Ok(Some(result))
}

fn format_fingerprint(bytes: &[u8; 32]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
