use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, TcpListener, TcpStream, UdpSocket},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    ServerConfig, ServerConnection, StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use tungstenite::{
    handshake::derive_accept_key,
    protocol::{frame::coding::CloseCode, CloseFrame, Message, Role},
    Error as WebSocketError, WebSocket,
};

use crate::{
    browser_server::BrowserControl,
    codex_bridge::CodexBridge,
    control,
    domain::{MobileScope, PairedDevice},
    mobile_gateway::MobileGateway,
    protocol::{
        self, HubSnapshot, HubStreamMessage, PROTOCOL_VERSION, STREAM_HEARTBEAT_INTERVAL_MS,
    },
    state::AppState,
};

pub const BOOTSTRAP_ADDRESS: &str = "127.0.0.1:43121";
pub const TLS_ADDRESS: &str = "127.0.0.1:43122";
const NETWORK_ADDRESS: &str = "0.0.0.0:43124";
const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_SECURE_BODY_BYTES: usize = 12 * 1024 * 1024;
const REALTIME_PATH: &str = "/api/v1/ws";
const REALTIME_SUBPROTOCOL: &str = "lume.hub.v1";
const REALTIME_POLL_INTERVAL: Duration = Duration::from_millis(80);
const REALTIME_READ_TIMEOUT: Duration = Duration::from_millis(5);
const MDNS_SERVICE_TYPE: &str = "_lume._tcp.local.";
const MOBILE_INDEX: &str = include_str!("../../mobile-pwa/index.html");
const MOBILE_APP: &str = include_str!("../../mobile-pwa/app.js");
const MOBILE_STYLES: &str = include_str!("../../mobile-pwa/styles.css");
const MOBILE_MANIFEST: &str = include_str!("../../mobile-pwa/manifest.webmanifest");
const MOBILE_SERVICE_WORKER: &str = include_str!("../../mobile-pwa/sw.js");
const MOBILE_ICON: &str = include_str!("../../mobile-pwa/lume-mobile-icon.svg");

trait MobileStream: Read + Write {
    fn set_mobile_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
}

impl MobileStream for TcpStream {
    fn set_mobile_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)
    }
}

impl MobileStream for StreamOwned<ServerConnection, TcpStream> {
    fn set_mobile_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.sock.set_read_timeout(timeout)
    }
}

pub struct MobileServer {
    running: Arc<AtomicBool>,
    network_running: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    discovery: Arc<Mutex<Option<MdnsAdvertisement>>>,
    status: Arc<Mutex<MobileServerStatus>>,
    state: Option<AppState>,
    gateway: Option<MobileGateway>,
    app: Option<AppHandle>,
}

impl Default for MobileServer {
    fn default() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            network_running: Arc::new(Mutex::new(None)),
            discovery: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(MobileServerStatus {
                running: false,
                address: String::new(),
                network_reachable: false,
                discovery_available: false,
                transport: "unavailable".into(),
            })),
            state: None,
            gateway: None,
            app: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileServerStatus {
    pub running: bool,
    pub address: String,
    pub network_reachable: bool,
    pub discovery_available: bool,
    pub transport: String,
}

struct MdnsAdvertisement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for MdnsAdvertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

impl MobileServer {
    pub fn start_loopback(
        state: AppState,
        gateway: MobileGateway,
        app: AppHandle,
        data_dir: &Path,
    ) -> Result<Self, String> {
        let identity = TlsIdentity::load_or_create(data_dir, &[])?;
        let listener = TcpListener::bind(TLS_ADDRESS)
            .map_err(|error| format!("Não foi possível iniciar o gateway mobile: {error}"))?;
        let bootstrap = TcpListener::bind(BOOTSTRAP_ADDRESS).map_err(|error| {
            format!("Não foi possível iniciar o instalador do certificado: {error}")
        })?;
        let running = Arc::new(AtomicBool::new(true));
        let address = format!("https://{TLS_ADDRESS}");
        gateway.set_desktop_id(identity.ca_fingerprint.clone());
        gateway.set_pairing_base_url(address.clone());
        start_listener_pair(
            listener,
            bootstrap,
            running.clone(),
            state.clone(),
            gateway.clone(),
            app.clone(),
            &identity,
            "local",
        )?;
        let status = MobileServerStatus {
            running: true,
            address,
            network_reachable: false,
            discovery_available: false,
            transport: "https".into(),
        };
        Ok(Self {
            running,
            network_running: Arc::new(Mutex::new(None)),
            discovery: Arc::new(Mutex::new(None)),
            status: Arc::new(Mutex::new(status)),
            state: Some(state),
            gateway: Some(gateway),
            app: Some(app),
        })
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    pub fn status(&self) -> MobileServerStatus {
        let mut status = self
            .status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| MobileServer::default().status());
        status.running = self.is_running();
        status
    }

    pub fn enable_network(&self) -> Result<MobileServerStatus, String> {
        if self.status().network_reachable {
            return Ok(self.status());
        }
        let state = self
            .state
            .clone()
            .ok_or_else(|| "O gateway mobile local não está disponível".to_string())?;
        let gateway = self
            .gateway
            .clone()
            .ok_or_else(|| "O pareamento mobile não está disponível".to_string())?;
        let app = self
            .app
            .clone()
            .ok_or_else(|| "O aplicativo não está disponível para comandos remotos".to_string())?;
        let ip = local_network_ip()?;
        let listener = TcpListener::bind(NETWORK_ADDRESS)
            .map_err(|error| format!("Não foi possível abrir o gateway na rede local: {error}"))?;
        let running = Arc::new(AtomicBool::new(true));
        start_plain_listener(listener, running.clone(), state, gateway.clone(), app)?;
        let address = format!("http://{ip}:43124");
        gateway.set_pairing_base_url(address.clone());
        let discovery = start_mdns_advertisement(ip, &gateway.desktop_id()).ok();
        let discovery_available = discovery.is_some();
        if let Ok(mut current) = self.discovery.lock() {
            *current = discovery;
        }
        *self
            .network_running
            .lock()
            .map_err(|_| "Não foi possível ativar o gateway mobile".to_string())? = Some(running);
        let status = MobileServerStatus {
            running: true,
            address,
            network_reachable: true,
            discovery_available,
            transport: "encrypted-local".into(),
        };
        *self
            .status
            .lock()
            .map_err(|_| "Não foi possível atualizar o gateway mobile".to_string())? =
            status.clone();
        Ok(status)
    }

    pub fn disable_network(&self) -> Result<MobileServerStatus, String> {
        if let Some(running) = self
            .network_running
            .lock()
            .map_err(|_| "Não foi possível desativar o gateway mobile".to_string())?
            .take()
        {
            running.store(false, Ordering::Relaxed);
        }
        if let Some(gateway) = &self.gateway {
            gateway.set_pairing_base_url(format!("https://{TLS_ADDRESS}"));
        }
        if let Ok(mut discovery) = self.discovery.lock() {
            discovery.take();
        }
        let status = MobileServerStatus {
            running: self.is_running(),
            address: format!("https://{TLS_ADDRESS}"),
            network_reachable: false,
            discovery_available: false,
            transport: "https".into(),
        };
        *self
            .status
            .lock()
            .map_err(|_| "Não foi possível atualizar o gateway mobile".to_string())? =
            status.clone();
        Ok(status)
    }
}

