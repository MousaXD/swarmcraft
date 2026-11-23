use hex::{decode, encode};
use rand_core::{OsRng, RngCore};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use swarm_protocol::{validate_runtime_selection, Hash32};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
    },
    sync::Mutex,
    task::JoinHandle,
    time::{sleep, timeout},
};

const MAX_CONTROL_LINE: usize = 16 * 1024;
const CONTROLLER_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
/// Leave time in the three-second canonical game-route proof budget for
/// fresh quorum collection, signature verification and final state rechecks.
const MAX_RUNTIME_CHALLENGE_WAIT: Duration = Duration::from_millis(1_500);

#[derive(Clone)]
pub struct IpcLaunchConfig {
    pub host: String,
    pub port: u16,
    token: String,
}

impl IpcLaunchConfig {
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn environment(&self) -> [(String, String); 3] {
        [
            ("SWARMCRAFT_IPC_HOST".into(), self.host.clone()),
            ("SWARMCRAFT_IPC_PORT".into(), self.port.to_string()),
            ("SWARMCRAFT_IPC_TOKEN".into(), self.token.clone()),
        ]
    }
}

impl fmt::Debug for IpcLaunchConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IpcLaunchConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricWorldInfo {
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub world_directory: String,
    pub compatibility_fingerprint: Hash32,
    pub java_major: u32,
}

#[derive(Debug, Error)]
pub enum IpcTransportError {
    #[error("IPC I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Fabric bridge connected from a non-loopback address")]
    NonLoopbackPeer,
    #[error("IPC control line exceeded {MAX_CONTROL_LINE} bytes")]
    LineTooLong,
    #[error("Fabric bridge authentication failed")]
    AuthenticationFailed,
    #[error("malformed IPC message: {0}")]
    Malformed(String),
    #[error("invalid compatibility fingerprint")]
    InvalidFingerprint,
    #[error("Fabric runtime is outside the shipped adapter contract: {0}")]
    UnsupportedRuntime(String),
    #[error("Fabric bridge request timed out")]
    Timeout,
    #[error("Fabric bridge returned {code} for request {request_id}")]
    RemoteError { request_id: u64, code: String },
    #[error("unexpected Fabric bridge response: {0}")]
    UnexpectedResponse(String),
    #[error("Fabric controller session is no longer trustworthy")]
    SessionInvalidated,
}

pub struct FabricBridgeListener {
    listener: TcpListener,
    token: String,
}

