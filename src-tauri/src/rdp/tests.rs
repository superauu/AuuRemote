use std::{
    io::Cursor,
    num::{NonZeroU16, NonZeroUsize},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use ironrdp::cliprdr::{
    backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage},
    pdu::{
        ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId,
        ClipboardGeneralCapabilityFlags, FileContentsFlags, FileContentsRequest,
        FileContentsResponse, FileDescriptor, FormatDataRequest, FormatDataResponse, LockDataId,
    },
};
use ironrdp_server::{
    BitmapUpdate, CliprdrServerFactory, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent,
    PixelFormat, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
    ServerEvent, ServerEventSender,
};
use ironrdp::core::impl_as_any;
use rcgen::generate_simple_self_signed;
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, time};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

use super::clipboard::{ClipboardLink, LocalFile, LocalSnapshot};
use super::*;

struct Updates(mpsc::Receiver<DisplayUpdate>);

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        Ok(self.0.recv().await)
    }
}

struct Display(Option<Updates>);

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 320,
            height: 240,
        }
    }

    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(self.0.take().expect("one test connection")))
    }
}

enum Received {
    Keyboard(KeyboardEvent),
    Mouse(MouseEvent),
}

struct InputLog(mpsc::UnboundedSender<Received>);

impl RdpServerInputHandler for InputLog {
    fn keyboard(&mut self, event: KeyboardEvent) {
        let _ = self.0.send(Received::Keyboard(event));
    }

    fn mouse(&mut self, event: MouseEvent) {
        let _ = self.0.send(Received::Mouse(event));
    }
}

fn color_frame(pixel: [u8; 4]) -> DisplayUpdate {
    DisplayUpdate::Bitmap(BitmapUpdate {
        x: 0,
        y: 0,
        width: NonZeroU16::new(320).unwrap(),
        height: NonZeroU16::new(240).unwrap(),
        format: PixelFormat::RgbA32,
        data: pixel.repeat(320 * 240).into(),
        stride: NonZeroUsize::new(320 * 4).unwrap(),
    })
}

