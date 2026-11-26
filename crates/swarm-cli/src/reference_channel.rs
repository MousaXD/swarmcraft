//! Common bounded Noise reference channel. Authority policy, domain and
//! independently pinned identity context belong to the caller.
use anyhow::{bail, ensure, Context, Result};
use std::{
    io::{Read, Write},
    net::{IpAddr, TcpStream},
    time::{Duration, Instant},
};
use swarm_core::PeerIdentity;

const CHUNK: usize = 32 * 1024;
const TAG: usize = 16;

pub(crate) fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
}

pub(crate) fn remaining(deadline: Instant) -> Result<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    ensure!(!left.is_zero(), "ownerless observation deadline expired");
    Ok(left)
}

pub(crate) fn read_all(stream: &mut TcpStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        match stream.read(bytes) {
            Ok(0) => bail!("truncated observation frame"),
            Ok(n) => bytes = &mut bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    remaining(deadline)?;
    Ok(())
}

pub(crate) fn write_all(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        match stream.write(bytes) {
            Ok(0) => bail!("observation frame write failed"),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    remaining(deadline)?;
    Ok(())
}

fn wire_send(stream: &mut TcpStream, bytes: &[u8], deadline: Instant) -> Result<()> {
    ensure!(!bytes.is_empty() && bytes.len() <= CHUNK + TAG, "invalid decision frame");
    write_all(stream, &(bytes.len() as u32).to_be_bytes(), deadline)?;
    write_all(stream, bytes, deadline)
}

fn wire_recv(stream: &mut TcpStream, max: usize, deadline: Instant) -> Result<Vec<u8>> {
    let mut size = [0; 4];
    read_all(stream, &mut size, deadline)?;
    let size = u32::from_be_bytes(size) as usize;
    ensure!((1..=max).contains(&size) && max <= CHUNK + TAG, "invalid decision frame size");
    let mut bytes = vec![0; size];
    read_all(stream, &mut bytes, deadline)?;
    Ok(bytes)
}

pub(crate) struct Channel {
    stream: TcpStream,
    noise: snow::TransportState,
    deadline: Instant,
}

impl Channel {
    pub(crate) fn send_chunk(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(!bytes.is_empty() && bytes.len() <= CHUNK, "invalid decision plaintext chunk");
        let mut cipher = vec![0; bytes.len() + TAG];
        let n = self.noise.write_message(bytes, &mut cipher)?;
        ensure!(n == cipher.len(), "unexpected decision ciphertext size");
        wire_send(&mut self.stream, &cipher, self.deadline)
    }

    pub(crate) fn recv_chunk(&mut self, len: usize) -> Result<Vec<u8>> {
        ensure!((1..=CHUNK).contains(&len), "invalid decision plaintext length");
        let cipher = wire_recv(&mut self.stream, len + TAG, self.deadline)?;
        ensure!(cipher.len() == len + TAG, "invalid decision ciphertext length");
        let mut plain = vec![0; len];
        let n = self.noise.read_message(&cipher, &mut plain)?;
        ensure!(n == len, "unexpected decision plaintext size");
        Ok(plain)
    }

    pub(crate) fn send(&mut self, bytes: &[u8], max: usize) -> Result<()> {
        ensure!(!bytes.is_empty() && bytes.len() <= max, "invalid decision message length");
        self.send_chunk(&(bytes.len() as u32).to_be_bytes())?;
        for chunk in bytes.chunks(CHUNK) {
            self.send_chunk(chunk)?;
        }
        Ok(())
    }

    pub(crate) fn receive(&mut self, max: usize) -> Result<Vec<u8>> {
        let size = u32::from_be_bytes(self.recv_chunk(4)?.try_into().expect("exact chunk length")) as usize;
        ensure!((1..=max).contains(&size), "invalid decision message length");
        let mut bytes = Vec::with_capacity(size);
        while bytes.len() < size {
            bytes.extend(self.recv_chunk((size - bytes.len()).min(CHUNK))?);
        }
        Ok(bytes)
    }
}

fn auth_bytes(auth_domain: &[u8], prologue: &[u8], transcript: &[u8], initiator: bool) -> Vec<u8> {
    let mut bytes = auth_domain.to_vec();
    bytes.push(u8::from(initiator));
    bytes.extend(prologue);
    bytes.extend(transcript);
    bytes
}

pub(crate) fn verify_auth(key: &[u8; 32], bytes: &[u8], signature: &[u8]) -> Result<()> {
    ed25519_dalek::VerifyingKey::from_bytes(key)?
        .verify_strict(bytes, &ed25519_dalek::Signature::from_slice(signature)?)
        .context("decision channel peer authentication failed")
}

pub(crate) fn authenticate(
    mut stream: TcpStream,
    prologue: &[u8],
    auth_domain: &[u8],
    local: &PeerIdentity,
    remote_key: [u8; 32],
    initiator: bool,
    deadline: Instant,
) -> Result<Channel> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    let builder = snow::Builder::new("Noise_NN_25519_ChaChaPoly_BLAKE2s".parse()?).prologue(prologue)?;
    let mut handshake = if initiator { builder.build_initiator()? } else { builder.build_responder()? };
    let mut output = [0; 128];
    if initiator {
        let len = handshake.write_message(&[], &mut output)?;
        wire_send(&mut stream, &output[..len], deadline)?;
        let input = wire_recv(&mut stream, 128, deadline)?;
        ensure!(handshake.read_message(&input, &mut output)? == 0, "unexpected Noise handshake payload");
    } else {
        let input = wire_recv(&mut stream, 128, deadline)?;
        ensure!(handshake.read_message(&input, &mut output)? == 0, "unexpected Noise handshake payload");
        let len = handshake.write_message(&[], &mut output)?;
        wire_send(&mut stream, &output[..len], deadline)?;
    }
    let own_auth = auth_bytes(auth_domain, prologue, handshake.get_handshake_hash(), initiator);
    let peer_auth = auth_bytes(auth_domain, prologue, handshake.get_handshake_hash(), !initiator);
    let mut channel = Channel { stream, noise: handshake.into_transport_mode()?, deadline };
    // The initiator receives the pinned responder's proof before signing.
    // The responder accepts no application data before the initiator proves
    // its own identity on this same channel and exact reviewed context.
    if initiator {
        verify_auth(&remote_key, &peer_auth, &channel.recv_chunk(64)?)?;
        channel.send_chunk(&local.sign(&own_auth))?;
    } else {
        channel.send_chunk(&local.sign(&own_auth))?;
        verify_auth(&remote_key, &peer_auth, &channel.recv_chunk(64)?)?;
    }
    remaining(deadline)?;
    Ok(channel)
}
