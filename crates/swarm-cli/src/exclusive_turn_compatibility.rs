//! Narrow offline preflight for the exclusive-turn reference policy.
//! Three exact core JARs and the level.dat data version are checked; this
//! does not establish an installed runtime, a save barrier or game authority.
use crate::server_mods::{inflate_java_gzip, read_zip_entry};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use swarm_protocol::{
    runtime_artifact_hash, Hash32, PINNED_FABRIC_LOADER_VERSION, PINNED_JAVA_MAJOR, PINNED_MINECRAFT_VERSION,
};

const DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/core-runtime-artifacts/v1\0";
pub const PINNED_DATA_VERSION: i32 = 4790; // Minecraft 26.1.2 version.json
pub const MAX_CORE_JAR_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_LEVEL_COMPRESSED_BYTES: usize = 1024 * 1024;
const MAX_LEVEL_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_TAGS: usize = 200_000;

#[derive(Debug, Deserialize)]
struct MinecraftMetadata {
    id: String,
    world_version: i32,
    java_version: u32,
    stable: bool,
    series_id: String,
}

#[derive(Debug, Deserialize)]
struct ModMetadata {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    id: String,
    version: String,
    environment: String,
}

#[derive(Debug, Serialize)]
pub struct CoreRuntimePin {
    pub minecraft_version: String,
    pub fabric_loader_version: String,
    pub bridge_version: String,
    pub declared_java_major: u32,
    pub data_version: i32,
    pub minecraft_jar: Hash32,
    pub fabric_loader_jar: Hash32,
    pub bridge_jar: Hash32,
}

impl CoreRuntimePin {
    pub fn fingerprint(&self) -> Result<Hash32> {
        Ok(Hash32::from_domain_bytes(DOMAIN, &postcard::to_allocvec(self)?))
    }
}

pub fn inspect_core_runtime(minecraft: &[u8], loader: &[u8], bridge: &[u8]) -> Result<CoreRuntimePin> {
    ensure!(
        [minecraft, loader, bridge].iter().all(|bytes| !bytes.is_empty() && bytes.len() <= MAX_CORE_JAR_BYTES),
        "core runtime JAR size out of bounds"
    );
    let mc: MinecraftMetadata = serde_json::from_slice(&read_zip_entry(minecraft, "version.json")?)?;
    ensure!(
        mc.id == PINNED_MINECRAFT_VERSION
            && mc.world_version == PINNED_DATA_VERSION
            && mc.java_version == PINNED_JAVA_MAJOR
            && mc.stable
            && mc.series_id == "main",
        "Minecraft JAR metadata differs from the pinned stable baseline"
    );
    let loader_meta: ModMetadata = serde_json::from_slice(&read_zip_entry(loader, "fabric.mod.json")?)?;
    ensure!(
        loader_meta.schema_version == 1
            && loader_meta.id == "fabricloader"
            && loader_meta.version == PINNED_FABRIC_LOADER_VERSION
            && loader_meta.environment == "*",
        "Fabric Loader JAR metadata differs from the pinned baseline"
    );
    let bridge_meta: ModMetadata = serde_json::from_slice(&read_zip_entry(bridge, "fabric.mod.json")?)?;
    ensure!(
        bridge_meta.schema_version == 1
            && bridge_meta.id == "swarmcraft"
            && bridge_meta.version == env!("CARGO_PKG_VERSION")
            && bridge_meta.environment == "server",
        "SwarmCraft bridge JAR metadata differs from the pinned server adapter"
    );
    Ok(CoreRuntimePin {
        minecraft_version: mc.id,
        fabric_loader_version: loader_meta.version,
        bridge_version: bridge_meta.version,
        declared_java_major: mc.java_version,
        data_version: mc.world_version,
        minecraft_jar: runtime_artifact_hash(minecraft),
        fabric_loader_jar: runtime_artifact_hash(loader),
        bridge_jar: runtime_artifact_hash(bridge),
    })
}

// Bounded structural NBT walk. Names are compared as raw Java modified-UTF-8
// bytes; no text is rendered or normalized. Only root/Data/DataVersion has
// version semantics. Ignore contents of other tags after validating shape.
struct Nbt<'a> {
    bytes: &'a [u8],
    tags: usize,
    data_version: Option<i32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    Root,
    Data,
    Other,
}