async fn wait_frame(view: &Mutex<View>, after: u32) -> Arc<Frame> {
    time::timeout(Duration::from_secs(8), async {
        loop {
            if let Some(frame) = view
                .lock()
                .unwrap()
                .frame
                .clone()
                .filter(|frame| frame.revision > after)
            {
                return frame;
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RDP frame arrived")
}

#[tokio::test]
async fn rdp_frames_png_and_input_share_one_tls_connection() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(verify_loopback())
        .await
}

async fn verify_loopback() -> anyhow::Result<()> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let identity = generate_simple_self_signed(vec!["localhost".to_owned()])?;
    let certificate = identity.cert.der().clone();
    let pin = format_fingerprint(&Sha256::digest(certificate.as_ref()).into());
    let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into();
    let tls = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)?;
    let (send_update, updates) = mpsc::channel(2);
    let (log, mut received) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(address)
        .with_tls(TlsAcceptor::from(Arc::new(tls)))
        .with_input_handler(InputLog(log))
        .with_display_handler(Display(Some(Updates(updates))))
        .build();
    let serve = tokio::task::spawn_local(async move {
        let (stream, _) = listener.accept().await?;
        server.run_connection(stream).await
    });
    let connection = |port| Connection {
        endpoint: Endpoint {
            host: "127.0.0.1".to_owned(),
            port,
            username: "fixture".to_owned(),
            fingerprint: pin.clone(),
        },
        password: "fixture-only".to_owned(),
        width: 1280,
        height: 800,
    };
    let sessions = Sessions::default();
    let id = start_session(
        &sessions,
        connection(address.port()),
        |_| ClipboardLink::unsupported(),
        trust::TrustMap::new(),
    )
    .map_err(anyhow::Error::msg)?;
    let (view, input) = sessions
        .get(id, |session| {
            Ok((session.view.clone(), session.input.clone()))
        })
        .map_err(anyhow::Error::msg)?;
    time::timeout(Duration::from_secs(8), async {
        loop {
            match view.lock().unwrap().status() {
                Status::WaitingForFrame => break Ok(()),
                Status::Ended { detail, .. } => break Err(anyhow::Error::msg(detail)),
                _ => {}
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    send_update.send(color_frame([255, 0, 0, 255])).await?;
    let before = wait_frame(&view, 0).await;
    assert!(matches!(view.lock().unwrap().status(), Status::Ready));
    assert_eq!((before.width, before.height), (320, 240));
    let [red, green, blue, alpha] = before.wire[8..12].try_into()?;
    assert!(red >= 250 && green <= 5 && blue <= 5 && alpha == 255);

    input
        .send(vec![
            Input::Button {
                x: 42,
                y: 77,
                button: 0,
                pressed: true,
            },
            Input::Button {
                x: 42,
                y: 77,
                button: 0,
                pressed: false,
            },
            Input::Text {
                text: "验".to_owned(),
            },
            Input::Key {
                scan_code: 0x1c,
                pressed: true,
            },
            Input::Key {
                scan_code: 0x1c,
                pressed: false,
            },
        ])
        .await?;
    time::timeout(Duration::from_secs(5), async {
        let mut observed = [false; 7];
        while !observed.iter().all(|value| *value) {
            let index = match received
                .recv()
                .await
                .expect("fixture input channel is open")
            {
                Received::Mouse(MouseEvent::Move { x: 42, y: 77 }) => 0,
                Received::Mouse(MouseEvent::LeftPressed) => 1,
                Received::Mouse(MouseEvent::LeftReleased) => 2,
                Received::Keyboard(KeyboardEvent::UnicodePressed(0x9a8c)) => 3,
                Received::Keyboard(KeyboardEvent::UnicodeReleased(0x9a8c)) => 4,
                Received::Keyboard(KeyboardEvent::Pressed {
                    code: 0x1c,
                    extended: false,
                }) => 5,
                Received::Keyboard(KeyboardEvent::Released {
                    code: 0x1c,
                    extended: false,
                }) => 6,
                _ => continue,
            };
            observed[index] = true;
        }
    })
    .await?;
    send_update.send(color_frame([0, 255, 0, 255])).await?;
    let after = wait_frame(&view, before.revision).await;
    let encoded = after.png()?;
    let mut decoded = png::Decoder::new(Cursor::new(encoded)).read_info()?;
    let mut pixels = vec![0; decoded.output_buffer_size().unwrap()];
    let info = decoded.next_frame(&mut pixels)?;
    assert_eq!((info.width, info.height), (320, 240));
    assert_eq!(&pixels[..info.buffer_size()], &after.wire[8..]);
    let [red, green, blue, alpha] = pixels[..4].try_into()?;
    assert!(red <= 5 && green >= 250 && blue <= 5 && alpha == 255);
    serve.abort();
    let _ = serve.await;
    // Auto-reconnect kicks in after a live desktop was shown; the worker only
    // stops retrying once the session is dropped.
    time::timeout(Duration::from_secs(5), async {
        while !matches!(view.lock().unwrap().status(), Status::Reconnecting) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    sessions.disconnect(id);
    assert!(sessions.frame(id).is_err());

    let standby = TcpListener::bind(("127.0.0.1", 0)).await?;
    let next_id = start_session(
        &sessions,
        connection(standby.local_addr()?.port()),
        |_| ClipboardLink::unsupported(),
        trust::TrustMap::new(),
    )
    .map_err(anyhow::Error::msg)?;
    assert_ne!(next_id, id);
    sessions.disconnect(id);
    assert!(sessions.frame(id).is_err());
    assert!(sessions.get(next_id, |_| Ok(())).is_ok());
    sessions.disconnect(next_id);
    assert!(sessions.frame(next_id).is_err());
    Ok(())
}

/// Resize requests against a server without the Display Control channel must
/// be no-ops that leave the session fully usable (frames keep flowing, input
/// accepted, no error surfaced).
#[tokio::test]
async fn resize_input_keeps_session_alive_without_display_control() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let address = listener.local_addr()?;
            let identity = generate_simple_self_signed(vec!["localhost".to_owned()])?;
            let certificate = identity.cert.der().clone();
            let pin = format_fingerprint(&Sha256::digest(certificate.as_ref()).into());
            let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into();
            let tls = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key)?;
            let (send_update, updates) = mpsc::channel(2);
            let (log, _received) = mpsc::unbounded_channel();
            let mut server = RdpServer::builder()
                .with_addr(address)
                .with_tls(TlsAcceptor::from(Arc::new(tls)))
                .with_input_handler(InputLog(log))
                .with_display_handler(Display(Some(Updates(updates))))
                .build();
            let serve = tokio::task::spawn_local(async move {
                let (stream, _) = listener.accept().await?;
                server.run_connection(stream).await
            });
            let sessions = Sessions::default();
            let id = start_session(
                &sessions,
                Connection {
                    endpoint: Endpoint {
                        host: "127.0.0.1".to_owned(),
                        port: address.port(),
                        username: "fixture".to_owned(),
                        fingerprint: pin.clone(),
                    },
                    password: "fixture-only".to_owned(),
                    width: 1280,
                    height: 800,
                },
                |_| ClipboardLink::unsupported(),
                trust::TrustMap::new(),
            )
            .map_err(anyhow::Error::msg)?;
            let (view, input) = sessions
                .get(id, |session| Ok((session.view.clone(), session.input.clone())))
                .map_err(anyhow::Error::msg)?;
            time::timeout(Duration::from_secs(8), async {
                loop {
                    match view.lock().unwrap().status() {
                        Status::WaitingForFrame => break,
                        Status::Ended { detail, .. } => panic!("{detail}"),
                        _ => {}
                    }
                    time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            send_update.send(color_frame([255, 0, 0, 255])).await?;
            let before = wait_frame(&view, 0).await;

            // Even, odd-and-too-small, and oversized requests: all sanitized and
            // all silently dropped because the loopback server has no DVC.
            for (width, height) in [(1024, 768), (999, 55), (5000, 3000)] {
                input
                    .send(vec![Input::Resize { width, height }])
                    .await?;
            }
            send_update.send(color_frame([0, 255, 0, 255])).await?;
            wait_frame(&view, before.revision).await;
            assert!(matches!(view.lock().unwrap().status(), Status::Ready));
            assert!(input.send(vec![Input::Key { scan_code: 0x1c, pressed: true }]).await.is_ok());

            serve.abort();
            let _ = serve.await;
            sessions.disconnect(id);
            Ok(())
        })
        .await
}

/// When the server side vanishes mid-session, the worker must reconnect
/// automatically (same session id, revisions keep rising) once a new listener
/// appears on the same address.
#[tokio::test]
async fn dropped_session_reconnects_to_a_new_server() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let address = listener.local_addr()?;
            let identity = generate_simple_self_signed(vec!["localhost".to_owned()])?;
            let certificate = identity.cert.der().clone();
            let pin = format_fingerprint(&Sha256::digest(certificate.as_ref()).into());
            let key_der = identity.signing_key.serialize_der();
            let make_tls = |key_der: &[u8]| {
                Ok::<_, anyhow::Error>(TlsAcceptor::from(Arc::new(
                    ServerConfig::builder()
                        .with_no_client_auth()
                        .with_single_cert(vec![certificate.clone()], PrivatePkcs8KeyDer::from(key_der.to_vec()).into())?,
                )))
            };
            let (send_update, updates) = mpsc::channel(2);
            let mut server1 = RdpServer::builder()
                .with_addr(address)
                .with_tls(make_tls(&key_der)?)
                .with_input_handler(InputLog(mpsc::unbounded_channel().0))
                .with_display_handler(Display(Some(Updates(updates))))
                .build();
            let serve1 = tokio::task::spawn_local(async move {
                let (stream, _) = listener.accept().await?;
                server1.run_connection(stream).await
            });
            let sessions = Sessions::default();
            let id = start_session(
                &sessions,
                Connection {
                    endpoint: Endpoint {
                        host: "127.0.0.1".to_owned(),
                        port: address.port(),
                        username: "fixture".to_owned(),
                        fingerprint: pin.clone(),
                    },
                    password: "fixture-only".to_owned(),
                    width: 1280,
                    height: 800,
                },
                |_| ClipboardLink::unsupported(),
                trust::TrustMap::new(),
            )
            .map_err(anyhow::Error::msg)?;
            let (view, _input) = sessions
                .get(id, |session| Ok((session.view.clone(), session.input.clone())))
                .map_err(anyhow::Error::msg)?;
            time::timeout(Duration::from_secs(8), async {
                loop {
                    match view.lock().unwrap().status() {
                        Status::WaitingForFrame => break,
                        Status::Ended { detail, .. } => panic!("{detail}"),
                        _ => {}
                    }
                    time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await?;
            send_update.send(color_frame([255, 0, 0, 255])).await?;
            let red = wait_frame(&view, 0).await;

            // Kill server 1; the worker retries while we stand up server 2 on
            // the same port.
            serve1.abort();
            let _ = serve1.await;
            let listener2 = TcpListener::bind(address).await?;
            let (send_update2, updates2) = mpsc::channel(2);
            let mut server2 = RdpServer::builder()
                .with_addr(address)
                .with_tls(make_tls(&key_der)?)
                .with_input_handler(InputLog(mpsc::unbounded_channel().0))
                .with_display_handler(Display(Some(Updates(updates2))))
                .build();
            let serve2 = tokio::task::spawn_local(async move {
                let (stream, _) = listener2.accept().await?;
                server2.run_connection(stream).await
            });
            send_update2.send(color_frame([0, 255, 0, 255])).await?;
            let green = wait_frame(&view, red.revision).await;
            let [red_byte, green_byte, blue_byte, alpha] = green.wire[8..12].try_into()?;
            assert!(red_byte <= 5 && green_byte >= 250 && blue_byte <= 5 && alpha == 255);
            assert!(matches!(view.lock().unwrap().status(), Status::Ready));

            serve2.abort();
            let _ = serve2.await;
            sessions.disconnect(id);
            Ok(())
        })
        .await
}

// ---------- Clipboard loopback (text + files, both directions) ----------

const CLIPBOARD_REMOTE_TEXT: &str = "remote-clipboard-payload";
const CLIPBOARD_LOCAL_TEXT: &str = "local-clipboard-payload";
const CLIPBOARD_FILE_NAME: &str = "hello.txt";
const CLIPBOARD_FILE_CONTENT: &[u8] = b"file-transfer-round-trip";

#[derive(Debug, Default)]
struct ClipboardFixture {
    server_received_text: StdMutex<Vec<String>>,
    server_received_file: StdMutex<Vec<u8>>,
}

/// In-memory OS clipboard driving the production worker/backend stack. Before
/// the first worker write it reports the seeded content; afterwards it reports
/// whatever was published (like a real clipboard being replaced).
#[derive(Debug)]
struct TestOs {
    items: StdMutex<Vec<(u32, Vec<u8>)>>,
    files: StdMutex<Vec<PathBuf>>,
    written: StdMutex<bool>,
    seed_text: Option<String>,
    seed_file: Option<(PathBuf, u64)>,
}

impl clipboard::OsClipboard for TestOs {
    fn is_supported(&self) -> bool {
        true
    }

    fn read(&self) -> Result<LocalSnapshot, String> {
        if *self.written.lock().unwrap() {
            let text = self
                .items
                .lock()
                .unwrap()
                .iter()
                .find(|(format, _)| *format == ClipboardFormatId::CF_UNICODETEXT.value())
                .map(|(_, data)| data.clone());
            let files = self
                .files
                .lock()
                .unwrap()
                .iter()
                .map(|path| LocalFile {
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".into()),
                    path: path.clone(),
                    size: std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
                })
                .collect();
            Ok(LocalSnapshot {
                text,
                dib: None,
                files,
            })
        } else {
            Ok(LocalSnapshot {
                text: self.seed_text.as_ref().map(|text| utf16_wire(text)),
                dib: None,
                files: self
                    .seed_file
                    .as_ref()
                    .map(|(path, size)| {
                        vec![LocalFile {
                            name: CLIPBOARD_FILE_NAME.to_owned(),
                            path: path.clone(),
                            size: *size,
                        }]
                    })
                    .unwrap_or_default(),
            })
        }
    }

    fn write(&self, items: &[(u32, Vec<u8>)], files: &[PathBuf]) -> Result<(), String> {
        *self.items.lock().unwrap() = items.to_vec();
        *self.files.lock().unwrap() = files.to_vec();
        *self.written.lock().unwrap() = true;
        Ok(())
    }
}

fn utf16_wire(text: &str) -> Vec<u8> {
    let mut bytes: Vec<u8> = text
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    bytes.extend_from_slice(&[0, 0]);
    bytes
}

fn decode_utf16_wire(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes(pair.try_into().expect("fixed size")))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Server-side cliprdr backend with selectable behavior per test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerMode {
    /// Announce remote text; pull the client's text when announced.
    Text,
    /// Announce a remote file; pull the client's announced file when announced.
    Files,
}

#[derive(Debug)]
struct ClipboardServerBackend {
    mode: ServerMode,
    events: StdMutex<Option<tokio::sync::mpsc::UnboundedSender<ServerEvent>>>,
    shared: Arc<ClipboardFixture>,
    file_path: PathBuf,
}

impl ClipboardServerBackend {
    fn emit(&self, message: ClipboardMessage) {
        if let Some(sender) = self.events.lock().unwrap().as_ref() {
            let _ = sender.send(ServerEvent::Clipboard(message));
        }
    }
}

impl CliprdrBackend for ClipboardServerBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr-test-server"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
    }

    fn on_ready(&mut self) {
        match self.mode {
            ServerMode::Text => self.emit(ClipboardMessage::SendInitiateCopy(vec![
                ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            ])),
            ServerMode::Files => self.emit(ClipboardMessage::SendInitiateFileCopy(vec![
                FileDescriptor::new(CLIPBOARD_FILE_NAME)
                    .with_attributes(ClipboardFileAttributes::ARCHIVE)
                    .with_file_size(CLIPBOARD_FILE_CONTENT.len() as u64),
            ])),
        }
    }

    fn on_request_format_list(&mut self) {}

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        match self.mode {
            ServerMode::Text => {
                if formats
                    .iter()
                    .any(|format| format.id == ClipboardFormatId::CF_UNICODETEXT)
                {
                    self.emit(ClipboardMessage::SendInitiatePaste(
                        ClipboardFormatId::CF_UNICODETEXT,
                    ));
                }
            }
            ServerMode::Files => {
                if formats
                    .iter()
                    .any(|format| format.id.value() == clipboard::FDW_FORMAT_ID)
                {
                    self.emit(ClipboardMessage::SendInitiatePaste(ClipboardFormatId::new(
                        clipboard::FDW_FORMAT_ID,
                    )));
                }
            }
        }
    }

    fn on_format_data_request(&mut self, _: FormatDataRequest) {
        match self.mode {
            ServerMode::Text => self.emit(ClipboardMessage::SendFormatData(
                FormatDataResponse::new_unicode_string(CLIPBOARD_REMOTE_TEXT),
            )),
            ServerMode::Files => {
                // FDW requests are served by the library from the stored list.
            }
        }
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if let Ok(text) = response.to_unicode_string() {
            self.shared.server_received_text.lock().unwrap().push(text);
        }
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], _: Option<u32>) {
        // The client announced files: request the first file's full contents.
        if let Some(file) = files.first() {
            let size = file.file_size.unwrap_or(0).min(u32::MAX as u64) as u32;
            self.emit(ClipboardMessage::SendFileContentsRequest(FileContentsRequest {
                stream_id: 1,
                index: 0,
                flags: FileContentsFlags::RANGE,
                position: 0,
                requested_size: size,
                data_id: None,
            }));
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        let data = std::fs::read(&self.file_path).unwrap_or_default();
        let start = request.position.min(data.len() as u64) as usize;
        let end = (start + request.requested_size as usize).min(data.len());
        self.emit(ClipboardMessage::SendFileContentsResponse(
            FileContentsResponse::new_data_response(request.stream_id, data[start..end].to_vec()),
        ));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        if !response.is_error() {
            self.shared
                .server_received_file
                .lock()
                .unwrap()
                .extend_from_slice(response.data());
        }
    }

    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

#[derive(Debug)]
struct ClipboardServerFactory {
    mode: ServerMode,
    events: StdMutex<Option<tokio::sync::mpsc::UnboundedSender<ServerEvent>>>,
    shared: Arc<ClipboardFixture>,
    file_path: PathBuf,
}

impl CliprdrBackendFactory for ClipboardServerFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(ClipboardServerBackend {
            mode: self.mode,
            events: StdMutex::new(self.events.lock().unwrap().clone()),
            shared: self.shared.clone(),
            file_path: self.file_path.clone(),
        })
    }
}

impl_as_any!(ClipboardServerBackend);

impl ServerEventSender for ClipboardServerFactory {
    fn set_sender(&mut self, sender: tokio::sync::mpsc::UnboundedSender<ServerEvent>) {
        *self.events.lock().unwrap() = Some(sender);
    }
}

impl CliprdrServerFactory for ClipboardServerFactory {}

/// TOFU: a host whose fingerprint is in the trust store connects with an
/// empty pinned fingerprint.
#[tokio::test]
async fn trusted_host_connects_without_pinned_fingerprint() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(async {
            let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let address = listener.local_addr()?;
            let identity = generate_simple_self_signed(vec!["localhost".to_owned()])?;
            let certificate = identity.cert.der().clone();
            let pin = format_fingerprint(&Sha256::digest(certificate.as_ref()).into());
            let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into();
            let tls = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![certificate], key)?;
            let (send_update, updates) = mpsc::channel(2);
            let (log, _) = mpsc::unbounded_channel();
            let mut server = RdpServer::builder()
                .with_addr(address)
                .with_tls(TlsAcceptor::from(Arc::new(tls)))
                .with_input_handler(InputLog(log))
                .with_display_handler(Display(Some(Updates(updates))))
                .build();
            let serve = tokio::task::spawn_local(async move {
                let (stream, _) = listener.accept().await?;
                server.run_connection(stream).await
            });

            let mut trusted = trust::TrustMap::new();
            trusted.insert("127.0.0.1".to_owned(), pin);

            let sessions = Sessions::default();
            let connection = Connection {
                endpoint: Endpoint {
                    host: "127.0.0.1".to_owned(),
                    port: address.port(),
                    username: "fixture".to_owned(),
                    fingerprint: String::new(), // no manual pin — the store decides
                },
                password: "fixture-only".to_owned(),
                width: 1280,
                height: 800,
            };
            let id = start_session(
                &sessions,
                connection,
                |_| ClipboardLink::unsupported(),
                trusted,
            )
            .map_err(anyhow::Error::msg)?;
            let (view, _) = sessions
                .get(id, |session| Ok((session.view.clone(), session.input.clone())))
                .map_err(anyhow::Error::msg)?;

            // TLS acceptance is proven by the session reaching WaitingForFrame.
            time::timeout(Duration::from_secs(8), async {
                loop {
                    match view.lock().unwrap().status() {
                        Status::WaitingForFrame => break Ok(()),
                        Status::Ended { detail, .. } => break Err(anyhow::Error::msg(detail)),
                        _ => {}
                    }
                    time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await??;
            send_update.send(color_frame([255, 0, 0, 255])).await?;

            serve.abort();
            let _ = serve.await;
            sessions.disconnect(id);
            Ok(())
        })
        .await
}

#[tokio::test]
async fn clipboard_text_syncs_both_ways() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(verify_clipboard_loopback(ServerMode::Text))
        .await
}

#[tokio::test]
async fn clipboard_files_sync_both_ways() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(verify_clipboard_loopback(ServerMode::Files))
        .await
}

async fn verify_clipboard_loopback(mode: ServerMode) -> anyhow::Result<()> {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    let identity = generate_simple_self_signed(vec!["localhost".to_owned()])?;
    let certificate = identity.cert.der().clone();
    let pin = format_fingerprint(&Sha256::digest(certificate.as_ref()).into());
    let key = PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into();
    let tls = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)?;

    // Local side seeds: text (both modes) plus a real file (files mode).
    // Parallel tests must not share temp paths: include the mode in the names.
    let local_file = std::env::temp_dir().join(format!(
        "auuremote-test-{}-{mode:?}.txt",
        std::process::id()
    ));
    std::fs::write(&local_file, CLIPBOARD_FILE_CONTENT)?;
    let staging_root = std::env::temp_dir().join(format!(
        "auuremote-staging-{}-{mode:?}",
        std::process::id()
    ));
    std::fs::create_dir_all(&staging_root)?;

    let shared = Arc::new(ClipboardFixture::default());
    let os: Arc<dyn clipboard::OsClipboard + Send + Sync> = Arc::new(TestOs {
        items: StdMutex::new(Vec::new()),
        files: StdMutex::new(Vec::new()),
        written: StdMutex::new(false),
        seed_text: if mode == ServerMode::Text {
            Some(CLIPBOARD_LOCAL_TEXT.to_owned())
        } else {
            None
        },
        seed_file: if mode == ServerMode::Files {
            Some((local_file.clone(), CLIPBOARD_FILE_CONTENT.len() as u64))
        } else {
            None
        },
    });

    let (send_update, updates) = mpsc::channel(2);
    let (log, _) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(address)
        .with_tls(TlsAcceptor::from(Arc::new(tls)))
        .with_input_handler(InputLog(log))
        .with_display_handler(Display(Some(Updates(updates))))
        .with_cliprdr_factory(Some(Box::new(ClipboardServerFactory {
            mode,
            events: StdMutex::new(None),
            shared: shared.clone(),
            file_path: local_file.clone(),
        })))
        .build();
    let serve = tokio::task::spawn_local(async move {
        let (stream, _) = listener.accept().await?;
        server.run_connection(stream).await
    });

    let connection = |port| Connection {
        endpoint: Endpoint {
            host: "127.0.0.1".to_owned(),
            port,
            username: "fixture".to_owned(),
            fingerprint: pin.clone(),
        },
        password: "fixture-only".to_owned(),
        width: 1280,
        height: 800,
    };
    let sessions = Sessions::default();

    // Production link: the real SessionBackend + real Worker stack, with an
    // in-memory OS clipboard standing in for Win32.
    let state = Arc::new(StdMutex::new(clipboard::SessionState::default()));
    let (clipboard_tx, clipboard_rx) = mpsc::unbounded_channel();
    let factory: clipboard::SharedCliprdrFactory = Arc::new(StdMutex::new(Box::new(
        clipboard::SessionFactory {
            events: clipboard_tx,
            state: state.clone(),
        },
    ) as Box<dyn CliprdrBackendFactory + Send>));
    let link = ClipboardLink::raw(
        Some(factory),
        clipboard_rx,
        Some(os.clone()),
        Some(staging_root.clone()),
        state,
    );
    let id = start_session(
        &sessions,
        connection(address.port()),
        |_| link,
        trust::TrustMap::new(),
    )
    .map_err(anyhow::Error::msg)?;
    let (view, _) = sessions
        .get(id, |session| Ok((session.view.clone(), session.input.clone())))
        .map_err(anyhow::Error::msg)?;

    time::timeout(Duration::from_secs(8), async {
        loop {
            match view.lock().unwrap().status() {
                Status::WaitingForFrame => break Ok(()),
                Status::Ended { detail, .. } => break Err(anyhow::Error::msg(detail)),
                _ => {}
            }
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    send_update.send(color_frame([255, 0, 0, 255])).await?;

    let passed = time::timeout(Duration::from_secs(30), async {
        loop {
            let text_synced = shared
                .server_received_text
                .lock()
                .unwrap()
                .iter()
                .any(|text| text == CLIPBOARD_LOCAL_TEXT);
            let os_text = os
                .read()
                .ok()
                .and_then(|snapshot| snapshot.text)
                .map(|wire| decode_utf16_wire(&wire) == CLIPBOARD_REMOTE_TEXT)
                .unwrap_or(false);
            let file_synced = shared
                .server_received_file
                .lock()
                .unwrap()
                .as_slice()
                == CLIPBOARD_FILE_CONTENT;
            // The worker stages downloads into a per-copy subdirectory, so
            // check the paths the worker published to the (in-memory) OS.
            let file_staged = os
                .read()
                .ok()
                .map(|snapshot| {
                    snapshot.files.iter().any(|file| {
                        std::fs::read(&file.path)
                            .map(|data| data == CLIPBOARD_FILE_CONTENT)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            let done = match mode {
                ServerMode::Text => text_synced && os_text,
                ServerMode::Files => file_synced && file_staged,
            };
            if done {
                break;
            }
            time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    if let Err(error) = passed {
        eprintln!(
            "clipboard state: server_text={:?} server_file={:?} os_items={:?}",
            shared.server_received_text.lock().unwrap(),
            shared.server_received_file.lock().unwrap(),
            os.read().ok(),
        );
        return Err(error.into());
    }

    let _ = std::fs::remove_file(&local_file);
    let _ = std::fs::remove_dir_all(&staging_root);
    serve.abort();
    let _ = serve.await;
    sessions.disconnect(id);
    Ok(())
}

