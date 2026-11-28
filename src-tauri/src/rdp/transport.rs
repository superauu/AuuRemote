use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ironrdp::{
    cliprdr::CliprdrClient,
    connector::{
        ClientConnector, Config, ConnectionResult, Credentials, DesktopSize,
        connection_activation::ConnectionActivationState,
    },
    core::WriteBuf,
    graphics::image_processing::PixelFormat,
    input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations},
    pdu::{gcc::KeyboardType, rdp::capability_sets::MajorPlatformType},
    session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput, fast_path, image::DecodedImage},
};
use ironrdp_tls::{CertificateValidationCallback, TlsStream};
use ironrdp_tokio::{FramedWrite, TokioFramed, single_sequence_step_read, reqwest::ReqwestNetworkClient};
use sha2::{Digest, Sha256};
use tokio::{
    io::{ReadHalf, WriteHalf},
    net::TcpStream,
    sync::mpsc,
    time,
};

use super::{
    Connection, Endpoint, Frame, Input, View, format_fingerprint,
    clipboard::ClipboardLink,
};
use crate::errors::{ApiError, ErrorCode};

type RdpStream = TokioFramed<TlsStream<TcpStream>>;
type RdpReader = TokioFramed<ReadHalf<TlsStream<TcpStream>>>;
type RdpWriter = TokioFramed<WriteHalf<TlsStream<TcpStream>>>;

/// Auto-reconnect budget for a session that already showed a live desktop.
const MAX_RECONNECTS: u32 = 3;
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

