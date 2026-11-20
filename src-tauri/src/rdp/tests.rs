use std::{
    io::Cursor,
    num::{NonZeroU16, NonZeroUsize},
    sync::Arc,
    time::Duration,
};

use ironrdp::cliprdr::{
    backend::{CliprdrBackend, CliprdrBackendFactory, ClipboardMessage},
    pdu::{
        ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
        FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    },
};
use ironrdp_server::{
    BitmapUpdate, CliprdrServerFactory, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent,
    PixelFormat, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
    ServerEvent, ServerEventSender,
};
use ironrdp::core::impl_as_any;
use rcgen::generate_simple_self_signed;
use anyhow::Context;
use sha2::{Digest, Sha256};
use tokio::{net::TcpListener, time};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
};

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
    };
    let sessions = Sessions::default();
    let id = start_session(
        &sessions,
        connection(address.port()),
        |_| clipboard::ClipboardLink::raw(None, mpsc::unbounded_channel().1),
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
    time::timeout(Duration::from_secs(5), async {
        while !matches!(view.lock().unwrap().status(), Status::Ended { .. }) {
            time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert!(sessions.frame(id).is_err());
    sessions.disconnect(id);

    let standby = TcpListener::bind(("127.0.0.1", 0)).await?;
    let next_id = start_session(
        &sessions,
        connection(standby.local_addr()?.port()),
        |_| clipboard::ClipboardLink::raw(None, mpsc::unbounded_channel().1),
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

// ---------- Clipboard loopback ----------

const CLIPBOARD_REMOTE_TEXT: &str = "remote-clipboard-payload";
const CLIPBOARD_LOCAL_TEXT: &str = "local-clipboard-payload";

#[derive(Debug, Default)]
struct ClipboardFixture {
    client_received: std::sync::Mutex<Vec<String>>,
    server_received: std::sync::Mutex<Vec<String>>,
    client_local_text: std::sync::Mutex<String>,
}

#[derive(Debug)]
struct ClipboardClientBackend {
    tx: tokio::sync::mpsc::UnboundedSender<ClipboardMessage>,
    shared: Arc<ClipboardFixture>,
}

impl CliprdrBackend for ClipboardClientBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr-test"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        // Server is ready: announce our local clipboard like the OS backend.
        let _ = self.tx.send(ClipboardMessage::SendInitiateCopy(vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]));
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        if formats
            .iter()
            .any(|format| format.id == ClipboardFormatId::CF_UNICODETEXT)
        {
            let _ = self.tx.send(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, _: FormatDataRequest) {
        let text = self.shared.client_local_text.lock().unwrap().clone();
        let _ = self.tx.send(ClipboardMessage::SendFormatData(
            FormatDataResponse::new_unicode_string(&text),
        ));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if let Ok(text) = response.to_unicode_string() {
            self.shared.client_received.lock().unwrap().push(text);
        }
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

#[derive(Debug)]
struct ClipboardClientFactory {
    tx: tokio::sync::mpsc::UnboundedSender<ClipboardMessage>,
    shared: Arc<ClipboardFixture>,
}

impl CliprdrBackendFactory for ClipboardClientFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(ClipboardClientBackend {
            tx: self.tx.clone(),
            shared: self.shared.clone(),
        })
    }
}

impl_as_any!(ClipboardClientBackend);

#[derive(Debug)]
struct ClipboardServerBackend {
    events: Arc<std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<ServerEvent>>>>,
    shared: Arc<ClipboardFixture>,
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
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_ready(&mut self) {
        // Simulate the remote user copying text.
        self.emit(ClipboardMessage::SendInitiateCopy(vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
        ]));
    }

    fn on_request_format_list(&mut self) {}

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        if formats
            .iter()
            .any(|format| format.id == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.emit(ClipboardMessage::SendInitiatePaste(
                ClipboardFormatId::CF_UNICODETEXT,
            ));
        }
    }

    fn on_format_data_request(&mut self, _: FormatDataRequest) {
        self.emit(ClipboardMessage::SendFormatData(
            FormatDataResponse::new_unicode_string(CLIPBOARD_REMOTE_TEXT),
        ));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if let Ok(text) = response.to_unicode_string() {
            self.shared.server_received.lock().unwrap().push(text);
        }
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}
    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}
    fn on_lock(&mut self, _: LockDataId) {}
    fn on_unlock(&mut self, _: LockDataId) {}
}

#[derive(Debug)]
struct ClipboardServerFactory {
    events: Arc<std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<ServerEvent>>>>,
    shared: Arc<ClipboardFixture>,
}

impl CliprdrBackendFactory for ClipboardServerFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(ClipboardServerBackend {
            events: self.events.clone(),
            shared: self.shared.clone(),
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

#[tokio::test]
async fn clipboard_syncs_both_ways_over_one_tls_connection() -> anyhow::Result<()> {
    tokio::task::LocalSet::new()
        .run_until(verify_clipboard_loopback())
        .await
}

async fn verify_clipboard_loopback() -> anyhow::Result<()> {
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
    let shared = Arc::new(ClipboardFixture {
        client_local_text: std::sync::Mutex::new(CLIPBOARD_LOCAL_TEXT.to_owned()),
        ..ClipboardFixture::default()
    });
    let (send_update, updates) = mpsc::channel(2);
    let (log, _) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(address)
        .with_tls(TlsAcceptor::from(Arc::new(tls)))
        .with_input_handler(InputLog(log))
        .with_display_handler(Display(Some(Updates(updates))))
        .with_cliprdr_factory(Some(Box::new(ClipboardServerFactory {
            events: Arc::default(),
            shared: shared.clone(),
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
    };
    let sessions = Sessions::default();
    let (clipboard_tx, clipboard_rx) = mpsc::unbounded_channel();
    let client_factory: clipboard::SharedCliprdrFactory =
        Arc::new(std::sync::Mutex::new(Box::new(ClipboardClientFactory {
            tx: clipboard_tx,
            shared: shared.clone(),
        })));
    let id = start_session(
        &sessions,
        connection(address.port()),
        |_| clipboard::ClipboardLink::raw(Some(client_factory), clipboard_rx),
    )
    .map_err(anyhow::Error::msg)?;
    let (view, input) = sessions
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
    wait_frame(&view, 0).await;

    // Both directions flow automatically once the channel handshake completes:
    // server announce → client paste → client received, and client announce →
    // server paste → server received.
    time::timeout(Duration::from_secs(8), async {
        loop {
            let client_got_remote = shared
                .client_received
                .lock()
                .unwrap()
                .iter()
                .any(|text| text == CLIPBOARD_REMOTE_TEXT);
            let server_got_local = shared
                .server_received
                .lock()
                .unwrap()
                .iter()
                .any(|text| text == CLIPBOARD_LOCAL_TEXT);
            if client_got_remote && server_got_local {
                break;
            }
            time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .context("clipboard round-trip did not complete in time")?;

    input
        .send(vec![Input::Text {
            text: "剪".to_owned(),
        }])
        .await?;
    serve.abort();
    let _ = serve.await;
    sessions.disconnect(id);
    Ok(())
}