impl FabricBridgeListener {
    pub async fn bind() -> Result<Self, IpcTransportError> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).await?;
        let mut secret = [0u8; 32];
        OsRng.fill_bytes(&mut secret);
        Ok(Self { listener, token: encode(secret) })
    }

    pub fn launch_config(&self) -> Result<IpcLaunchConfig, IpcTransportError> {
        let address = self.listener.local_addr()?;
        Ok(IpcLaunchConfig { host: address.ip().to_string(), port: address.port(), token: self.token.clone() })
    }

    pub async fn accept(&self, deadline: Duration) -> Result<FabricSession, IpcTransportError> {
        // The budget covers the COMPLETE handshake. A loopback client that
        // connects but withholds AUTH or WORLD_INFO must not stall a launch.
        let (reader, write, info) = timeout(deadline, async {
            let (stream, peer) = self.listener.accept().await?;
            if !peer.ip().is_loopback() {
                return Err(IpcTransportError::NonLoopbackPeer);
            }
            let (read, write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let auth = read_line_bounded(&mut reader).await?;
            if auth != format!("AUTH\t{}", self.token) {
                return Err(IpcTransportError::AuthenticationFailed);
            }
            let info = parse_world_info(&read_line_bounded(&mut reader).await?)?;
            Ok::<_, IpcTransportError>((reader, write, info))
        })
        .await
        .map_err(|_| IpcTransportError::Timeout)??;
        let writer = Arc::new(Mutex::new(write));
        let heartbeat_writer = Arc::clone(&writer);
        let heartbeat = tokio::spawn(async move {
            loop {
                sleep(CONTROLLER_HEARTBEAT_INTERVAL).await;
                let mut writer = heartbeat_writer.lock().await;
                if writer.write_all(b"CONTROLLER_HEARTBEAT\t0\n").await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        });
        Ok(FabricSession { reader, writer, world_info: info, heartbeat, invalidated: false })
    }
}

pub struct FabricSession {
    reader: BufReader<OwnedReadHalf>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    world_info: FabricWorldInfo,
    heartbeat: JoinHandle<()>,
    invalidated: bool,
}

impl FabricSession {
    pub fn world_info(&self) -> &FabricWorldInfo {
        &self.world_info
    }

    pub async fn save_barrier(&mut self, request_id: u64, deadline: Duration) -> Result<(), IpcTransportError> {
        self.request("SAVE_BARRIER", "SAVE_COMPLETE", request_id, deadline).await
    }

    pub async fn prepare_shutdown(&mut self, request_id: u64, deadline: Duration) -> Result<(), IpcTransportError> {
        self.request("PREPARE_SHUTDOWN", "READY_FOR_SHUTDOWN", request_id, deadline).await
    }

    /// Ask the SAME token-authenticated live Fabric controller for fresh
    /// runtime evidence. The caller MUST subsequently validate the complete
    /// reply against its managed world, reserved loopback port and canonical
    /// fingerprint using `live_fabric_readiness::verify_runtime_reply`.
    /// This is never usable as authority or quorum proof on its own.
    pub async fn runtime_challenge(
        &mut self,
        request_id: u64,
        nonce: [u8; 32],
        deadline: Duration,
    ) -> Result<String, IpcTransportError> {
        if self.invalidated {
            return Err(IpcTransportError::SessionInvalidated);
        }
        if request_id == 0 || nonce == [0; 32] || deadline.is_zero() {
            return Err(IpcTransportError::Malformed("missing runtime challenge identity, nonce or deadline".into()));
        }
        let request = format!("RUNTIME_CHALLENGE\t{request_id}\t{}\n", encode(nonce));
        let result = timeout(deadline.min(MAX_RUNTIME_CHALLENGE_WAIT), async {
            {
                let mut writer = self.writer.lock().await;
                writer.write_all(request.as_bytes()).await?;
                writer.flush().await?;
            }
            let line = read_line_bounded(&mut self.reader).await?;
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() == 3 && fields[0] == "ERROR" && fields[1] == request_id.to_string() {
                return Err(IpcTransportError::RemoteError { request_id, code: fields[2].to_owned() });
            }
            if fields.len() != 7
                || fields[0] != "RUNTIME_READY"
                || fields[1] != request_id.to_string()
                || fields[2] != encode(nonce)
            {
                return Err(IpcTransportError::UnexpectedResponse(line));
            }
            Ok(line)
        })
        .await
        .map_err(|_| IpcTransportError::Timeout)
        .and_then(|reply| reply);
        if result.as_ref().is_err_and(|error| !matches!(error, IpcTransportError::RemoteError { .. })) {
            // A late response may otherwise satisfy a subsequent request on
            // this socket. Abandon the entire session after ANY RPC failure.
            self.invalidated = true;
            self.heartbeat.abort();
        }
        result
    }

    async fn request(
        &mut self,
        command: &str,
        expected: &str,
        request_id: u64,
        deadline: Duration,
    ) -> Result<(), IpcTransportError> {
        if self.invalidated {
            return Err(IpcTransportError::SessionInvalidated);
        }
        let result = timeout(deadline, async {
            {
                let mut writer = self.writer.lock().await;
                writer.write_all(format!("{command}\t{request_id}\n").as_bytes()).await?;
                writer.flush().await?;
            }
            let line = read_line_bounded(&mut self.reader).await?;
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() == 3 && fields[0] == "ERROR" && fields[1] == request_id.to_string() {
                return Err(IpcTransportError::RemoteError { request_id, code: fields[2].to_owned() });
            }
            if fields.len() != 2 || fields[0] != expected || fields[1] != request_id.to_string() {
                return Err(IpcTransportError::UnexpectedResponse(line));
            }
            Ok(())
        })
        .await
        .map_err(|_| IpcTransportError::Timeout)
        .and_then(|reply| reply);
        if result.as_ref().is_err_and(|error| !matches!(error, IpcTransportError::RemoteError { .. })) {
            self.invalidated = true;
            self.heartbeat.abort();
        }
        result
    }
}

impl Drop for FabricSession {
    fn drop(&mut self) {
        // Dropping the authenticated controller session must close the socket,
        // not leave a heartbeat task holding the writer open. The Fabric side
        // treats that EOF as a fail-closed save/stop signal.
        self.heartbeat.abort();
    }
}

async fn read_line_bounded<R>(reader: &mut R) -> Result<String, IpcTransportError>
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::with_capacity(128);
    loop {
        let mut byte = [0u8; 1];
        let read = reader.read(&mut byte).await?;
        if read == 0 {
            return Err(IpcTransportError::Malformed("unexpected EOF".into()));
        }
        if byte[0] == b'\n' {
            break;
        }
        if byte[0] != b'\r' {
            bytes.push(byte[0]);
        }
        if bytes.len() > MAX_CONTROL_LINE {
            return Err(IpcTransportError::LineTooLong);
        }
    }
    String::from_utf8(bytes).map_err(|_| IpcTransportError::Malformed("IPC message is not UTF-8".into()))
}