pub(super) async fn run(
    connection: Connection,
    fingerprint: Option<[u8; 32]>,
    input: mpsc::Receiver<Vec<Input>>,
    clipboard: ClipboardLink,
    trusted: super::trust::TrustMap,
    view: &Mutex<View>,
) -> Result<(), ApiError> {
    let ClipboardLink {
        factory: cliprdr_factory,
        rx: mut clipboard_rx,
        os,
        staging_root,
        state,
    } = clipboard;
    let transfer = view.lock().expect("view lock").transfer.clone();
    let mut clipboard_worker = super::clipboard::Worker::new(os, staging_root, state, transfer);
    let mut input = input;
    let mut reconnects_left: u32 = MAX_RECONNECTS;
    // Frame revisions keep rising across reconnects so the canvas keeps
    // pulling frames without a reset.
    let mut revision: u32 = 0;

    loop {
        let before = revision;
        let outcome = session_once(
            connection.clone(),
            fingerprint,
            trusted.clone(),
            cliprdr_factory.clone(),
            &mut input,
            &mut clipboard_worker,
            &mut clipboard_rx,
            view,
            &mut revision,
        )
        .await;
        // This attempt showed frames: the reconnect budget refills for the
        // next drop, while a streak of failed attempts drains it.
        if revision != before {
            reconnects_left = MAX_RECONNECTS;
        }
        let retryable = outcome.is_err()
            && view.lock().expect("view lock").established
            && view.lock().expect("view lock").error.is_none()
            && reconnects_left > 0;
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) if retryable => {
                eprintln!("rdp: session dropped ({error}); auto-reconnecting");
                reconnects_left -= 1;
                view.lock().expect("view lock").reconnecting = true;
                time::sleep(RECONNECT_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// One full connect + active-session cycle. Only `Ok` when the user closed
/// the tab (input channel closed); every protocol failure is an `Err` that
/// the `run` wrapper may turn into an automatic retry.
#[allow(clippy::too_many_arguments)]
async fn session_once(
    connection: Connection,
    fingerprint: Option<[u8; 32]>,
    trusted: super::trust::TrustMap,
    cliprdr_factory: Option<super::clipboard::SharedCliprdrFactory>,
    input: &mut mpsc::Receiver<Vec<Input>>,
    clipboard_worker: &mut super::clipboard::Worker,
    clipboard_rx: &mut mpsc::UnboundedReceiver<super::clipboard::CliprdrEvent>,
    view: &Mutex<View>,
    revision: &mut u32,
) -> Result<(), ApiError> {
    let (result, stream) =
        time::timeout(
            Duration::from_secs(25),
            connect(connection, fingerprint, cliprdr_factory, trusted),
        )
        .await
        .map_err(|_| ApiError::new(ErrorCode::ConnectTimeout, "RDP 连接超时"))??;
    {
        let mut latest = view.lock().expect("view lock");
        latest.established = true;
        latest.reconnecting = false;
    }
    let activation_factory = result.activation_factory;

    let DesktopSize { mut width, mut height } = result.desktop_size;
    if !(width > 0 && height > 0 && width <= 4096 && height <= 2160) {
        return Err(ApiError::new(
            ErrorCode::ResolutionUnsupported,
            "远端分辨率超出原型范围。",
        ));
    }
    let mut image = DecodedImage::new(PixelFormat::RgbA32, width, height);
    let mut stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();
    // Split so the Deactivation-Reactivation handshake can read the stream
    // directly while the main loop owns the read half the rest of the time.
    let (mut reader, mut writer) = ironrdp_tokio::split_tokio_framed(stream);
    let mut keys = Database::new();
    let mut tick = time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let mut cliprdr_sweep = time::interval(Duration::from_secs(5));
    cliprdr_sweep.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    cliprdr_sweep.tick().await; // consume the immediate first tick
    let first_frame = time::sleep(Duration::from_secs(30));
    tokio::pin!(first_frame);
    let mut dirty = false;

    loop {
        let output = tokio::select! {
            received = reader.read_pdu() => {
                let (action, bytes) = received
                    .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "读取远端画面失败"))?;
                stage.process(&mut image, action, &bytes)
                    .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "解码远端画面失败"))?
            }
            received = input.recv() => {
                let Some(batch) = received else { return Ok(()); };
                let mut output = Vec::new();
                for event in batch {
                    match event {
                        Input::Resize { width: requested_width, height: requested_height } => {
                            output.extend(request_desktop_resize(&mut stage, requested_width, requested_height));
                        }
                        other => {
                            let events = input_events(&mut keys, other, width, height);
                            for chunk in events.chunks(64) {
                                output.extend(stage.process_fastpath_input(&mut image, chunk)
                                    .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "编码远端输入失败"))?);
                            }
                        }
                    }
                }
                output
            }
            message = async {
                match clipboard_rx.recv().await {
                    Some(event) => event,
                    // Channel closed (clipboard unsupported): park this branch
                    // forever instead of ending the session.
                    None => std::future::pending::<super::clipboard::CliprdrEvent>().await,
                }
            } => {
                clipboard_worker.handle(&mut stage, message)
            }
            _ = cliprdr_sweep.tick() => {
                sweep_cliprdr_timeouts(&mut stage)
            }
            _ = tick.tick() => {
                if dirty {
                    *revision = revision.wrapping_add(1).max(1);
                    let frame = Arc::new(make_frame(&image, *revision));
                    view.lock().expect("view lock").frame = Some(frame);
                    dirty = false;
                }
                continue;
            }
            _ = &mut first_frame, if *revision == 0 => return Err(ApiError::new(
                ErrorCode::ConnectionFailed,
                "RDP 已建立，但未收到远端桌面画面。",
            )),
        };

        for event in output {
            match event {
                ActiveStageOutput::ResponseFrame(frame) if !frame.is_empty() => {
                    time::timeout(Duration::from_secs(10), writer.write_all(&frame))
                        .await
                        .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "发送 RDP 数据超时"))?
                        .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "发送 RDP 数据失败"))?;
                }
                ActiveStageOutput::GraphicsUpdate(_) => dirty = true,
                ActiveStageOutput::Terminate(reason) => {
                    return Err(ApiError::new(
                        ErrorCode::ConnectionFailed,
                        format!("远端已断开。{}", reason.description()),
                    ));
                }
                ActiveStageOutput::DeactivateAll => {
                    reactivate(
                        &mut reader,
                        &mut writer,
                        &activation_factory,
                        &mut stage,
                        &mut image,
                        &mut width,
                        &mut height,
                    )
                    .await?;
                }
                _ => {}
            }
        }
    }
}

