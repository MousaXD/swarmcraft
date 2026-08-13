use hex::{decode, encode};
use rand_core::{OsRng, RngCore};
use std::{fmt, net::{IpAddr, Ipv4Addr, SocketAddr}, str::FromStr, time::Duration};
use swarm_protocol::Hash32;
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{tcp::{OwnedReadHalf, OwnedWriteHalf}, TcpListener},
    time::timeout,
};

const MAX_CONTROL_LINE: usize = 16 * 1024;

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
    #[error("Fabric bridge request timed out")]
    Timeout,
    #[error("Fabric bridge returned {code} for request {request_id}")]
    RemoteError { request_id: u64, code: String },
    #[error("unexpected Fabric bridge response: {0}")]
    UnexpectedResponse(String),
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
        Ok(IpcLaunchConfig {
            host: address.ip().to_string(),
            port: address.port(),
            token: self.token.clone(),
        })
    }

    pub async fn accept(&self, deadline: Duration) -> Result<FabricSession, IpcTransportError> {
        let (stream, peer) = timeout(deadline, self.listener.accept())
            .await
            .map_err(|_| IpcTransportError::Timeout)??;
        if !peer.ip().is_loopback() {
            return Err(IpcTransportError::NonLoopbackPeer);
        }
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let auth = read_line_bounded(&mut reader).await?;
        if auth != format!("AUTH\t{}", self.token) {
            return Err(IpcTransportError::AuthenticationFailed);
        }
        write.write_all(b"AUTH_OK\n").await?;
        write.flush().await?;

        let info = parse_world_info(&read_line_bounded(&mut reader).await?)?;
        Ok(FabricSession { reader, writer: write, world_info: info })
    }
}

pub struct FabricSession {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    world_info: FabricWorldInfo,
}

impl FabricSession {
    pub fn world_info(&self) -> &FabricWorldInfo {
        &self.world_info
    }

    pub async fn save_barrier(&mut self, request_id: u64, deadline: Duration) -> Result<(), IpcTransportError> {
        self.request("SAVE_BARRIER", "SAVE_COMPLETE", request_id, deadline).await
    }

    pub async fn prepare_shutdown(
        &mut self,
        request_id: u64,
        deadline: Duration,
    ) -> Result<(), IpcTransportError> {
        self.request("PREPARE_SHUTDOWN", "READY_FOR_SHUTDOWN", request_id, deadline).await
    }

    async fn request(
        &mut self,
        command: &str,
        expected: &str,
        request_id: u64,
        deadline: Duration,
    ) -> Result<(), IpcTransportError> {
        self.writer.write_all(format!("{command}\t{request_id}\n").as_bytes()).await?;
        self.writer.flush().await?;
        let line = timeout(deadline, read_line_bounded(&mut self.reader))
            .await
            .map_err(|_| IpcTransportError::Timeout)??;
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() >= 3 && fields[0] == "ERROR" {
            let remote_request = fields[1]
                .parse::<u64>()
                .map_err(|_| IpcTransportError::Malformed(line.clone()))?;
            return Err(IpcTransportError::RemoteError { request_id: remote_request, code: fields[2].to_owned() });
        }
        if fields.len() != 2 || fields[0] != expected || fields[1] != request_id.to_string() {
            return Err(IpcTransportError::UnexpectedResponse(line));
        }
        Ok(())
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
    if fields.len() != 5 || fields[0] != "WORLD_INFO" {
        return Err(IpcTransportError::Malformed(line.to_owned()));
    }
    let fingerprint = Hash32::from_str(fields[4]).map_err(|_| IpcTransportError::InvalidFingerprint)?;
    Ok(FabricWorldInfo {
        minecraft_version: decode_hex_string(fields[1])?,
        fabric_loader_version: decode_hex_string(fields[2])?,
        world_directory: decode_hex_string(fields[3])?,
        compatibility_fingerprint: fingerprint,
    })
}

fn decode_hex_string(value: &str) -> Result<String, IpcTransportError> {
    let bytes = decode(value).map_err(|_| IpcTransportError::Malformed("invalid hex field".into()))?;
    String::from_utf8(bytes).map_err(|_| IpcTransportError::Malformed("hex field is not UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{io::{AsyncBufReadExt, BufReader}, net::TcpStream};

    #[tokio::test]
    async fn authenticated_bridge_reports_info_and_completes_save_barrier() {
        let listener = FabricBridgeListener::bind().await.unwrap();
        let config = listener.launch_config().unwrap();
        let client = tokio::spawn(async move {
            let stream = TcpStream::connect((config.host.as_str(), config.port)).await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            write.write_all(format!("AUTH\t{}\n", config.token()).as_bytes()).await.unwrap();
            write.flush().await.unwrap();
            let mut auth_ok = String::new();
            reader.read_line(&mut auth_ok).await.unwrap();
            assert_eq!(auth_ok.trim(), "AUTH_OK");
            write
                .write_all(format!("WORLD_INFO\t{}\t{}\t{}\t{}\n", encode("26.1.2"), encode("0.19.3"), encode("/world"), Hash32([7; 32])).as_bytes())
                .await
                .unwrap();
            write.flush().await.unwrap();
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command.trim(), "SAVE_BARRIER\t42");
            write.write_all(b"SAVE_COMPLETE\t42\n").await.unwrap();
            write.flush().await.unwrap();
        });

        let mut session = listener.accept(Duration::from_secs(5)).await.unwrap();
        assert_eq!(session.world_info().minecraft_version, "26.1.2");
        assert_eq!(session.world_info().compatibility_fingerprint, Hash32([7; 32]));
        session.save_barrier(42, Duration::from_secs(5)).await.unwrap();
        client.await.unwrap();
    }
}