impl Drop for MobileServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Ok(mut network) = self.network_running.lock() {
            if let Some(running) = network.take() {
                running.store(false, Ordering::Relaxed);
            }
        }
        if let Ok(mut discovery) = self.discovery.lock() {
            discovery.take();
        }
    }
}

fn start_listener_pair(
    listener: TcpListener,
    bootstrap: TcpListener,
    running: Arc<AtomicBool>,
    state: AppState,
    gateway: MobileGateway,
    app: AppHandle,
    identity: &TlsIdentity,
    label: &'static str,
) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    bootstrap
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let thread_running = running.clone();
    let tls_config = identity.server_config.clone();
    thread::Builder::new()
        .name(format!("lume-mobile-{label}-gateway"))
        .spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
                        let state = state.clone();
                        let gateway = gateway.clone();
                        let app = app.clone();
                        let tls_config = tls_config.clone();
                        let connection_running = thread_running.clone();
                        let _ = thread::Builder::new()
                            .name("lume-mobile-client".into())
                            .spawn(move || {
                                handle_tls(
                                    stream,
                                    state,
                                    gateway,
                                    app,
                                    tls_config,
                                    connection_running,
                                )
                            });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(40));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(80)),
                }
            }
        })
        .map_err(|error| error.to_string())?;
    let bootstrap_running = running;
    let ca_pem = Arc::new(identity.ca_pem.as_bytes().to_vec());
    thread::Builder::new()
        .name(format!("lume-mobile-{label}-bootstrap"))
        .spawn(move || {
            while bootstrap_running.load(Ordering::Relaxed) {
                match bootstrap.accept() {
                    Ok((stream, _)) => {
                        let ca_pem = ca_pem.clone();
                        let _ = thread::Builder::new()
                            .name("lume-mobile-bootstrap-client".into())
                            .spawn(move || handle_bootstrap(stream, &ca_pem));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(40));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(80)),
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn start_plain_listener(
    listener: TcpListener,
    running: Arc<AtomicBool>,
    state: AppState,
    gateway: MobileGateway,
    app: AppHandle,
) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    thread::Builder::new()
        .name("lume-mobile-network-gateway".into())
        .spawn(move || {
            while running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
                        let state = state.clone();
                        let gateway = gateway.clone();
                        let app = app.clone();
                        let connection_running = running.clone();
                        let _ = thread::Builder::new()
                            .name("lume-mobile-network-client".into())
                            .spawn(move || {
                                handle_stream(stream, state, gateway, app, connection_running)
                            });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(40));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(80)),
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn start_mdns_advertisement(ip: IpAddr, desktop_id: &str) -> Result<MdnsAdvertisement, String> {
    let info = mdns_service_info(ip, desktop_id)?;
    let fullname = info.get_fullname().to_string();
    let daemon = ServiceDaemon::new().map_err(|error| error.to_string())?;
    daemon.register(info).map_err(|error| error.to_string())?;
    Ok(MdnsAdvertisement { daemon, fullname })
}

fn mdns_service_info(ip: IpAddr, desktop_id: &str) -> Result<ServiceInfo, String> {
    let stable_suffix = desktop_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    if stable_suffix.len() < 8 {
        return Err("Identidade do desktop indisponível para descoberta local".into());
    }
    let instance = format!("Lume-{stable_suffix}");
    let hostname = format!("lume-{}.local.", stable_suffix.to_ascii_lowercase());
    let properties = HashMap::from([
        ("id".to_string(), desktop_id.to_string()),
        ("protocol".to_string(), PROTOCOL_VERSION.to_string()),
        ("transport".to_string(), "encrypted-local".to_string()),
        ("realtime".to_string(), "1".to_string()),
    ]);
    ServiceInfo::new(
        MDNS_SERVICE_TYPE,
        &instance,
        &hostname,
        ip,
        43124,
        properties,
    )
    .map_err(|error| error.to_string())
}

fn local_network_ip() -> Result<IpAddr, String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|error| format!("Não foi possível consultar a rede local: {error}"))?;
    socket
        .connect("1.1.1.1:80")
        .map_err(|_| "Conecte o computador e o telefone à mesma rede local".to_string())?;
    let ip = socket.local_addr().map_err(|error| error.to_string())?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        return Err("Nenhum endereço de rede local foi encontrado".into());
    }
    Ok(ip)
}

struct TlsIdentity {
    server_config: Arc<ServerConfig>,
    ca_pem: String,
    ca_fingerprint: String,
}

impl TlsIdentity {
    fn load_or_create(data_dir: &Path, additional_hosts: &[String]) -> Result<Self, String> {
        let directory = data_dir.join("mobile");
        fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        let ca_der_path = directory.join("lume-ca.der");
        let ca_pem_path = directory.join("lume-ca.crt");
        let ca_key_path = directory.join("lume-ca-key.der");
        let server_der_path = directory.join("lume-server.der");
        let server_key_path = directory.join("lume-server-key.der");
        let persisted_ca = (
            fs::read(&ca_der_path),
            fs::read_to_string(&ca_pem_path),
            fs::read(&ca_key_path),
        );
        let (ca_der, ca_pem, ca_key_der) =
            if let (Ok(ca_der), Ok(ca_pem), Ok(ca_key_der)) = persisted_ca {
                (ca_der, ca_pem, ca_key_der)
            } else {
                let mut ca_params = CertificateParams::new(Vec::<String>::new())
                    .map_err(|error| error.to_string())?;
                ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
                ca_params
                    .distinguished_name
                    .push(DnType::OrganizationName, "Lume Local");
                ca_params
                    .distinguished_name
                    .push(DnType::CommonName, "Lume Local CA");
                ca_params.key_usages = vec![
                    KeyUsagePurpose::DigitalSignature,
                    KeyUsagePurpose::KeyCertSign,
                    KeyUsagePurpose::CrlSign,
                ];
                let ca_key = KeyPair::generate().map_err(|error| error.to_string())?;
                let ca_key_der = ca_key.serialize_der();
                let ca_cert = ca_params
                    .self_signed(&ca_key)
                    .map_err(|error| error.to_string())?;
                let ca_der = ca_cert.der().to_vec();
                let ca_pem = ca_cert.pem();
                fs::write(&ca_der_path, &ca_der).map_err(|error| error.to_string())?;
                fs::write(&ca_pem_path, &ca_pem).map_err(|error| error.to_string())?;
                write_private_key(&ca_key_path, &ca_key_der)?;
                (ca_der, ca_pem, ca_key_der)
            };

        let ca_key = KeyPair::try_from(ca_key_der.as_slice()).map_err(|error| error.to_string())?;
        let ca_certificate = CertificateDer::from(ca_der.clone());
        let ca =
            Issuer::from_ca_cert_der(&ca_certificate, ca_key).map_err(|error| error.to_string())?;
        let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        for host in additional_hosts {
            if !hosts.contains(host) {
                hosts.push(host.clone());
            }
        }
        let mut server_params = CertificateParams::new(hosts).map_err(|error| error.to_string())?;
        server_params
            .distinguished_name
            .push(DnType::OrganizationName, "Lume Local");
        server_params
            .distinguished_name
            .push(DnType::CommonName, "Lume Mobile Gateway");
        server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        server_params.use_authority_key_identifier_extension = true;
        let server_key = KeyPair::generate().map_err(|error| error.to_string())?;
        let server_cert = server_params
            .signed_by(&server_key, &ca)
            .map_err(|error| error.to_string())?;

        let server_der = server_cert.der().to_vec();
        let server_key = server_key.serialize_der();
        fs::write(&server_der_path, &server_der).map_err(|error| error.to_string())?;
        write_private_key(&server_key_path, &server_key)?;
        Self::from_der(ca_der, ca_pem, server_der, server_key)
    }