/// Drives cliprdr lock-expiry bookkeeping every few seconds.
fn sweep_cliprdr_timeouts(stage: &mut ironrdp::session::ActiveStage) -> Vec<ActiveStageOutput> {
    let Some(cliprdr) = stage.get_svc_processor_mut::<CliprdrClient>() else {
        return Vec::new();
    };
    match cliprdr.drive_timeouts() {
        Ok(messages) => match stage.process_svc_processor_messages(messages) {
            Ok(frame) => vec![ActiveStageOutput::ResponseFrame(frame)],
            Err(error) => {
                eprintln!("cliprdr: sweep encode failed ({error})");
                Vec::new()
            }
        },
        Err(error) => {
            eprintln!("cliprdr: sweep failed ({error})");
            Vec::new()
        }
    }
}

/// Time budget for the whole Deactivation-Reactivation handshake.
const REACTIVATION_TIMEOUT: Duration = Duration::from_secs(15);

/// Pushes a new single-monitor layout over the Display Control channel. The
/// server responds with a Deactivation-Reactivation Sequence (handled at the
/// `DeactivateAll` output); servers without the channel keep the old size.
fn request_desktop_resize(stage: &mut ActiveStage, width: u16, height: u16) -> Vec<ActiveStageOutput> {
    let size = sanitized_desktop_size(width, height);
    match stage.encode_resize(u32::from(size.width), u32::from(size.height), None, None) {
        Some(Ok(frame)) => vec![ActiveStageOutput::ResponseFrame(frame)],
        // A failed encode sent nothing: keep the session alive and the old size.
        Some(Err(error)) => {
            eprintln!("displaycontrol: resize encode failed ({error})");
            Vec::new()
        }
        None => Vec::new(),
    }
}

/// Runs the Deactivation-Reactivation Sequence (MS-RDPBCGR §1.3.11.1.1) after
/// the server suspended the session, typically to adopt a new desktop size.
/// The handshake reads its own PDUs from the stream, bypassing the active
/// stage, and ends by adopting the negotiated framebuffer size.
async fn reactivate(
    reader: &mut RdpReader,
    writer: &mut RdpWriter,
    factory: &ironrdp::connector::connection_activation::ConnectionActivationFactory,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    width: &mut u16,
    height: &mut u16,
) -> Result<(), ApiError> {
    let mut connection_activation = factory.create();
    let mut buffer = WriteBuf::new();
    time::timeout(REACTIVATION_TIMEOUT, async {
        loop {
            let written = single_sequence_step_read(reader, &mut connection_activation, &mut buffer)
                .await
                .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "桌面重激活握手失败"))?;
            if let Some(_response_len) = written.size() {
                writer
                    .write_all(buffer.filled())
                    .await
                    .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "桌面重激活握手失败"))?;
            }
            if let ConnectionActivationState::Finalized {
                desktop_size,
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
            } = connection_activation.connection_activation_state()
            {
                *width = desktop_size.width;
                *height = desktop_size.height;
                *image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                stage.set_fastpath_processor(
                    fast_path::ProcessorBuilder {
                        io_channel_id: connection_activation.io_channel_id(),
                        user_channel_id: connection_activation.user_channel_id(),
                        share_id,
                        enable_server_pointer,
                        pointer_software_rendering,
                        bulk_decompressor: None,
                    }
                    .build(),
                );
                stage.set_share_id(share_id);
                stage.set_enable_server_pointer(enable_server_pointer);
                return Ok::<(), ApiError>(());
            }
        }
    })
    .await
    .map_err(|_| ApiError::new(ErrorCode::ConnectionFailed, "桌面重激活握手超时。"))??;
    Ok(())
}

fn make_frame(image: &DecodedImage, revision: u32) -> Frame {
    let (width, height) = (image.width(), image.height());
    let mut wire = Vec::with_capacity(8 + image.data().len());
    wire.extend_from_slice(&revision.to_le_bytes());
    wire.extend_from_slice(&width.to_le_bytes());
    wire.extend_from_slice(&height.to_le_bytes());
    wire.extend_from_slice(image.data());
    // RDP desktop pixels are opaque, including when a bitmap decoder supplies a padding byte.
    for pixel in wire[8..].chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    Frame {
        revision,
        width,
        height,
        wire,
    }
}