impl<'a> Nbt<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let (head, tail) = self.bytes.split_at_checked(count).context("truncated level.dat NBT")?;
        self.bytes = tail;
        Ok(head)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn int(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into()?))
    }
    fn name(&mut self) -> Result<&'a [u8]> {
        let size = u16::from_be_bytes(self.take(2)?.try_into()?) as usize;
        let bytes = self.take(size)?;
        // Canonical modified UTF-8 prevents overlong ASCII spellings of
        // Data and DataVersion from hiding duplicate keys from this walk.
        let mut remaining = bytes;
        while let Some((&first, rest)) = remaining.split_first() {
            let width = match first {
                1..=0x7f => 1,
                0xc0 if rest.first() == Some(&0x80) => 2,
                0xc2..=0xdf if rest.first().is_some_and(|b| (0x80..=0xbf).contains(b)) => 2,
                0xe0..=0xef
                    if rest.len() >= 2
                        && (0x80..=0xbf).contains(&rest[0])
                        && (0x80..=0xbf).contains(&rest[1])
                        && (first != 0xe0 || rest[0] >= 0xa0) =>
                {
                    3
                }
                _ => bail!("noncanonical Java modified UTF-8 in level.dat NBT"),
            };
            remaining = &remaining[width..];
        }
        Ok(bytes)
    }
    fn count(&mut self) -> Result<usize> {
        usize::try_from(self.int()?).context("negative NBT collection length")
    }
    fn budget(&mut self, count: usize) -> Result<()> {
        self.tags = self.tags.checked_add(count).context("NBT tag budget overflow")?;
        ensure!(self.tags <= MAX_TAGS, "level.dat NBT tag budget exceeded");
        Ok(())
    }
    fn compound(&mut self, depth: usize, scope: Scope) -> Result<()> {
        ensure!(depth <= MAX_DEPTH, "level.dat NBT depth exceeded");
        let mut seen = BTreeSet::new();
        loop {
            let tag = self.byte()?;
            if tag == 0 {
                break;
            }
            self.budget(1)?;
            let name = self.name()?;
            ensure!(seen.insert(name), "duplicate level.dat NBT compound key");
            if scope == Scope::Root && name == b"Data" {
                ensure!(tag == 10, "level.dat Data must be a compound");
                self.compound(depth + 1, Scope::Data)?;
            } else if scope == Scope::Data && name == b"DataVersion" {
                ensure!(tag == 3, "level.dat DataVersion must be an integer");
                self.data_version = Some(self.int()?);
            } else {
                self.payload(tag, depth + 1)?;
            }
        }
        Ok(())
    }
    fn payload(&mut self, tag: u8, depth: usize) -> Result<()> {
        ensure!(depth <= MAX_DEPTH, "level.dat NBT depth exceeded");
        match tag {
            1 => {
                self.take(1)?;
            }
            2 => {
                self.take(2)?;
            }
            3 | 5 => {
                self.take(4)?;
            }
            4 | 6 => {
                self.take(8)?;
            }
            7 | 11 | 12 => {
                let count = self.count()?;
                let width = match tag {
                    7 => 1,
                    11 => 4,
                    _ => 8,
                };
                self.take(count.checked_mul(width).context("NBT array length overflow")?)?;
            }
            8 => {
                self.name()?;
            }
            9 => {
                let element = self.byte()?;
                let count = self.count()?;
                ensure!(element <= 12 && (element != 0 || count == 0), "invalid NBT list element type");
                self.budget(count)?;
                for _ in 0..count {
                    self.payload(element, depth + 1)?;
                }
            }
            10 => self.compound(depth, Scope::Other)?,
            _ => bail!("invalid level.dat NBT tag type"),
        }
        Ok(())
    }
}

pub fn verify_level_version(compressed: &[u8], expected: i32) -> Result<i32> {
    ensure!(
        !compressed.is_empty() && compressed.len() <= MAX_LEVEL_COMPRESSED_BYTES,
        "level.dat compressed size out of bounds"
    );
    let bytes = inflate_java_gzip(compressed, MAX_LEVEL_BYTES)?;
    let mut nbt = Nbt { bytes: &bytes, tags: 0, data_version: None };
    ensure!(nbt.byte()? == 10 && nbt.name()?.is_empty(), "level.dat requires an unnamed root compound");
    nbt.compound(0, Scope::Root)?;
    ensure!(nbt.bytes.is_empty(), "trailing level.dat NBT bytes");
    let version = nbt.data_version.context("level.dat missing Data/DataVersion")?;
    ensure!(version == expected, "level.dat DataVersion {version} differs from pinned runtime {expected}");
    Ok(version)
}