    fn from_der(
        ca_der: Vec<u8>,
        ca_pem: String,
        server_der: Vec<u8>,
        server_key: Vec<u8>,
    ) -> Result<Self, String> {
        let ca_fingerprint = Sha256::digest(&ca_der)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![
                    CertificateDer::from(server_der),
                    CertificateDer::from(ca_der),
                ],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key)),
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            server_config: Arc::new(server_config),
            ca_pem,
            ca_fingerprint,
        })
    }
}

fn write_private_key(path: &Path, value: &[u8]) -> Result<(), String> {
    fs::write(path, value).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRequest {
    code: String,
    device_name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct EncryptedEnvelope {
    nonce: String,
    ciphertext: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecurePairPayload {
    device_name: String,
    timestamp: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureTransportRequest {
    device_id: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureRequestPayload {
    method: String,
    path: String,
    body: serde_json::Value,
    timestamp: i64,
    request_nonce: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SecureResponsePayload {
    status: u16,
    body: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureStreamAuthRequest {
    device_id: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SecureStreamAuthPayload {
    timestamp: i64,
    request_nonce: String,
    stream_nonce: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SecureStreamMessage {
    #[serde(rename = "type")]
    message_type: &'static str,
    sequence: u64,
    nonce: String,
    ciphertext: String,
}

fn handle_tls(
    stream: TcpStream,
    state: AppState,
    gateway: MobileGateway,
    app: AppHandle,
    config: Arc<ServerConfig>,
    running: Arc<AtomicBool>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
    let Ok(connection) = ServerConnection::new(config) else {
        return;
    };
    handle_stream(
        StreamOwned::new(connection, stream),
        state,
        gateway,
        app,
        running,
    );
}

fn handle_stream<S: MobileStream>(
    mut stream: S,
    state: AppState,
    gateway: MobileGateway,
    app: AppHandle,
    running: Arc<AtomicBool>,
) {
    match read_request(&mut stream) {
        Ok(request) if request.path.split('?').next() == Some(REALTIME_PATH) => {
            handle_realtime_stream(stream, request, state, gateway, running);
        }
        Ok(request) => {
            let response = route_with_app(request, &state, &gateway, Some(&app));
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
        Err(message) => {
            let response = json_error(400, "bad_request", &message);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    }
}

fn handle_realtime_stream<S: MobileStream>(
    mut stream: S,
    request: HttpRequest,
    state: AppState,
    gateway: MobileGateway,
    running: Arc<AtomicBool>,
) {
    let response = match websocket_upgrade_response(&request) {
        Ok(response) => response,
        Err(message) => {
            let response = json_error(400, "websocket_upgrade_failed", &message);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            return;
        }
    };
    if stream.write_all(response.as_bytes()).is_err() || stream.flush().is_err() {
        return;
    }

    let mut socket = WebSocket::from_raw_socket(stream, Role::Server, None);
    let (key, stream_nonce) = match authenticate_realtime_stream(&mut socket, &state, &gateway) {
        Ok(authenticated) => authenticated,
        Err(_) => {
            let _ = socket.close(Some(CloseFrame {
                code: CloseCode::Policy,
                reason: "authentication_failed".into(),
            }));
            return;
        }
    };

    let mut message_sequence = 0_u64;
    if send_secure_stream_message(
        &mut socket,
        &key,
        &stream_nonce,
        &mut message_sequence,
        &HubStreamMessage::hello(),
    )
    .is_err()
    {
        return;
    }

    let mut event_sequence = protocol::latest_event_sequence();
    let snapshot = match state.sessions() {
        Ok(sessions) => HubSnapshot::new(sessions),
        Err(message) => {
            let _ = send_secure_stream_message(
                &mut socket,
                &key,
                &stream_nonce,
                &mut message_sequence,
                &HubStreamMessage::Error {
                    code: "snapshot_failed".into(),
                    message,
                    retryable: true,
                },
            );
            let _ = socket.close(None);
            return;
        }
    };
    if send_secure_stream_message(
        &mut socket,
        &key,
        &stream_nonce,
        &mut message_sequence,
        &HubStreamMessage::Snapshot {
            sequence: event_sequence,
            snapshot,
        },
    )
    .is_err()
    {
        return;
    }

    let _ = socket
        .get_mut()
        .set_mobile_read_timeout(Some(REALTIME_READ_TIMEOUT));
    let mut last_heartbeat = Instant::now();
    while running.load(Ordering::Relaxed) {
        match read_realtime_control(&mut socket) {
            Ok(true) => {}
            Ok(false) | Err(_) => break,
        }
        let events = protocol::events_since(event_sequence);
        if !events.is_empty() {
            let snapshot = match state.sessions() {
                Ok(sessions) => HubSnapshot::new(sessions),
                Err(message) => {
                    if send_secure_stream_message(
                        &mut socket,
                        &key,
                        &stream_nonce,
                        &mut message_sequence,
                        &HubStreamMessage::Error {
                            code: "snapshot_failed".into(),
                            message,
                            retryable: true,
                        },
                    )
                    .is_err()
                    {
                        break;
                    }
                    thread::sleep(REALTIME_POLL_INTERVAL);
                    continue;
                }
            };
            let latest = events
                .last()
                .map(|event| event.sequence)
                .unwrap_or(event_sequence);
            if send_secure_stream_message(
                &mut socket,
                &key,
                &stream_nonce,
                &mut message_sequence,
                &HubStreamMessage::Update { events, snapshot },
            )
            .is_err()
            {
                break;
            }
            event_sequence = latest;
        }
        if last_heartbeat.elapsed() >= Duration::from_millis(STREAM_HEARTBEAT_INTERVAL_MS) {
            if socket.send(Message::Ping(Vec::new().into())).is_err() {
                break;
            }
            last_heartbeat = Instant::now();
        }
        thread::sleep(REALTIME_POLL_INTERVAL);
    }
    let _ = socket.close(None);
}

fn read_realtime_control<S: Read + Write>(socket: &mut WebSocket<S>) -> Result<bool, String> {
    match socket.read() {
        Ok(Message::Ping(value)) => socket
            .send(Message::Pong(value))
            .map(|_| true)
            .map_err(|error| error.to_string()),
        Ok(Message::Close(_)) => Ok(false),
        Ok(_) => Ok(true),
        Err(WebSocketError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(true)
        }
        Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn websocket_upgrade_response(request: &HttpRequest) -> Result<String, String> {
    if request.method != "GET" {
        return Err("O canal em tempo real requer GET".into());
    }
    if let Some(origin) = request.headers.get("origin") {
        if !is_mobile_origin(origin) {
            return Err("Origem não autorizada".into());
        }
    }
    if !header_contains_token(&request.headers, "connection", "upgrade")
        || !request
            .headers
            .get("upgrade")
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    {
        return Err("Upgrade WebSocket ausente".into());
    }
    if request
        .headers
        .get("sec-websocket-version")
        .is_none_or(|version| version != "13")
    {
        return Err("Versão WebSocket não suportada".into());
    }
    if !header_contains_token(
        &request.headers,
        "sec-websocket-protocol",
        REALTIME_SUBPROTOCOL,
    ) {
        return Err("Subprotocolo do Lume ausente".into());
    }
    let key = request
        .headers
        .get("sec-websocket-key")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Chave WebSocket ausente".to_string())?;
    Ok(format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\
         Sec-WebSocket-Protocol: {REALTIME_SUBPROTOCOL}\r\n\r\n",
        derive_accept_key(key.as_bytes())
    ))
}

fn header_contains_token(headers: &HashMap<String, String>, name: &str, expected: &str) -> bool {
    headers.get(name).is_some_and(|value| {
        value
            .split(',')
            .map(str::trim)
            .any(|token| token.eq_ignore_ascii_case(expected))
    })
}

fn authenticate_realtime_stream<S: Read + Write>(
    socket: &mut WebSocket<S>,
    state: &AppState,
    gateway: &MobileGateway,
) -> Result<([u8; 32], String), String> {
    let message = loop {
        match socket.read().map_err(|error| error.to_string())? {
            Message::Text(value) => break value,
            Message::Ping(value) => {
                socket
                    .send(Message::Pong(value))
                    .map_err(|error| error.to_string())?;
            }
            Message::Close(_) => return Err("Conexão encerrada antes da autenticação".into()),
            _ => {}
        }
    };
    let request = serde_json::from_str::<SecureStreamAuthRequest>(&message)
        .map_err(|_| "Autenticação do canal inválida".to_string())?;
    let (device, key) = gateway
        .authenticate_encrypted(state, &request.device_id)?
        .ok_or_else(|| "Dispositivo não pareado".to_string())?;
    if !device.scopes.contains(&MobileScope::Monitor) {
        return Err("Monitoramento não autorizado".into());
    }
    let payload = decrypt_json::<SecureStreamAuthPayload>(
        &key,
        &EncryptedEnvelope {
            nonce: request.nonce,
            ciphertext: request.ciphertext,
        },
        b"lume-stream-auth-v1",
    )
    .map_err(|_| "Autenticação do canal inválida".to_string())?;
    if !timestamp_is_recent(payload.timestamp)
        || payload.stream_nonce.len() < 16
        || payload.stream_nonce.len() > 128
    {
        return Err("Autenticação do canal expirada".into());
    }
    if !gateway.accept_request_nonce(
        &request.device_id,
        &payload.request_nonce,
        payload.timestamp,
    )? {
        return Err("Autenticação do canal expirada ou repetida".into());
    }
    Ok((key, payload.stream_nonce))
}

fn send_secure_stream_message<S: Read + Write>(
    socket: &mut WebSocket<S>,
    key: &[u8; 32],
    stream_nonce: &str,
    sequence: &mut u64,
    message: &HubStreamMessage,
) -> Result<(), String> {
    *sequence = sequence.saturating_add(1);
    let aad = format!("lume-stream-message-v1:{stream_nonce}:{sequence}");
    let encrypted = encrypt_json(key, message, aad.as_bytes())?;
    let body = serde_json::to_string(&SecureStreamMessage {
        message_type: "secure_message",
        sequence: *sequence,
        nonce: encrypted.nonce,
        ciphertext: encrypted.ciphertext,
    })
    .map_err(|error| error.to_string())?;
    socket
        .send(Message::Text(body.into()))
        .map_err(|error| error.to_string())
}

fn handle_bootstrap(mut stream: TcpStream, ca_pem: &[u8]) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(4)));
    let response = match read_request(&mut stream) {
        Ok(request) if request.method == "GET" && request.path == "/lume-ca.crt" => {
            raw_response(200, "application/x-pem-file", ca_pem)
        }
        Ok(request) if request.method == "GET" && request.path == "/api/v1/health" => {
            json_response(
                200,
                &serde_json::json!({
                    "ok": true,
                    "protocolVersion": PROTOCOL_VERSION,
                    "purpose": "certificate_bootstrap",
                    "realtimePath": REALTIME_PATH,
                }),
            )
        }
        Ok(_) => json_error(404, "not_found", "Rota não encontrada"),
        Err(message) => json_error(400, "bad_request", &message),
    };
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
fn route(request: HttpRequest, state: &AppState, gateway: &MobileGateway) -> String {
    route_with_app(request, state, gateway, None)
}

fn route_with_app(
    request: HttpRequest,
    state: &AppState,
    gateway: &MobileGateway,
    app: Option<&AppHandle>,
) -> String {
    let allowed_origin = request
        .headers
        .get("origin")
        .filter(|origin| is_mobile_origin(origin))
        .cloned();
    let response = route_core(request, state, gateway, app, None);
    if let Some(origin) = allowed_origin {
        with_cors(response, &origin)
    } else {
        response
    }
}

fn route_core(
    request: HttpRequest,
    state: &AppState,
    gateway: &MobileGateway,
    app: Option<&AppHandle>,
    authenticated_device: Option<PairedDevice>,
) -> String {
    if request.method == "OPTIONS" {
        return empty_response(204, "No Content");
    }
    let path = request.path.split('?').next().unwrap_or(&request.path);
    if request.method == "GET" {
        let asset = match path {
            "/" | "/pair" => Some(("text/html; charset=utf-8", MOBILE_INDEX)),
            "/app.js" => Some(("text/javascript; charset=utf-8", MOBILE_APP)),
            "/styles.css" => Some(("text/css; charset=utf-8", MOBILE_STYLES)),
            "/manifest.webmanifest" => {
                Some(("application/manifest+json; charset=utf-8", MOBILE_MANIFEST))
            }
            "/sw.js" => Some(("text/javascript; charset=utf-8", MOBILE_SERVICE_WORKER)),
            "/lume-mobile-icon.svg" => Some(("image/svg+xml; charset=utf-8", MOBILE_ICON)),
            _ => None,
        };
        if let Some((content_type, body)) = asset {
            return static_response(content_type, body, path == "/sw.js");
        }
    }
    if request.method == "GET" && request.path == "/api/v1/health" {
        return json_response(
            200,
            &serde_json::json!({
                "ok": true,
                "protocolVersion": PROTOCOL_VERSION,
                "transport": "encrypted-local",
                "realtimePath": REALTIME_PATH,
                "realtimeEncrypted": true,
            }),
        );
    }
    if request.method == "POST" && request.path == "/api/v1/pair" {
        let pairing = serde_json::from_slice::<PairRequest>(&request.body)
            .map_err(|error| error.to_string())
            .and_then(|pairing| {
                gateway.complete_pairing(state, &pairing.code, &pairing.device_name)
            });
        return match pairing {
            Ok(credentials) => json_response(201, &credentials),
            Err(message) => json_error(401, "pairing_failed", &message),
        };
    }
    if request.method == "POST" && request.path == "/api/v1/pair-secure" {
        return secure_pairing_response(&request.body, state, gateway);
    }
    if request.method == "POST" && request.path == "/api/v1/secure" {
        return secure_transport_response(&request.body, state, gateway, app);
    }

    let device = match authenticated_device {
        Some(device) => device,
        None => {
            let Some(token) = bearer_token(&request.headers) else {
                return json_error(401, "authentication_required", "Token ausente");
            };
            match gateway.authenticate(state, token) {
                Ok(Some(device)) => device,
                Ok(None) => return json_error(401, "invalid_token", "Token inválido"),
                Err(message) => return json_error(500, "authentication_failed", &message),
            }
        }
    };
    if !device.scopes.contains(&MobileScope::Monitor) {
        return json_error(
            403,
            "scope_required",
            "Acesso de monitoramento não autorizado",
        );
    }

    if request.method == "GET" && request.path == "/api/v1/me" {
        return json_response(200, &device);
    }

    if request.method == "POST" && request.path == "/api/v1/commands" {
        let command = match serde_json::from_slice::<protocol::HubCommandRequest>(&request.body) {
            Ok(command) => command,
            Err(error) => return json_error(400, "invalid_command", &error.to_string()),
        };
        let required_scope = match &command.command {
            protocol::HubCommand::SubmitPrompt { .. } => MobileScope::Prompt,
            protocol::HubCommand::ResolvePermission { .. } => MobileScope::Approve,
            protocol::HubCommand::TerminateSession { .. } => MobileScope::Terminate,
            protocol::HubCommand::OpenSessionSource { .. } => {
                return json_error(
                    403,
                    "desktop_only",
                    "Abrir a origem só está disponível no computador",
                )
            }
            protocol::HubCommand::RefreshRateLimits { .. } => MobileScope::Monitor,
        };
        if !device.scopes.contains(&required_scope) {
            return json_error(
                403,
                "scope_required",
                "Este dispositivo não possui essa permissão",
            );
        }
        let Some(app) = app else {
            return json_error(
                503,
                "desktop_unavailable",
                "O aplicativo não está disponível",
            );
        };
        let response = control::execute_hub_command(
            app,
            state,
            app.state::<CodexBridge>().inner(),
            app.state::<BrowserControl>().inner(),
            command,
        );
        return json_response(if response.ok { 200 } else { 400 }, &response);
    }

    if request.method == "GET" && request.path == "/api/v1/snapshot" {
        return match state.sessions() {
            Ok(sessions) => json_response(200, &HubSnapshot::new(sessions)),
            Err(message) => json_error(500, "snapshot_failed", &message),
        };
    }
    if request.method == "GET" && request.path.starts_with("/api/v1/events") {
        let sequence = query_value(&request.path, "since")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        return json_response(
            200,
            &serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "events": protocol::events_since(sequence),
            }),
        );
    }
    json_error(404, "not_found", "Rota não encontrada")
}

fn secure_pairing_response(body: &[u8], state: &AppState, gateway: &MobileGateway) -> String {
    let envelope = match serde_json::from_slice::<EncryptedEnvelope>(body) {
        Ok(envelope) => envelope,
        Err(error) => return json_error(400, "invalid_envelope", &error.to_string()),
    };
    let key = match gateway.active_pairing_key() {
        Ok(key) => key,
        Err(message) => return json_error(401, "pairing_failed", &message),
    };
    let pairing = match decrypt_json::<SecurePairPayload>(&key, &envelope, b"lume-pair-request-v1")
    {
        Ok(pairing) if timestamp_is_recent(pairing.timestamp) => pairing,
        Ok(_) => return json_error(401, "pairing_expired", "Solicitação de pareamento expirada"),
        Err(_) => {
            return json_error(
                401,
                "pairing_failed",
                "O QR Code não corresponde ao pareamento ativo",
            )
        }
    };
    let credentials = match gateway.complete_pairing_with_key(state, &key, &pairing.device_name) {
        Ok(credentials) => credentials,
        Err(message) => return json_error(401, "pairing_failed", &message),
    };
    match encrypt_json(&key, &credentials, b"lume-pair-response-v1") {
        Ok(envelope) => json_response(201, &envelope),
        Err(message) => json_error(500, "encryption_failed", &message),
    }
}

fn secure_transport_response(
    body: &[u8],
    state: &AppState,
    gateway: &MobileGateway,
    app: Option<&AppHandle>,
) -> String {
    let request = match serde_json::from_slice::<SecureTransportRequest>(body) {
        Ok(request) => request,
        Err(error) => return json_error(400, "invalid_envelope", &error.to_string()),
    };
    let (device, key) = match gateway.authenticate_encrypted(state, &request.device_id) {
        Ok(Some(value)) => value,
        Ok(None) => return json_error(401, "invalid_device", "Dispositivo não pareado"),
        Err(message) => return json_error(500, "authentication_failed", &message),
    };
    let envelope = EncryptedEnvelope {
        nonce: request.nonce,
        ciphertext: request.ciphertext,
    };
    let payload =
        match decrypt_json::<SecureRequestPayload>(&key, &envelope, b"lume-secure-request-v1") {
            Ok(payload) => payload,
            Err(_) => return json_error(401, "invalid_envelope", "Solicitação inválida"),
        };
    if !matches!(payload.method.as_str(), "GET" | "POST")
        || !payload.path.starts_with("/api/v1/")
        || matches!(
            payload.path.split('?').next(),
            Some("/api/v1/pair" | "/api/v1/pair-secure" | "/api/v1/secure")
        )
    {
        return json_error(400, "invalid_request", "Rota segura inválida");
    }
    match gateway.accept_request_nonce(
        &request.device_id,
        &payload.request_nonce,
        payload.timestamp,
    ) {
        Ok(true) => {}
        Ok(false) => return json_error(401, "expired_request", "Solicitação expirada ou repetida"),
        Err(message) => return json_error(500, "authentication_failed", &message),
    }
    let inner_body = if payload.method == "POST" {
        match serde_json::to_vec(&payload.body) {
            Ok(body) if body.len() <= MAX_BODY_BYTES => body,
            Ok(_) => return json_error(413, "request_too_large", "Solicitação muito grande"),
            Err(error) => return json_error(400, "invalid_request", &error.to_string()),
        }
    } else {
        Vec::new()
    };
    let response = route_core(
        HttpRequest {
            method: payload.method,
            path: payload.path,
            headers: HashMap::new(),
            body: inner_body,
        },
        state,
        gateway,
        app,
        Some(device),
    );
    let (status, response_body) = response_json_parts(&response);
    let response_aad = format!("lume-secure-response-v1:{}", payload.request_nonce);
    match encrypt_json(
        &key,
        &SecureResponsePayload {
            status,
            body: response_body,
        },
        response_aad.as_bytes(),
    ) {
        Ok(envelope) => json_response(200, &envelope),
        Err(message) => json_error(500, "encryption_failed", &message),
    }
}

fn timestamp_is_recent(timestamp: i64) -> bool {
    crate::state::now_millis()
        .checked_sub(timestamp)
        .and_then(i64::checked_abs)
        .is_some_and(|age| age <= 60 * 1_000)
}

fn decrypt_json<T: for<'de> Deserialize<'de>>(
    key: &[u8; 32],
    envelope: &EncryptedEnvelope,
    aad: &[u8],
) -> Result<T, String> {
    let nonce_bytes = URL_SAFE_NO_PAD
        .decode(&envelope.nonce)
        .map_err(|_| "Nonce inválido".to_string())?;
    let nonce_bytes: [u8; 12] = nonce_bytes
        .try_into()
        .map_err(|_| "Nonce inválido".to_string())?;
    let mut ciphertext = URL_SAFE_NO_PAD
        .decode(&envelope.ciphertext)
        .map_err(|_| "Conteúdo cifrado inválido".to_string())?;
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| "Chave inválida".to_string())?,
    );
    let plaintext = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce_bytes),
            Aad::from(aad),
            &mut ciphertext,
        )
        .map_err(|_| "Não foi possível decifrar a solicitação".to_string())?;
    serde_json::from_slice(plaintext).map_err(|error| error.to_string())
}

fn encrypt_json(
    key: &[u8; 32],
    value: &impl Serialize,
    aad: &[u8],
) -> Result<EncryptedEnvelope, String> {
    let mut nonce_bytes = [0_u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|error| error.to_string())?;
    let mut ciphertext = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    let key = LessSafeKey::new(
        UnboundKey::new(&AES_256_GCM, key).map_err(|_| "Chave inválida".to_string())?,
    );
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce_bytes),
        Aad::from(aad),
        &mut ciphertext,
    )
    .map_err(|_| "Não foi possível cifrar a resposta".to_string())?;
    Ok(EncryptedEnvelope {
        nonce: URL_SAFE_NO_PAD.encode(nonce_bytes),
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
    })
}

fn response_json_parts(response: &str) -> (u16, serde_json::Value) {
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(500);
    let body = response
        .split_once("\r\n\r\n")
        .and_then(|(_, body)| serde_json::from_str(body).ok())
        .unwrap_or(serde_json::Value::Null);
    (status, body)
}

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut impl Read) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| error.to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("Método ausente")?.to_string();
    let path = parts.next().ok_or("Caminho ausente")?.to_string();
    if !matches!(method.as_str(), "GET" | "POST" | "OPTIONS") {
        return Err("Método não permitido".into());
    }
    let mut content_length = 0usize;
    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|error| error.to_string())?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "content-length" {
            content_length = value.parse().map_err(|_| "Tamanho inválido")?;
        }
        headers.insert(name, value);
    }
    let max_body_bytes = if path == "/api/v1/secure" {
        MAX_SECURE_BODY_BYTES
    } else {
        MAX_BODY_BYTES
    };
    if content_length > max_body_bytes {
        return Err("Requisição excede o limite permitido".into());
    }
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn bearer_token(headers: &HashMap<String, String>) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn query_value<'a>(path: &'a str, name: &str) -> Option<&'a str> {
    path.split_once('?')?.1.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn json_response(status: u16, body: &impl serde::Serialize) -> String {
    let body = serde_json::to_string(body)
        .unwrap_or_else(|_| r#"{"ok":false,"error":{"code":"serialization_failed"}}"#.into());
    let label = match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    format!(
        "HTTP/1.1 {status} {label}\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'none'\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn raw_response(status: u16, content_type: &str, body: &[u8]) -> String {
    let label = if status == 200 {
        "OK"
    } else {
        "Internal Server Error"
    };
    format!(
        "HTTP/1.1 {status} {label}\r\n\
         Content-Type: {content_type}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\r\n{}",
        body.len(),
        String::from_utf8_lossy(body)
    )
}

fn static_response(content_type: &str, body: &str, service_worker: bool) -> String {
    let worker_header = if service_worker {
        "Service-Worker-Allowed: /\r\n"
    } else {
        ""
    };
    format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {content_type}\r\n\
         Cache-Control: no-cache\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\n\
         Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; manifest-src 'self'; base-uri 'none'; frame-ancestors 'none'\r\n\
         {worker_header}Content-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn json_error(status: u16, code: &str, message: &str) -> String {
    json_response(
        status,
        &serde_json::json!({
            "ok": false,
            "error": { "code": code, "message": message },
        }),
    )
}

fn empty_response(status: u16, label: &str) -> String {
    format!(
        "HTTP/1.1 {status} {label}\r\n\
         Cache-Control: no-store\r\n\
         Content-Length: 0\r\n\
         Connection: close\r\n\r\n"
    )
}

fn is_mobile_origin(origin: &str) -> bool {
    matches!(
        origin,
        "capacitor://localhost"
            | "https://localhost"
            | "https://tulerws.github.io"
            | "http://localhost:4173"
            | "http://127.0.0.1:4173"
    )
}

fn with_cors(response: String, origin: &str) -> String {
    response.replacen(
        "Content-Length:",
        &format!(
            "Access-Control-Allow-Origin: {origin}\r\n\
             Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
             Access-Control-Allow-Headers: Authorization, Content-Type\r\n\
             Access-Control-Allow-Private-Network: true\r\n\
             Vary: Origin\r\n\
             Content-Length:"
        ),
        1,
    )
}

#[cfg(test)]
mod tests {
    use std::{
        net::TcpListener,
        path::Path,
        sync::Arc,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use tungstenite::{client::IntoClientRequest, connect, http::HeaderValue, Message};

    fn request(
        method: &str,
        path: &str,
        token: Option<&str>,
        body: serde_json::Value,
    ) -> HttpRequest {
        let mut headers = HashMap::new();
        if let Some(token) = token {
            headers.insert("authorization".into(), format!("Bearer {token}"));
        }
        HttpRequest {
            method: method.into(),
            path: path.into(),
            headers,
            body: serde_json::to_vec(&body).expect("body"),
        }
    }

    fn websocket_request(origin: Option<&str>) -> HttpRequest {
        let mut headers = HashMap::from([
            ("connection".into(), "keep-alive, Upgrade".into()),
            ("upgrade".into(), "websocket".into()),
            ("sec-websocket-version".into(), "13".into()),
            (
                "sec-websocket-key".into(),
                "dGhlIHNhbXBsZSBub25jZQ==".into(),
            ),
            ("sec-websocket-protocol".into(), REALTIME_SUBPROTOCOL.into()),
        ]);
        if let Some(origin) = origin {
            headers.insert("origin".into(), origin.into());
        }
        HttpRequest {
            method: "GET".into(),
            path: REALTIME_PATH.into(),
            headers,
            body: Vec::new(),
        }
    }

    #[test]
    fn realtime_upgrade_requires_the_lume_subprotocol_and_trusted_origin() {
        let response =
            websocket_upgrade_response(&websocket_request(Some("capacitor://localhost")))
                .expect("upgrade");
        assert!(response.starts_with("HTTP/1.1 101"));
        assert!(response.contains("Sec-WebSocket-Protocol: lume.hub.v1"));
        assert!(response.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

        let mut missing_protocol = websocket_request(None);
        missing_protocol.headers.remove("sec-websocket-protocol");
        assert!(websocket_upgrade_response(&missing_protocol).is_err());
        assert!(
            websocket_upgrade_response(&websocket_request(Some("https://example.com"))).is_err()
        );
    }

    #[test]
    fn mdns_advertisement_is_bound_to_the_persistent_desktop_identity() {
        let desktop_id = "0123456789abcdef0123456789abcdef";
        let info =
            mdns_service_info("192.168.1.50".parse().expect("ip"), desktop_id).expect("serviço");

        assert_eq!(info.get_type(), MDNS_SERVICE_TYPE);
        assert_eq!(info.get_port(), 43124);
        assert!(info.get_fullname().starts_with("Lume-0123456789ab."));
        assert_eq!(
            info.get_properties().get_property_val_str("id"),
            Some(desktop_id)
        );
    }

    #[test]
    fn realtime_messages_are_bound_to_the_stream_and_sequence() {
        let key: [u8; 32] = Sha256::digest(b"stream-test").into();
        let stream_nonce = "a-unique-stream-nonce";
        let aad = format!("lume-stream-message-v1:{stream_nonce}:1");
        let envelope = encrypt_json(&key, &HubStreamMessage::hello(), aad.as_bytes())
            .expect("mensagem cifrada");
        let decoded: serde_json::Value =
            decrypt_json(&key, &envelope, aad.as_bytes()).expect("mensagem decifrada");
        assert_eq!(decoded["type"], "hello");
        assert!(decrypt_json::<serde_json::Value>(
            &key,
            &envelope,
            b"lume-stream-message-v1:another-stream:1"
        )
        .is_err());
    }

    #[test]
    fn realtime_channel_authenticates_and_delivers_a_snapshot() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        gateway.set_pairing_base_url("http://127.0.0.1:43124".into());
        let offer = gateway.begin_pairing().expect("oferta");
        let credentials = gateway
            .complete_pairing(&state, &offer.code, "Telefone")
            .expect("pareamento");
        let transport_key: [u8; 32] = Sha256::digest(credentials.token.as_bytes()).into();

        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("endereço");
        let running = Arc::new(AtomicBool::new(true));
        let server_running = running.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("cliente");
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .expect("timeout");
            let request = read_request(&mut stream).expect("upgrade");
            handle_realtime_stream(stream, request, state, gateway, server_running);
        });

        let mut request = format!("ws://{address}{REALTIME_PATH}")
            .into_client_request()
            .expect("requisição");
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            HeaderValue::from_static(REALTIME_SUBPROTOCOL),
        );
        let (mut socket, response) = connect(request).expect("websocket");
        assert_eq!(response.status(), 101);

        let request_nonce = URL_SAFE_NO_PAD.encode([3_u8; 16]);
        let stream_nonce = URL_SAFE_NO_PAD.encode([4_u8; 18]);
        let auth = encrypt_json(
            &transport_key,
            &serde_json::json!({
                "timestamp": crate::state::now_millis(),
                "requestNonce": request_nonce,
                "streamNonce": stream_nonce,
            }),
            b"lume-stream-auth-v1",
        )
        .expect("autenticação cifrada");
        socket
            .send(Message::Text(
                serde_json::json!({
                    "deviceId": credentials.device.id,
                    "nonce": auth.nonce,
                    "ciphertext": auth.ciphertext,
                })
                .to_string()
                .into(),
            ))
            .expect("autenticação");

        for (expected_sequence, expected_type) in [(1_u64, "hello"), (2, "snapshot")] {
            let text = loop {
                match socket.read().expect("mensagem") {
                    Message::Text(text) => break text,
                    Message::Ping(value) => {
                        socket.send(Message::Pong(value)).expect("pong");
                    }
                    _ => {}
                }
            };
            let outer: serde_json::Value = serde_json::from_str(&text).expect("envelope");
            assert_eq!(outer["type"], "secure_message");
            assert_eq!(outer["sequence"], expected_sequence);
            let encrypted = EncryptedEnvelope {
                nonce: outer["nonce"].as_str().expect("nonce").into(),
                ciphertext: outer["ciphertext"].as_str().expect("conteúdo").into(),
            };
            let decoded: serde_json::Value = decrypt_json(
                &transport_key,
                &encrypted,
                format!("lume-stream-message-v1:{stream_nonce}:{expected_sequence}").as_bytes(),
            )
            .expect("mensagem decifrada");
            assert_eq!(decoded["type"], expected_type);
        }

        running.store(false, Ordering::Relaxed);
        socket.close(None).expect("encerramento");
        server.join().expect("servidor");
    }

    #[test]
    fn snapshot_requires_a_paired_token() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        let response = route(
            request("GET", "/api/v1/snapshot", None, serde_json::Value::Null),
            &state,
            &gateway,
        );
        assert!(response.starts_with("HTTP/1.1 401"));
    }

    #[test]
    fn mobile_shell_is_public_but_session_data_is_not() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        let response = route(
            request("GET", "/pair?code=temporary", None, serde_json::Value::Null),
            &state,
            &gateway,
        );
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("Content-Security-Policy:"));
        assert!(response.contains("<title>Lume Mobile</title>"));
        assert!(!response.contains("temporary"));
    }

    #[test]
    fn cors_is_limited_to_mobile_app_and_published_pwa() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        let mut native = request("OPTIONS", "/api/v1/pair", None, serde_json::Value::Null);
        native
            .headers
            .insert("origin".into(), "capacitor://localhost".into());
        let native_response = route(native, &state, &gateway);
        assert!(native_response.contains("Access-Control-Allow-Origin: capacitor://localhost"));

        let mut pwa = request(
            "OPTIONS",
            "/api/v1/pair-secure",
            None,
            serde_json::Value::Null,
        );
        pwa.headers
            .insert("origin".into(), "https://tulerws.github.io".into());
        let pwa_response = route(pwa, &state, &gateway);
        assert!(pwa_response.contains("Access-Control-Allow-Origin: https://tulerws.github.io"));
        assert!(pwa_response.contains("Access-Control-Allow-Private-Network: true"));

        let mut external = request("OPTIONS", "/api/v1/pair", None, serde_json::Value::Null);
        external
            .headers
            .insert("origin".into(), "https://example.com".into());
        let external_response = route(external, &state, &gateway);
        assert!(!external_response.contains("Access-Control-Allow-Origin"));
    }

    #[test]
    fn pairing_grants_monitoring_but_never_returns_the_hash() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        gateway.set_pairing_base_url("https://127.0.0.1:43122".into());
        let offer = gateway.begin_pairing().expect("oferta");
        let response = route(
            request(
                "POST",
                "/api/v1/pair",
                None,
                serde_json::json!({
                    "code": offer.code,
                    "deviceName": "Telefone",
                }),
            ),
            &state,
            &gateway,
        );
        assert!(response.starts_with("HTTP/1.1 201"));
        assert!(!response.contains("token_hash"));
        assert!(!response.contains("tokenHash"));
    }

    #[test]
    fn encrypted_pairing_and_requests_never_expose_credentials() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        gateway.set_pairing_base_url("http://192.168.1.50:43124".into());
        let offer = gateway.begin_pairing().expect("oferta");
        let pairing_key: [u8; 32] = Sha256::digest(offer.code.as_bytes()).into();
        let pairing = encrypt_json(
            &pairing_key,
            &serde_json::json!({
                "deviceName": "Telefone",
                "timestamp": crate::state::now_millis(),
            }),
            b"lume-pair-request-v1",
        )
        .expect("pareamento cifrado");
        let response = route(
            request(
                "POST",
                "/api/v1/pair-secure",
                None,
                serde_json::to_value(pairing).expect("envelope"),
            ),
            &state,
            &gateway,
        );
        assert!(response.starts_with("HTTP/1.1 201"));
        assert!(!response.contains("\"token\""));
        let (_, response_body) = response_json_parts(&response);
        let response_envelope: EncryptedEnvelope =
            serde_json::from_value(response_body).expect("resposta cifrada");
        let credentials: serde_json::Value =
            decrypt_json(&pairing_key, &response_envelope, b"lume-pair-response-v1")
                .expect("credenciais");
        let token = credentials["token"].as_str().expect("token");
        let device_id = credentials["device"]["id"].as_str().expect("dispositivo");
        let transport_key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let request_nonce = URL_SAFE_NO_PAD.encode([7_u8; 16]);
        let secure_request = encrypt_json(
            &transport_key,
            &serde_json::json!({
                "method": "GET",
                "path": "/api/v1/me",
                "body": null,
                "timestamp": crate::state::now_millis(),
                "requestNonce": request_nonce,
            }),
            b"lume-secure-request-v1",
        )
        .expect("requisição cifrada");
        let secure_response = route(
            request(
                "POST",
                "/api/v1/secure",
                None,
                serde_json::json!({
                    "deviceId": device_id,
                    "nonce": secure_request.nonce,
                    "ciphertext": secure_request.ciphertext,
                }),
            ),
            &state,
            &gateway,
        );
        assert!(secure_response.starts_with("HTTP/1.1 200"));
        assert!(!secure_response.contains("Telefone"));
        let (_, secure_body) = response_json_parts(&secure_response);
        let secure_envelope: EncryptedEnvelope =
            serde_json::from_value(secure_body).expect("resposta segura");
        let decoded: SecureResponsePayload = decrypt_json(
            &transport_key,
            &secure_envelope,
            format!("lume-secure-response-v1:{request_nonce}").as_bytes(),
        )
        .expect("resposta decifrada");
        assert_eq!(decoded.status, 200);
        assert_eq!(decoded.body["name"], "Telefone");
    }

    #[test]
    fn browser_webcrypto_envelope_matches_the_desktop_cipher() {
        let key: [u8; 32] = Sha256::digest(b"lume-cross-platform-test").into();
        let envelope = EncryptedEnvelope {
            nonce: "AAECAwQFBgcICQoL".into(),
            ciphertext: "JXSIvh5kXBXm-lf4vWqiHmNTDrBOFzG-D8gPiTqOg_iyNwrvgk0EZBJi3hHNUbYTHYFUVT3R"
                .into(),
        };
        let payload: SecurePairPayload =
            decrypt_json(&key, &envelope, b"lume-pair-request-v1").expect("Web Crypto");

        assert_eq!(payload.device_name, "Phone");
        assert_eq!(payload.timestamp, 123);
    }

    #[test]
    fn monitoring_token_cannot_execute_prompts() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = MobileGateway::default();
        gateway.set_pairing_base_url("https://127.0.0.1:43122".into());
        let offer = gateway.begin_pairing().expect("oferta");
        let credentials = gateway
            .complete_pairing(&state, &offer.code, "Telefone")
            .expect("pareamento");
        let response = route(
            request(
                "POST",
                "/api/v1/commands",
                Some(&credentials.token),
                serde_json::json!({
                    "requestId": "mobile-test",
                    "type": "submit_prompt",
                    "sessionId": "codex:test",
                    "prompt": "continue",
                }),
            ),
            &state,
            &gateway,
        );
        assert!(response.starts_with("HTTP/1.1 403"));
        assert!(response.contains("scope_required"));
    }

    #[test]
    fn tls_identity_is_persisted_and_reused() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("relógio")
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("lume-mobile-tls-{}-{suffix}", std::process::id()));

        let first = TlsIdentity::load_or_create(&directory, &["192.168.1.50".into()])
            .expect("primeira identidade");
        let first_fingerprint = first.ca_fingerprint.clone();
        drop(first);
        let second = TlsIdentity::load_or_create(&directory, &["192.168.1.51".into()])
            .expect("identidade persistida");

        assert_eq!(second.ca_fingerprint, first_fingerprint);
        assert!(directory.join("mobile/lume-ca.crt").is_file());
        assert!(directory.join("mobile/lume-ca-key.der").is_file());
        assert!(directory.join("mobile/lume-server-key.der").is_file());

        let _ = fs::remove_dir_all(directory);
    }
}