async fn connect(
    connection: Connection,
    fingerprint: Option<[u8; 32]>,
    cliprdr_factory: Option<super::clipboard::SharedCliprdrFactory>,
    trusted: super::trust::TrustMap,
) -> Result<(ConnectionResult, RdpStream), ApiError> {
    let Connection {
        endpoint:
            Endpoint {
                host,
                port,
                username,
                ..
            },
        password,
        width,
        height,
    } = connection;
    let desktop_size = sanitized_desktop_size(width, height);
    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "无法连接 RDP 主机"))?;
    tcp.set_nodelay(true)
        .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "无法配置 TCP 连接"))?;
    let local = tcp
        .local_addr()
        .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "无法读取本机地址"))?;
    let mut stream = TokioFramed::new(tcp);
    let (domain, username) = match username.split_once('\\') {
        Some((domain, user)) => (Some(domain.to_owned()), user.to_owned()),
        None => (None, username),
    };
    let mut connector = ClientConnector::new(
        client_config(username, password, domain, desktop_size),
        local,
    );
    // Display Control channel: the server resizes the session to our layout
    // (Windows 10+/Server 2016+). Unsupported servers simply ignore it.
    let display_control =
        ironrdp::displaycontrol::client::DisplayControlClient::new(move |_| {
            // Our sizes are clamped/validated up front, so building the layout
            // cannot fail; on the off chance it does, skip the layout push and
            // keep the negotiated session size.
            let Ok(layout) = ironrdp::displaycontrol::pdu::DisplayControlMonitorLayout::new_single_primary_monitor(
                u32::from(desktop_size.width),
                u32::from(desktop_size.height),
                None,
                None,
            ) else {
                return Ok(Vec::new());
            };
            Ok(vec![Box::new(
                ironrdp::displaycontrol::pdu::DisplayControlPdu::from(layout),
            ) as Box<dyn ironrdp::dvc::DvcEncode>])
        });
    connector.attach_static_channel(
        ironrdp::dvc::DrdynvcClient::new().with_dynamic_channel(display_control),
    );
    if let Some(factory) = &cliprdr_factory {
        let backend = factory
            .lock()
            .expect("cliprdr factory")
            .build_cliprdr_backend();
        connector.attach_static_channel(CliprdrClient::new(backend));
    }
    // Audio playback: the server streams wave blocks; the cpal backend plays
    // them on the default output device (PCM 44.1 kHz stereo).
    connector.attach_static_channel(ironrdp::rdpsnd::client::Rdpsnd::new(Box::new(
        ironrdp_rdpsnd_native::cpal::RdpsndBackend::new(),
    )));
    let upgrade = ironrdp_tokio::connect_begin(&mut stream, &mut connector)
        .await
        .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "RDP 协商失败"))?;
    let (tcp, remainder) = stream.into_inner();
    if !remainder.is_empty() {
        return Err(ApiError::new(
            ErrorCode::ConnectionFailed,
            "RDP 服务器提前发送了 TLS 数据。",
        ));
    }

    let presented = Arc::new(Mutex::new(None));
    let observed = presented.clone();
    let trusted_host = host.clone();
    let callback: CertificateValidationCallback = Arc::new(move |der, _, _| {
        let digest: [u8; 32] = Sha256::digest(der).into();
        *observed.lock().expect("certificate lock") = Some(digest);
        if fingerprint == Some(digest) {
            return true;
        }
        // TOFU: hosts whose certificate the user accepted before connect
        // without re-prompting.
        trusted
            .get(&trusted_host)
            .is_some_and(|stored| *stored == format_fingerprint(&digest))
    });
    let tls = ironrdp_tls::upgrade_with_certificate_validation_callback_for_endpoint(
        tcp,
        &host,
        &format!("{host}:{port}"),
        callback,
    )
    .await;
    let (stream, certificate) = match tls {
        Ok(upgraded) => upgraded,
        Err(error) => {
            if let Some(digest) = *presented.lock().expect("certificate lock") {
                let hex = format_fingerprint(&digest);
                return Err(ApiError::new(
                    ErrorCode::CertificateMismatch,
                    format!(
                        "无法验证远端证书。服务端提交的 SHA-256 指纹为 {hex}。请从可信渠道核对后再填写指纹。"
                    ),
                ));
            }
            return Err(ApiError::chain(ErrorCode::ConnectionFailed, &error, "RDP TLS 握手失败"));
        }
    };
    if let Some(expected) = fingerprint {
        let leaf = stream
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|chain| chain.first())
            .ok_or_else(|| ApiError::new(ErrorCode::ConnectionFailed, "没有收到远端叶证书"))?;
        let actual: [u8; 32] = Sha256::digest(leaf.as_ref()).into();
        if actual != expected {
            return Err(ApiError::new(
                ErrorCode::CertificateMismatch,
                "远端证书与指定指纹不匹配。",
            ));
        }
    }
    let public_key = ironrdp_tls::extract_tls_server_public_key(&certificate)
        .ok_or_else(|| ApiError::new(ErrorCode::ConnectionFailed, "无法读取远端证书公钥"))?
        .to_vec();
    let upgraded = ironrdp_tokio::mark_as_upgraded(upgrade, &mut connector);
    let mut stream = TokioFramed::new(stream);
    let result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut stream,
        &mut ReqwestNetworkClient::new(),
        host.into(),
        public_key,
        None,
    )
    .await
    .map_err(|error| {
        ApiError::chain(ErrorCode::ConnectionFailed, &error, "RDP 登录或桌面初始化失败")
    })?;
    Ok((result, stream))
}

