use std::{
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

#[cfg(test)]
mod tests;
mod transport;

#[derive(Default)]
pub(crate) struct Sessions {
    next_id: AtomicU32,
    active: Mutex<Option<Session>>,
}

struct Session {
    id: u32,
    view: Arc<Mutex<View>>,
    input: mpsc::Sender<Vec<Input>>,
    worker: JoinHandle<()>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[derive(Default)]
struct View {
    established: bool,
    frame: Option<Arc<Frame>>,
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum Status {
    Connecting,
    WaitingForFrame,
    Ready,
    Ended { message: String },
}

impl View {
    fn status(&self) -> Status {
        if let Some(message) = &self.error {
            Status::Ended {
                message: message.clone(),
            }
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

#[derive(Deserialize)]
pub(crate) struct Connection {
    #[serde(flatten)]
    endpoint: Endpoint,
    password: String,
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
    pub(crate) fn validate(&mut self) -> Result<Option<[u8; 32]>, String> {
        let host = self.host.trim();
        self.host = host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(host)
            .to_owned();
        if self.port == 0 || !valid_host(&self.host) {
            return Err("请输入有效的主机名或 IP，端口需单独填写。".into());
        }
        self.username = self.username.trim().to_owned();
        if self.username.is_empty() {
            return Err("请输入远端用户名。".into());
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
}

impl Sessions {
    fn get<R>(
        &self,
        id: u32,
        read: impl FnOnce(&Session) -> Result<R, String>,
    ) -> Result<R, String> {
        let active = self.active.lock().expect("session lock");
        let session = active
            .as_ref()
            .filter(|session| session.id == id)
            .ok_or("此会话已关闭。")?;
        read(session)
    }

    fn frame(&self, id: u32) -> Result<Option<Arc<Frame>>, String> {
        self.get(id, |session| {
            let view = session.view.lock().expect("view lock");
            if let Some(error) = &view.error {
                return Err(error.clone());
            }
            if session.worker.inner().is_finished() {
                return Err("RDP 会话意外结束。".into());
            }
            Ok(view.frame.clone())
        })
    }

    fn disconnect(&self, id: u32) {
        let mut active = self.active.lock().expect("session lock");
        if active.as_ref().is_some_and(|session| session.id == id) {
            active.take();
        }
    }
}

#[tauri::command]
pub(crate) fn rdp_connect(
    sessions: State<'_, Sessions>,
    connection: Connection,
) -> Result<u32, String> {
    start_session(&sessions, connection)
}

fn start_session(sessions: &Sessions, mut connection: Connection) -> Result<u32, String> {
    let fingerprint = connection.endpoint.validate()?;
    if connection.password.is_empty() {
        return Err("请输入远端密码。".into());
    }
    let mut active = sessions.active.lock().expect("session lock");
    if active.is_some() {
        return Err("请先断开当前会话。".into());
    }
    let id = sessions
        .next_id
        .load(Ordering::Relaxed)
        .checked_add(1)
        .ok_or("请重启应用后重新连接。")?;
    sessions.next_id.store(id, Ordering::Relaxed);
    let view = Arc::new(Mutex::new(View::default()));
    let (input, receiver) = mpsc::channel(32);
    let worker_view = view.clone();
    let worker = tauri::async_runtime::spawn(async move {
        let result = transport::run(connection, fingerprint, receiver, &worker_view).await;
        let mut view = worker_view.lock().expect("view lock");
        view.frame = None;
        view.error = Some(match result {
            Ok(()) => "远程会话已断开。".to_owned(),
            Err(error) => format!("{error:#}"),
        });
    });
    *active = Some(Session {
        id,
        view,
        input,
        worker,
    });
    Ok(id)
}

#[tauri::command]
pub(crate) fn rdp_status(sessions: State<'_, Sessions>, id: u32) -> Result<Status, String> {
    sessions.get(id, |session| {
        let view = session.view.lock().expect("view lock");
        let status = view.status();
        if !matches!(status, Status::Ended { .. }) && session.worker.inner().is_finished() {
            return Ok(Status::Ended {
                message: "RDP 会话意外结束。".into(),
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
) -> Result<Response, String> {
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
) -> Result<(), String> {
    if events.len() > 64
        || events
            .iter()
            .any(|event| matches!(event, Input::Text { text } if text.len() > 256))
    {
        return Err("输入批次过大。".into());
    }
    sessions.get(id, |session| {
        if session.view.lock().expect("view lock").frame.is_none() {
            return Err("会话尚无可操作画面。".into());
        }
        session
            .input
            .try_send(events)
            .map_err(|_| "会话无法接收输入。".to_owned())
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
) -> Result<bool, String> {
    let frame = sessions.frame(id)?.ok_or("尚未收到远端画面。")?;
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
        let path = path.into_path().map_err(|error| error.to_string())?;
        let bytes = frame.png().map_err(|error| error.to_string())?;
        std::fs::write(path, bytes).map_err(|error| format!("无法保存截图。{error}"))?;
        Ok(true)
    })
    .await
    .map_err(|error| error.to_string())?
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

fn parse_fingerprint(value: &str) -> Result<Option<[u8; 32]>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let hex: String = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ':')
        .collect();
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("请输入 64 位十六进制 SHA-256 证书指纹。".into());
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