fn parse_world_info(line: &str) -> Result<FabricWorldInfo, IpcTransportError> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 6 || fields[0] != "WORLD_INFO" {
        return Err(IpcTransportError::Malformed(line.to_owned()));
    }
    let minecraft_version = decode_hex_string(fields[1])?;
    let fabric_loader_version = decode_hex_string(fields[2])?;
    let world_directory = decode_hex_string(fields[3])?;
    let compatibility_fingerprint = Hash32::from_str(fields[4]).map_err(|_| IpcTransportError::InvalidFingerprint)?;
    let java_major = fields[5]
        .parse::<u32>()
        .map_err(|_| IpcTransportError::Malformed("Java major version is not an unsigned integer".into()))?;
    validate_runtime_selection(&minecraft_version, &fabric_loader_version, Some(java_major))
        .map_err(|error| IpcTransportError::UnsupportedRuntime(error.to_string()))?;
    Ok(FabricWorldInfo {
        minecraft_version,
        fabric_loader_version,
        world_directory,
        compatibility_fingerprint,
        java_major,
    })
}

fn decode_hex_string(value: &str) -> Result<String, IpcTransportError> {
    let bytes = decode(value).map_err(|_| IpcTransportError::Malformed("invalid hex field".into()))?;
    String::from_utf8(bytes).map_err(|_| IpcTransportError::Malformed("hex field is not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        net::TcpStream,
    };

    #[tokio::test]
    async fn authenticated_bridge_reports_info_and_completes_save_barrier() {
        let listener = FabricBridgeListener::bind().await.unwrap();
        let config = listener.launch_config().unwrap();
        let client = tokio::spawn(async move {
            let stream = TcpStream::connect((config.host.as_str(), config.port)).await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            write.write_all(format!("AUTH\t{}\n", config.token()).as_bytes()).await.unwrap();
            write
                .write_all(
                    format!(
                        "WORLD_INFO\t{}\t{}\t{}\t{}\t25\n",
                        encode("26.1.2"),
                        encode("0.19.3"),
                        encode("/world"),
                        Hash32([7; 32])
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            write.flush().await.unwrap();
            loop {
                let mut command = String::new();
                reader.read_line(&mut command).await.unwrap();
                if command.starts_with("CONTROLLER_HEARTBEAT\t") {
                    continue;
                }
                assert_eq!(command.trim(), "SAVE_BARRIER\t42");
                break;
            }
            write.write_all(b"SAVE_COMPLETE\t42\n").await.unwrap();
            write.flush().await.unwrap();
        });

        let mut session = listener.accept(Duration::from_secs(5)).await.unwrap();
        assert_eq!(session.world_info().minecraft_version, "26.1.2");
        assert_eq!(session.world_info().java_major, 25);
        assert_eq!(session.world_info().compatibility_fingerprint, Hash32([7; 32]));
        session.save_barrier(42, Duration::from_secs(5)).await.unwrap();
        client.await.unwrap();
    }

    #[test]
    fn world_info_rejects_java_below_shipped_bridge_contract() {
        let line = format!(
            "WORLD_INFO\t{}\t{}\t{}\t{}\t21",
            encode("26.1.2"),
            encode("0.19.3"),
            encode("/world"),
            Hash32([7; 32])
        );
        assert!(matches!(parse_world_info(&line), Err(IpcTransportError::UnsupportedRuntime(_))));
    }

    async fn connected_test_bridge() -> (FabricSession, tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf)
    {
        let listener = FabricBridgeListener::bind().await.unwrap();
        let config = listener.launch_config().unwrap();
        let client = tokio::spawn(async move {
            let stream = TcpStream::connect((config.host.as_str(), config.port)).await.unwrap();
            let (read, mut write) = stream.into_split();
            write.write_all(format!("AUTH\t{}\n", config.token()).as_bytes()).await.unwrap();
            write
                .write_all(
                    format!(
                        "WORLD_INFO\t{}\t{}\t{}\t{}\t25\n",
                        encode("26.1.2"),
                        encode("0.19.3"),
                        encode("/world"),
                        Hash32([7; 32])
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            write.flush().await.unwrap();
            (read, write)
        });
        let session = listener.accept(Duration::from_secs(2)).await.unwrap();
        let (read, write) = client.await.unwrap();
        (session, read, write)
    }

    async fn next_command(reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> String {
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            if !line.starts_with("CONTROLLER_HEARTBEAT\t") {
                return line;
            }
        }
    }

    #[tokio::test]
    async fn runtime_challenge_is_one_use_and_uses_authenticated_session() {
        let (mut session, read, mut write) = connected_test_bridge().await;
        let client = tokio::spawn(async move {
            let mut reader = BufReader::new(read);
            assert_eq!(next_command(&mut reader).await, format!("RUNTIME_CHALLENGE\t42\t{}\n", encode([9; 32])));
            write
                .write_all(
                    format!(
                        "RUNTIME_READY\t42\t{}\t{}\t25567\t{}\t{}\n",
                        encode([9; 32]),
                        encode([3; 32]),
                        encode("/world"),
                        Hash32([7; 32])
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            write.flush().await.unwrap();
            assert_eq!(next_command(&mut reader).await, "SAVE_BARRIER\t43\n");
            write.write_all(b"SAVE_COMPLETE\t43\n").await.unwrap();
        });
        let reply = session.runtime_challenge(42, [9; 32], Duration::from_secs(2)).await.unwrap();
        assert!(reply.starts_with("RUNTIME_READY\t42\t"));
        session.save_barrier(43, Duration::from_secs(2)).await.unwrap();
        client.await.unwrap();
    }

    #[tokio::test]
    async fn stale_or_mismatched_runtime_reply_poison_session() {
        let (mut session, read, mut write) = connected_test_bridge().await;
        let client = tokio::spawn(async move {
            let mut reader = BufReader::new(read);
            assert!(next_command(&mut reader).await.starts_with("RUNTIME_CHALLENGE\t42\t"));
            write.write_all(b"RUNTIME_READY\t41\twrong\n").await.unwrap();
        });
        assert!(matches!(
            session.runtime_challenge(42, [9; 32], Duration::from_millis(500)).await,
            Err(IpcTransportError::UnexpectedResponse(_))
        ));
        assert!(matches!(
            session.save_barrier(43, Duration::from_millis(500)).await,
            Err(IpcTransportError::SessionInvalidated)
        ));
        client.await.unwrap();
    }

    #[tokio::test]
    async fn stalled_runtime_challenge_is_bounded_and_poisoned() {
        let (mut session, read, _write) = connected_test_bridge().await;
        let client = tokio::spawn(async move {
            let mut reader = BufReader::new(read);
            assert!(next_command(&mut reader).await.starts_with("RUNTIME_CHALLENGE\t42\t"));
            sleep(Duration::from_millis(120)).await;
        });
        assert!(matches!(
            session.runtime_challenge(42, [9; 32], Duration::from_millis(40)).await,
            Err(IpcTransportError::Timeout)
        ));
        assert!(matches!(
            session.runtime_challenge(43, [10; 32], Duration::from_secs(1)).await,
            Err(IpcTransportError::SessionInvalidated)
        ));
        client.await.unwrap();
    }

    #[tokio::test]
    async fn authenticated_handshake_has_a_total_deadline() {
        let listener = FabricBridgeListener::bind().await.unwrap();
        let config = listener.launch_config().unwrap();
        let _silent = TcpStream::connect((config.host.as_str(), config.port)).await.unwrap();
        assert!(matches!(listener.accept(Duration::from_millis(50)).await, Err(IpcTransportError::Timeout)));
    }
}