fn sanitized_desktop_size(width: u16, height: u16) -> DesktopSize {
    // Display Control requires even dimensions within 200..=8192; fall back to
    // the prototype default when the client sent nothing sensible.
    let clamp = |value: u16, min: u16, max: u16, fallback: u16| {
        let value = value & !1; // force even
        if (min..=max).contains(&value) { value } else { fallback }
    };
    DesktopSize {
        width: clamp(width, 200, 8192, 1280),
        height: clamp(height, 200, 8192, 800),
    }
}

fn client_config(
    username: String,
    password: String,
    domain: Option<String>,
    desktop_size: DesktopSize,
) -> Config {
    Config {
        desktop_size,
        desktop_scale_factor: 0,
        enable_tls: true,
        enable_credssp: true,
        credentials: Credentials::UsernamePassword { username, password },
        domain,
        client_build: 0,
        client_name: "AuuRemote".into(),
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0x0409,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: MajorPlatformType::WINDOWS,
        hardware_id: None,
        request_data: None,
        autologon: true,
        enable_audio_playback: true,
        performance_flags: Default::default(),
        license_cache: None,
        timezone_info: Default::default(),
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
    }
}

fn input_events(
    keys: &mut Database,
    event: Input,
    width: u16,
    height: u16,
) -> Vec<ironrdp::pdu::input::fast_path::FastPathInputEvent> {
    let position = |x: u16, y: u16| {
        Operation::MouseMove(MousePosition {
            x: x.min(width - 1),
            y: y.min(height - 1),
        })
    };
    match event {
        Input::Move { x, y } => keys.apply([position(x, y)]),
        Input::Button {
            x,
            y,
            button,
            pressed,
        } => {
            let Some(button) = MouseButton::from_web_button(button) else {
                return Vec::new();
            };
            let button = if pressed {
                Operation::MouseButtonPressed(button)
            } else {
                Operation::MouseButtonReleased(button)
            };
            keys.apply([position(x, y), button])
        }
        Input::Wheel {
            x,
            y,
            units,
            vertical,
        } => keys.apply([
            position(x, y),
            Operation::WheelRotations(WheelRotations {
                is_vertical: vertical,
                rotation_units: units.clamp(-255, 255),
            }),
        ]),
        Input::Key { scan_code, pressed } => keys.apply([key(scan_code, pressed)]),
        Input::Text { text } => keys.apply(text.chars().flat_map(|c| {
            [
                Operation::UnicodeKeyPressed(c),
                Operation::UnicodeKeyReleased(c),
            ]
        })),
        Input::RemoteRun => keys.apply([
            key(0xE05B, true),
            key(0x13, true),
            key(0x13, false),
            key(0xE05B, false),
        ]),
        Input::ReleaseAll => keys.release_all(),
        // Handled before this mapping, in the session loop; nothing to encode.
        Input::Resize { .. } => keys.apply([]),
    }
    .into_vec()
}

fn key(scan_code: u16, pressed: bool) -> Operation {
    let scan_code = Scancode::from_u16(scan_code);
    if pressed {
        Operation::KeyPressed(scan_code)
    } else {
        Operation::KeyReleased(scan_code)
    }
}
