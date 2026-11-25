//! Private process-lifetime loopback control for Desktop's game-route CLI.
//!
//! The daemon, not a second CLI process, owns the connected authenticated
//! SwarmNode, RouteVerifier and loopback Minecraft listener. Sidecar status
//! requests are fresh, nonce-bound RPCs to that running daemon; no persisted
//! `migration Ready`, remembered address or cached positive reply is accepted.

use anyhow::{anyhow, bail, Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};
use swarm_core::{random_nonce, DataPaths};
use swarm_network::game_transport::{GameSessionDiagnostic, VerifiedGameRouteStatus};
use swarm_protocol::WorldId;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, oneshot, Semaphore},
    task::JoinSet,
    time::timeout,
};

const VERSION: u16 = 1;
const MAX_RECORD_BYTES: u64 = 1024;
const MAX_REQUEST_BYTES: usize = 2048;
const MAX_RESPONSE_BYTES: usize = 4096;
const MAX_CLIENTS: usize = 8;
const CLIENT_READ_WAIT: Duration = Duration::from_secs(2);
const CLIENT_REPLY_WAIT: Duration = Duration::from_secs(20);
const CLIENT_CONNECT_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameRouteApiOperation {
    Select,
    Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointV1 {
    version: u16,
    port: u16,
    secret: [u8; 32],
    owner_pid: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestV1 {
    version: u16,
    secret: [u8; 32],
    nonce: [u8; 32],
    operation: GameRouteApiOperation,
    world_id: WorldId,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyV1 {
    version: u16,
    nonce: [u8; 32],
    result: GameRouteApiReply,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameRouteApiReply {
    Selected,
    Status { value: Box<GameRouteStatusV1> },
    Unavailable,
}

/// Denial-only diagnostic categories; never substitute for route verification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameRouteDiagnostic {
    GatewayUnavailable,
    PeerLinkUnavailable,
    HostProofUnavailable,
    RouteCheckTimedOut,
    AuthorityChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameRouteStatusV1 {
    pub world_id: String,
    pub state: String,
    pub quorum_verified: bool,
    pub hosting_consent_verified: bool,
    pub server_ready: bool,
    pub route_verified: bool,
    pub local_gateway: Option<String>,
    pub authority_peer_id: Option<String>,
    pub epoch: Option<u64>,
    pub fencing_token: Option<u64>,
    pub snapshot_hash: Option<String>,
    pub verified_at_unix_ms: Option<u64>,
    pub valid_until_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<GameRouteDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_session: Option<GameSessionDiagnostic>,
}

impl GameRouteStatusV1 {
    pub fn unavailable(world: WorldId) -> Self {
        Self {
            world_id: world.to_string(),
            state: "unavailable".to_owned(),
            quorum_verified: false,
            hosting_consent_verified: false,
            server_ready: false,
            route_verified: false,
            local_gateway: None,
            authority_peer_id: None,
            epoch: None,
            fencing_token: None,
            snapshot_hash: None,
            verified_at_unix_ms: None,
            valid_until_unix_ms: None,
            diagnostic: None,
            recent_session: None,
        }
    }

    pub fn unavailable_with_reason(world: WorldId, diagnostic: GameRouteDiagnostic) -> Self {
        Self { diagnostic: Some(diagnostic), ..Self::unavailable(world) }
    }

    /// A positive status can ONLY originate from the RouteVerifier-produced
    /// forward-route capability, while this daemon owns an actual loopback
    /// listener. No component may synthesize the verification booleans.
    pub fn verified(route: VerifiedGameRouteStatus, address: std::net::SocketAddrV4) -> Option<Self> {
        if *address.ip() != Ipv4Addr::LOCALHOST
            || address.port() == 0
            || route.epoch() == 0
            || route.fencing_token() == 0
            || route.valid_until_unix_ms() <= route.verified_at_unix_ms()
        {
            return None;
        }
        Some(Self {
            world_id: route.world_id().to_string(),
            state: "verified".to_owned(),
            quorum_verified: true,
            hosting_consent_verified: true,
            server_ready: true,
            route_verified: true,
            local_gateway: Some(address.to_string()),
            authority_peer_id: Some(route.authority_peer_id().to_string()),
            epoch: Some(route.epoch()),
            fencing_token: Some(route.fencing_token()),
            snapshot_hash: Some(route.checkpoint_hash().to_string()),
            verified_at_unix_ms: Some(route.verified_at_unix_ms()),
            valid_until_unix_ms: Some(route.valid_until_unix_ms()),
            diagnostic: None,
            recent_session: None,
        })
    }
}

pub struct GameRouteApiCommand {
    pub operation: GameRouteApiOperation,
    pub world: WorldId,
    pub response: oneshot::Sender<GameRouteApiReply>,
}

struct PublishedEndpoint {
    path: PathBuf,
    record: EndpointV1,
    _exclusive: File,
}

impl Drop for PublishedEndpoint {
    fn drop(&mut self) {
        // A stopped old daemon must never remove a newer daemon's endpoint.
        if fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<EndpointV1>(&bytes).ok())
            .is_some_and(|current| current.owner_pid == self.record.owner_pid && current.secret == self.record.secret)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub struct GameRouteApiServer {
    listener: TcpListener,
    endpoint: PublishedEndpoint,
}

fn control_directory(paths: &DataPaths) -> PathBuf {
    paths.root.join("control")
}

fn endpoint_path(paths: &DataPaths) -> PathBuf {
    control_directory(paths).join("game-route-daemon.json")
}

fn open_private(path: &Path) -> Result<OpenOptions> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("game-route control path is not a regular private file");
        }
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options)
}

fn prepare_private_directory(paths: &DataPaths) -> Result<PathBuf> {
    let dir = control_directory(paths);
    swarm_core::create_private_dir_all(&dir)?;
    let metadata = fs::symlink_metadata(&dir)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("private game-route control directory is unsafe");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(dir)
}

impl GameRouteApiServer {
    pub async fn bind(paths: &DataPaths) -> Result<Self> {
        let dir = prepare_private_directory(paths)?;
        let lock_path = dir.join("game-route-daemon.lock");
        let lock = open_private(&lock_path)?.create(true).truncate(false).open(&lock_path)?;
        lock.try_lock_exclusive().context("another local game-route daemon already owns the control endpoint")?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let record = EndpointV1 {
            version: VERSION,
            port: listener.local_addr()?.port(),
            secret: random_nonce(),
            owner_pid: std::process::id(),
        };
        if record.secret == [0; 32] || record.port == 0 {
            bail!("could not create a private live game-route control endpoint");
        }
        let path = endpoint_path(paths);
        let tmp = dir.join(format!(".game-route-daemon-{}-{}.tmp", record.owner_pid, hex::encode(random_nonce())));
        let result = (|| -> Result<()> {
            let mut file = open_private(&tmp)?.create_new(true).open(&tmp)?;
            file.write_all(&serde_json::to_vec(&record)?)?;
            file.sync_all()?;
            drop(file);
            #[cfg(windows)]
            if path.exists() {
                fs::remove_file(&path)?;
            }
            fs::rename(&tmp, &path)?;
            #[cfg(unix)]
            File::open(&dir)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        Ok(Self { listener, endpoint: PublishedEndpoint { path, record, _exclusive: lock } })
    }

    pub async fn serve(self, requests: mpsc::Sender<GameRouteApiCommand>) -> Result<()> {
        let clients = std::sync::Arc::new(Semaphore::new(MAX_CLIENTS));
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                completed = connections.join_next(), if !connections.is_empty() => {
                    if let Some(Err(error)) = completed {
                        tracing::debug!(%error, "game-route local control client failed");
                    }
                }
                next = self.listener.accept() => {
                    let (socket, addr) = next?;
                    if !addr.ip().is_loopback() { continue; }
                    let Ok(slot) = std::sync::Arc::clone(&clients).try_acquire_owned() else { continue; };
                    let sender = requests.clone();
                    let secret = self.endpoint.record.secret;
                    connections.spawn(async move {
                        let _slot = slot;
                        let _ = handle_client(socket, secret, sender).await;
                    });
                }
            }
        }
    }
}

async fn read_bounded_line(socket: &mut TcpStream, maximum: usize) -> Result<Vec<u8>> {
    let mut result = Vec::with_capacity(maximum.min(512));
    while result.len() < maximum {
        let mut byte = [0u8; 1];
        if socket.read(&mut byte).await? == 0 {
            bail!("game-route local control peer closed before a full message");
        }
        if byte[0] == b'\n' {
            return Ok(result);
        }
        result.push(byte[0]);
    }
    bail!("game-route local control message exceeds hard size limit")
}

fn same_secret(a: &[u8; 32], b: &[u8; 32]) -> bool {
    // Avoid a byte-position timing oracle on the short-lived local token.
    a.iter().zip(b).fold(0u8, |diff, (left, right)| diff | (left ^ right)) == 0
}

async fn handle_client(
    mut socket: TcpStream,
    secret: [u8; 32],
    requests: mpsc::Sender<GameRouteApiCommand>,
) -> Result<()> {
    let raw = timeout(CLIENT_READ_WAIT, read_bounded_line(&mut socket, MAX_REQUEST_BYTES)).await??;
    let request: RequestV1 = serde_json::from_slice(&raw)?;
    if request.version != VERSION
        || request.nonce == [0; 32]
        || request.world_id == WorldId::default()
        || !same_secret(&secret, &request.secret)
    {
        bail!("game-route local control request is not authenticated");
    }
    let (tx, rx) = oneshot::channel();
    requests
        .try_send(GameRouteApiCommand { operation: request.operation, world: request.world_id, response: tx })
        .map_err(|_| anyhow!("game-route daemon route controller is unavailable or saturated"))?;
    let result = timeout(CLIENT_REPLY_WAIT, rx).await??;
    let mut bytes = serde_json::to_vec(&ReplyV1 { version: VERSION, nonce: request.nonce, result })?;
    if bytes.len() >= MAX_RESPONSE_BYTES {
        bail!("game-route status report exceeds size limit");
    }
    bytes.push(b'\n');
    socket.write_all(&bytes).await?;
    socket.flush().await?;
    Ok(())
}

pub async fn call_live_daemon(
    paths: &DataPaths,
    operation: GameRouteApiOperation,
    world: WorldId,
) -> Result<GameRouteApiReply> {
    if world == WorldId::default() {
        bail!("invalid selected game-route world");
    }
    let directory = control_directory(paths);
    let dir_metadata = fs::symlink_metadata(&directory)?;
    if !dir_metadata.is_dir() || dir_metadata.file_type().is_symlink() {
        bail!("untrusted local game-route daemon control directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if dir_metadata.permissions().mode() & 0o077 != 0 {
            bail!("game-route control directory allows other OS users to replace the live proof endpoint");
        }
    }
    let path = endpoint_path(paths);
    let endpoint_metadata = fs::symlink_metadata(&path)?;
    if !endpoint_metadata.is_file() || endpoint_metadata.file_type().is_symlink() {
        bail!("untrusted game-route daemon endpoint path");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if endpoint_metadata.permissions().mode() & 0o077 != 0 {
            bail!("game-route daemon endpoint token is visible to other OS users");
        }
    }
    let bytes = fs::File::open(&path)?;
    let mut raw = Vec::new();
    use std::io::Read;
    bytes.take(MAX_RECORD_BYTES + 1).read_to_end(&mut raw)?;
    if raw.is_empty() || raw.len() as u64 > MAX_RECORD_BYTES {
        bail!("invalid game-route daemon endpoint record");
    }
    let endpoint: EndpointV1 = serde_json::from_slice(&raw)?;
    if endpoint.version != VERSION || endpoint.port == 0 || endpoint.secret == [0; 32] || endpoint.owner_pid == 0 {
        bail!("invalid game-route daemon process identity");
    }
    let nonce = random_nonce();
    if nonce == [0; 32] {
        bail!("invalid local route status request nonce");
    }
    let mut socket = timeout(CLIENT_CONNECT_WAIT, TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port))).await??;
    let mut request = serde_json::to_vec(&RequestV1 {
        version: VERSION,
        secret: endpoint.secret,
        nonce,
        operation,
        world_id: world,
    })?;
    if request.len() >= MAX_REQUEST_BYTES {
        bail!("local game-route request exceeds size limit");
    }
    request.push(b'\n');
    timeout(CLIENT_READ_WAIT, socket.write_all(&request)).await??;
    let result = timeout(CLIENT_REPLY_WAIT, read_bounded_line(&mut socket, MAX_RESPONSE_BYTES)).await??;
    let envelope: ReplyV1 = serde_json::from_slice(&result)?;
    if envelope.version != VERSION || envelope.nonce != nonce {
        bail!("local game-route status did not match the live one-use request");
    }
    if let GameRouteApiReply::Status { value } = &envelope.result {
        if value.world_id != world.to_string() {
            bail!("local daemon returned another world's game route");
        }
    }
    Ok(envelope.result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn endpoint_never_reuses_status_or_claims_proof_without_daemon_reply() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path());
        assert!(call_live_daemon(&paths, GameRouteApiOperation::Status, WorldId([3; 32])).await.is_err());
        let server = GameRouteApiServer::bind(&paths).await.unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        let task = tokio::spawn(server.serve(tx));
        let caller = tokio::spawn({
            let paths = paths.clone();
            async move { call_live_daemon(&paths, GameRouteApiOperation::Status, WorldId([3; 32])).await }
        });
        let request = rx.recv().await.unwrap();
        assert_eq!(request.operation, GameRouteApiOperation::Status);
        assert_eq!(request.world, WorldId([3; 32]));
        request
            .response
            .send(GameRouteApiReply::Status { value: Box::new(GameRouteStatusV1::unavailable(WorldId([3; 32]))) })
            .unwrap();
        let reply = caller.await.unwrap().unwrap();
        assert!(
            matches!(reply, GameRouteApiReply::Status { value } if !value.route_verified && value.local_gateway.is_none())
        );
        task.abort();
        let _ = task.await;
        assert!(call_live_daemon(&paths, GameRouteApiOperation::Status, WorldId([3; 32])).await.is_err());
    }

    #[test]
    fn unverified_status_never_leaks_a_route_or_authority_generation() {
        let value = GameRouteStatusV1::unavailable(WorldId([3; 32]));
        assert_eq!(value.state, "unavailable");
        assert!(!value.route_verified);
        assert!(value.local_gateway.is_none());
        assert!(value.snapshot_hash.is_none());
        assert!(value.authority_peer_id.is_none());
    }
}
