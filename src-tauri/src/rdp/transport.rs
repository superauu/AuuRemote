use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use ironrdp::{
    connector::{ClientConnector, Config, ConnectionResult, Credentials, DesktopSize},
    graphics::image_processing::PixelFormat,
    input::{Database, MouseButton, MousePosition, Operation, Scancode, WheelRotations},
    pdu::{gcc::KeyboardType, rdp::capability_sets::MajorPlatformType},
    session::{ActiveStageBuilder, ActiveStageOutput, image::DecodedImage},
};
use ironrdp_tls::{CertificateValidationCallback, TlsStream};
use ironrdp_tokio::{FramedWrite, TokioFramed, reqwest::ReqwestNetworkClient};
use sha2::{Digest, Sha256};
use tokio::{net::TcpStream, sync::mpsc, time};

use super::{Connection, Endpoint, Frame, Input, View, format_fingerprint};
use crate::errors::{ApiError, ErrorCode};

type RdpStream = TokioFramed<TlsStream<TcpStream>>;

pub(super) async fn run(
    connection: Connection,
    fingerprint: Option<[u8; 32]>,
    mut input: mpsc::Receiver<Vec<Input>>,
    view: &Mutex<View>,
) -> Result<(), ApiError> {
    let (result, mut stream) =
        time::timeout(Duration::from_secs(25), connect(connection, fingerprint))
            .await
            .map_err(|_| ApiError::new(ErrorCode::ConnectTimeout, "RDP 连接超时"))??;
    view.lock().expect("view lock").established = true;

    let DesktopSize { width, height } = result.desktop_size;
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
    let mut keys = Database::new();
    let mut tick = time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let first_frame = time::sleep(Duration::from_secs(30));
    tokio::pin!(first_frame);
    let mut dirty = false;
    let mut revision: u32 = 0;

    loop {
        let output = tokio::select! {
            received = stream.read_pdu() => {
                let (action, bytes) = received
                    .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "读取远端画面失败"))?;
                stage.process(&mut image, action, &bytes)
                    .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "解码远端画面失败"))?
            }
            received = input.recv() => {
                let Some(batch) = received else { return Ok(()); };
                let mut output = Vec::new();
                for event in batch {
                    let events = input_events(&mut keys, event, width, height);
                    for chunk in events.chunks(64) {
                        output.extend(stage.process_fastpath_input(&mut image, chunk)
                            .map_err(|error| ApiError::chain(ErrorCode::ConnectionFailed, &error, "编码远端输入失败"))?);
                    }
                }
                output
            }
            _ = tick.tick() => {
                if dirty {
                    revision = revision.wrapping_add(1).max(1);
                    let frame = Arc::new(make_frame(&image, revision));
                    view.lock().expect("view lock").frame = Some(frame);
                    dirty = false;
                }
                continue;
            }
            _ = &mut first_frame, if revision == 0 => return Err(ApiError::new(
                ErrorCode::ConnectionFailed,
                "RDP 已建立，但未收到远端桌面画面。",
            )),
        };

        for event in output {
            match event {
                ActiveStageOutput::ResponseFrame(frame) if !frame.is_empty() => {
                    time::timeout(Duration::from_secs(10), stream.write_all(&frame))
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
                    return Err(ApiError::new(
                        ErrorCode::ConnectionFailed,
                        "远端已停用或更改桌面，请重新连接。",
                    ));
                }
                _ => {}
            }
        }
    }
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
    } = connection;
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
    let mut connector = ClientConnector::new(client_config(username, password, domain), local);
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
    let callback: CertificateValidationCallback = Arc::new(move |der, _, _| {
        let digest: [u8; 32] = Sha256::digest(der).into();
        *observed.lock().expect("certificate lock") = Some(digest);
        fingerprint == Some(digest)
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

fn client_config(username: String, password: String, domain: Option<String>) -> Config {
    Config {
        desktop_size: DesktopSize {
            width: 1280,
            height: 800,
        },
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
        enable_audio_playback: false,
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
