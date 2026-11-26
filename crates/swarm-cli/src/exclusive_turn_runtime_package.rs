//! Exact offline Linux package inventory. This is not an executable launch
//! permit: the operator must independently trust the package, and a later
//! supervisor must keep this exact immutable generation fenced until exit.
use crate::{
    exclusive_turn_compatibility::{inspect_core_runtime, CoreRuntimePin, MAX_CORE_JAR_BYTES},
    machine_hosting::linux::{uid, validate_ancestors},
    server_mods::{read_zip_entry, read_zip_entry_bounded},
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, Metadata, OpenOptions},
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};
use swarm_protocol::{Hash32, PINNED_FABRIC_LOADER_VERSION, PINNED_MINECRAFT_VERSION};

const DOMAIN: &[u8] = b"swarmcraft/exclusive-turn/linux-runtime-package/v1\0";
const MAX_ENTRIES: usize = 4096;
const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_METADATA: usize = 1024 * 1024;
const MINECRAFT: &str = "versions/26.1.2/server-26.1.2.jar";
const LOADER: &str = "fabric-libraries/fabric-loader-0.19.3.jar";
const BRIDGE: &str = "mods/swarmcraft-fabric.jar";
const PROFILE: &str = "fabric-profile.json";

#[derive(Debug, Serialize, PartialEq, Eq)]
enum Entry {
    Directory,
    File { bytes: u64, executable: u32, sha256: Hash32 },
    LegalLink { target: String },
}

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    mode: u32,
    uid: u32,
    links: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl Stamp {
    fn new(meta: &Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
            mode: meta.mode(),
            uid: meta.uid(),
            links: meta.nlink(),
            bytes: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        }
    }
}

struct Inventory {
    entries: BTreeMap<String, Entry>,
    stamps: BTreeMap<String, Stamp>,
    total: u64,
}

#[derive(Debug)]
pub struct RuntimePackagePin {
    pub core: CoreRuntimePin,
    pub fingerprint: Hash32,
    pub tree_hash: Hash32,
    pub files: usize,
    pub entries: usize,
    pub bytes: u64,
    pub minecraft_libraries: usize,
    pub fabric_libraries: usize,
    pub java_version: String,
    pub embedded_fabric_api_version: String,
}

fn token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value != "."
        && value != ".."
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && value.split('/').all(token)
}

fn relative(root: &Path, path: &Path) -> Result<String> {
    let value = path.strip_prefix(root)?.to_str().context("non-UTF8 runtime path")?;
    ensure!(safe_relative(value), "unsafe runtime package path");
    Ok(value.to_owned())
}

fn check_permissions(meta: &Metadata, link: bool) -> Result<()> {
    ensure!(meta.uid() == uid(), "runtime package must be owned by the local account");
    if !link {
        ensure!(meta.mode() & 0o7022 == 0, "unsafe runtime package permissions");
    }
    Ok(())
}

fn open_checked(path: &Path, expected: &Stamp) -> Result<File> {
    let file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path)?;
    let meta = file.metadata()?;
    ensure!(meta.is_file() && meta.nlink() == 1 && Stamp::new(&meta) == *expected, "runtime file changed or is linked");
    Ok(file)
}

fn scan(root: &Path) -> Result<Inventory> {
    let mut entries = BTreeMap::new();
    let mut stamps = BTreeMap::new();
    let mut total = 0u64;
    let mut buffer = vec![0; 64 * 1024];
    for item in walkdir::WalkDir::new(root).follow_links(false).max_depth(32).sort_by_file_name() {
        let item = item?;
        let path = item.path();
        let meta = fs::symlink_metadata(path)?;
        let stamp = Stamp::new(&meta);
        check_permissions(&meta, meta.file_type().is_symlink())?;
        if path == root {
            ensure!(meta.is_dir(), "runtime package root is not a directory");
            stamps.insert(String::new(), stamp);
            continue;
        }
        ensure!(item.depth() < 32 && entries.len() < MAX_ENTRIES, "runtime package inventory bounds exceeded");
        let name = relative(root, path)?;
        let value = if meta.is_dir() {
            Entry::Directory
        } else if meta.is_file() {
            ensure!(meta.nlink() == 1 && meta.len() <= MAX_FILE_BYTES, "linked or oversized runtime file");
            total = total.checked_add(meta.len()).context("runtime package size overflow")?;
            ensure!(total <= MAX_TOTAL_BYTES, "runtime package total byte bound exceeded");
            let mut file = open_checked(path, &stamp)?;
            let mut hash = Sha256::new();
            let mut read = 0u64;
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                read += n as u64;
                ensure!(read <= meta.len(), "runtime file grew during verification");
                hash.update(&buffer[..n]);
            }
            ensure!(
                read == meta.len() && Stamp::new(&file.metadata()?) == stamp,
                "runtime file changed during verification"
            );
            Entry::File { bytes: read, executable: meta.mode() & 0o111, sha256: Hash32(hash.finalize().into()) }
        } else if meta.file_type().is_symlink() {
            // The pinned Temurin distribution has relative license links.
            // No executable, dependency, config or directory links are allowed.
            ensure!(name.starts_with("java/legal/"), "runtime package symlink outside Java legal files");
            let target = fs::read_link(path)?;
            ensure!(!target.is_absolute(), "absolute Java legal symlink");
            let text = target.to_str().context("non-UTF8 Java legal symlink")?;
            ensure!(text.len() <= 1024 && !text.is_empty(), "invalid Java legal symlink");
            let resolved = path.canonicalize()?;
            ensure!(
                resolved.starts_with(root.join("java/legal")) && fs::symlink_metadata(&resolved)?.is_file(),
                "external or non-file Java legal symlink"
            );
            Entry::LegalLink { target: text.to_owned() }
        } else {
            bail!("special file in runtime package");
        };
        entries.insert(name.clone(), value);
        stamps.insert(name, stamp);
    }
    Ok(Inventory { entries, stamps, total })
}

impl Inventory {
    fn sha(&self, name: &str) -> Result<Hash32> {
        match self.entries.get(name) {
            Some(Entry::File { bytes, sha256, .. }) if *bytes != 0 => Ok(*sha256),
            _ => bail!("required nonempty runtime file missing: {name}"),
        }
    }

    fn read(&self, root: &Path, name: &str, limit: usize) -> Result<Vec<u8>> {
        let digest = self.sha(name)?;
        let stamp = self.stamps.get(name).context("missing runtime file identity")?;
        ensure!(stamp.bytes <= limit as u64, "runtime metadata exceeds byte bound");
        let mut file = open_checked(&root.join(name), stamp)?;
        let mut bytes = Vec::new();
        (&mut file).take(limit as u64 + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 == stamp.bytes
                && Hash32(Sha256::digest(&bytes).into()) == digest
                && Stamp::new(&file.metadata()?) == *stamp,
            "runtime bytes changed after inventory"
        );
        Ok(bytes)
    }

    fn stable(&self, root: &Path) -> Result<()> {
        let mut count = 0;
        for item in walkdir::WalkDir::new(root).follow_links(false).max_depth(32) {
            let item = item?;
            let name = if item.path() == root { String::new() } else { relative(root, item.path())? };
            ensure!(
                self.stamps.get(&name) == Some(&Stamp::new(&fs::symlink_metadata(item.path())?)),
                "runtime package changed during review"
            );
            count += 1;
        }
        ensure!(count == self.stamps.len(), "runtime package entries changed during review");
        Ok(())
    }
}

fn hash_text(value: &str) -> Result<Hash32> {
    ensure!(
        value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid dependency SHA-256"
    );
    Ok(value.parse()?)
}

fn minecraft_records(bytes: &[u8]) -> Result<BTreeMap<String, Hash32>> {
    let mut paths = BTreeMap::new();
    let mut coordinates = BTreeSet::new();
    for line in std::str::from_utf8(bytes)?.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        ensure!(
            fields.len() == 3 && !fields[1].is_empty() && safe_relative(fields[2]) && fields[2].ends_with(".jar"),
            "invalid Minecraft dependency record"
        );
        ensure!(
            coordinates.insert(fields[1]) && paths.insert(fields[2].to_owned(), hash_text(fields[0])?).is_none(),
            "duplicate Minecraft dependency record"
        );
        ensure!(paths.len() <= 256, "Minecraft dependency list exceeds bounds");
    }
    ensure!(!paths.is_empty(), "empty Minecraft dependency list");
    Ok(paths)
}

#[derive(Deserialize)]
struct FabricProfile {
    id: String,
    #[serde(rename = "inheritsFrom")]
    inherits_from: String,
    #[serde(rename = "mainClass")]
    main_class: String,
    libraries: Vec<FabricLibrary>,
}

#[derive(Deserialize)]
struct FabricLibrary {
    name: String,
    url: String,
    sha256: Option<String>,
    size: Option<u64>,
}

fn fabric_records(bytes: &[u8], inventory: &Inventory) -> Result<BTreeSet<String>> {
    let profile: FabricProfile = serde_json::from_slice(bytes)?;
    ensure!(
        profile.id == format!("fabric-loader-{PINNED_FABRIC_LOADER_VERSION}-{PINNED_MINECRAFT_VERSION}")
            && profile.inherits_from == PINNED_MINECRAFT_VERSION
            && profile.main_class == "net.fabricmc.loader.impl.launch.knot.KnotServer",
        "Fabric profile does not match pinned server entry point"
    );
    let required: BTreeSet<_> = [
        "org.ow2.asm:asm",
        "org.ow2.asm:asm-analysis",
        "org.ow2.asm:asm-commons",
        "org.ow2.asm:asm-tree",
        "org.ow2.asm:asm-util",
        "net.fabricmc:sponge-mixin",
        "net.fabricmc:fabric-loader",
    ]
    .into_iter()
    .collect();
    ensure!(profile.libraries.len() == required.len(), "Fabric profile dependency count differs");
    let mut actual = BTreeSet::new();
    let mut files = BTreeSet::new();
    for lib in &profile.libraries {
        let parts: Vec<_> = lib.name.split(':').collect();
        ensure!(
            parts.len() == 3 && parts.iter().all(|part| token(part)) && lib.url == "https://maven.fabricmc.net/",
            "invalid Fabric dependency coordinate or source"
        );
        let id = format!("{}:{}", parts[0], parts[1]);
        ensure!(
            required.contains(id.as_str()) && actual.insert(id.clone()),
            "unexpected or duplicate Fabric dependency"
        );
        let path = format!("fabric-libraries/{}-{}.jar", parts[1], parts[2]);
        let sha = inventory.sha(&path)?;
        ensure!(files.insert(path.clone()), "duplicate Fabric dependency file");
        if id == "net.fabricmc:fabric-loader" {
            ensure!(parts[2] == PINNED_FABRIC_LOADER_VERSION && path == LOADER, "Fabric Loader version differs");
        } else {
            ensure!(lib.sha256.is_some() && lib.size.is_some(), "Fabric dependency lacks checksum or size");
        }
        if let Some(expected) = &lib.sha256 {
            ensure!(sha == hash_text(expected)?, "Fabric dependency checksum differs: {path}");
        }
        if let Some(size) = lib.size {
            ensure!(inventory.stamps[&path].bytes == size, "Fabric dependency size differs: {path}");
        }
    }
    Ok(files)
}

fn java_release(bytes: &[u8]) -> Result<String> {
    let mut fields = BTreeMap::new();
    for line in std::str::from_utf8(bytes)?.lines() {
        let (name, value) = line.split_once('=').context("invalid Java release property")?;
        ensure!(
            token(name) && value.starts_with('"') && value.ends_with('"') && value.len() >= 2,
            "invalid Java release property"
        );
        ensure!(fields.insert(name, &value[1..value.len() - 1]).is_none(), "duplicate Java release property");
    }
    let version = fields.get("JAVA_VERSION").context("Java release version missing")?;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => bail!("unqualified Linux package architecture"),
    };
    ensure!(
        token(version)
            && version.split('.').next() == Some("25")
            && fields.get("OS_NAME") == Some(&"Linux")
            && fields.get("OS_ARCH") == Some(&arch),
        "Java release metadata differs from Linux Java 25 baseline"
    );
    Ok((*version).to_owned())
}

fn expected_entries(files: &BTreeSet<String>) -> BTreeSet<String> {
    let mut expected = files.clone();
    for file in files {
        let mut parent = Path::new(file).parent();
        while let Some(dir) = parent {
            if dir.as_os_str().is_empty() {
                break;
            }
            expected.insert(dir.to_str().expect("validated UTF-8").to_owned());
            parent = dir.parent();
        }
    }
    expected
}

#[derive(Deserialize)]
struct NestedMod {
    file: String,
}
#[derive(Deserialize)]
struct BridgeMetadata {
    jars: Vec<NestedMod>,
}
#[derive(Deserialize)]
struct ApiMetadata {
    id: String,
    version: String,
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
}

pub fn inspect_runtime_package(package: &Path) -> Result<RuntimePackagePin> {
    let absolute = std::path::absolute(package)?;
    ensure!(!absolute.components().any(|c| c == Component::ParentDir), "unsafe runtime package traversal");
    validate_ancestors(&absolute)?;
    let root: PathBuf = absolute.canonicalize()?;
    let inventory = scan(&root)?;
    let minecraft = inventory.read(&root, MINECRAFT, MAX_CORE_JAR_BYTES)?;
    let loader = inventory.read(&root, LOADER, MAX_CORE_JAR_BYTES)?;
    let bridge = inventory.read(&root, BRIDGE, MAX_CORE_JAR_BYTES)?;
    let core = inspect_core_runtime(&minecraft, &loader, &bridge)?;
    let bridge_meta: BridgeMetadata = serde_json::from_slice(&read_zip_entry(&bridge, "fabric.mod.json")?)?;
    ensure!(bridge_meta.jars.len() == 1, "package requires the single embedded Fabric API JAR");
    let api_path = &bridge_meta.jars[0].file;
    ensure!(
        safe_relative(api_path) && api_path.starts_with("META-INF/jars/fabric-api-") && api_path.ends_with(".jar"),
        "invalid embedded Fabric API path"
    );
    let api_jar = read_zip_entry_bounded(&bridge, api_path, 8 * 1024 * 1024)?;
    let api: ApiMetadata = serde_json::from_slice(&read_zip_entry(&api_jar, "fabric.mod.json")?)?;
    ensure!(
        api.id == "fabric-api" && api.schema_version == 1 && token(&api.version),
        "embedded Fabric API metadata differs"
    );
    let bundler = inventory.read(&root, "server.jar", MAX_CORE_JAR_BYTES)?;
    ensure!(
        read_zip_entry(&bundler, "META-INF/main-class")? == b"net.minecraft.server.Main",
        "Minecraft bundler entry point differs"
    );
    let versions = minecraft_records(&read_zip_entry(&bundler, "META-INF/versions.list")?)?;
    ensure!(
        versions.len() == 1 && versions.get("26.1.2/server-26.1.2.jar") == Some(&inventory.sha(MINECRAFT)?),
        "extracted Minecraft JAR differs from bundler record"
    );
    let libraries = minecraft_records(&read_zip_entry(&bundler, "META-INF/libraries.list")?)?;
    let mut files: BTreeSet<_> = ["server.jar", MINECRAFT, BRIDGE, PROFILE].into_iter().map(str::to_owned).collect();
    for (relative, expected) in &libraries {
        let name = format!("libraries/{relative}");
        ensure!(inventory.sha(&name)? == *expected, "Minecraft dependency checksum differs: {name}");
        files.insert(name);
    }
    let fabric = fabric_records(&inventory.read(&root, PROFILE, MAX_METADATA)?, &inventory)?;
    files.extend(fabric.iter().cloned());
    let expected = expected_entries(&files);
    let actual: BTreeSet<_> =
        inventory.entries.keys().filter(|path| *path != "java" && !path.starts_with("java/")).cloned().collect();
    ensure!(actual == expected, "runtime package has missing or unexpected dependency, mod or configuration paths");
    for name in ["java/bin/java", "java/lib/modules", "java/lib/libjava.so", "java/lib/server/libjvm.so"] {
        inventory.sha(name)?;
    }
    ensure!(
        matches!(inventory.entries.get("java/bin/java"), Some(Entry::File { executable, .. }) if executable & 0o100 != 0),
        "Java executable is not owner-executable"
    );
    let java_version = java_release(&inventory.read(&root, "java/release", MAX_METADATA)?)?;
    inventory.stable(&root)?;
    let tree_hash = Hash32::from_domain_bytes(DOMAIN, &postcard::to_allocvec(&inventory.entries)?);
    let fingerprint = Hash32::from_domain_bytes(
        DOMAIN,
        &postcard::to_allocvec(&(std::env::consts::ARCH, core.fingerprint()?, tree_hash))?,
    );
    Ok(RuntimePackagePin {
        core,
        fingerprint,
        tree_hash,
        files: inventory.entries.values().filter(|entry| matches!(entry, Entry::File { .. })).count(),
        entries: inventory.entries.len(),
        bytes: inventory.total,
        minecraft_libraries: libraries.len(),
        fabric_libraries: fabric.len(),
        java_version,
        embedded_fabric_api_version: api.version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dependency_records_reject_unsafe_and_ambiguous_paths() {
        let hash = "ab".repeat(32);
        assert_eq!(minecraft_records(format!("{hash}\tg:a:1\ta/b.jar").as_bytes()).unwrap().len(), 1);
        for path in ["../escape.jar", "/absolute.jar", "a//b.jar", "a/./b.jar", "a\\b.jar", "a/b.txt"] {
            assert!(minecraft_records(format!("{hash}\tg:a:1\t{path}").as_bytes()).is_err());
        }
        assert!(minecraft_records(format!("{hash}\tg:a:1\ta.jar\n{hash}\tg:a:1\tb.jar").as_bytes()).is_err());
        assert!(minecraft_records(format!("{hash}\tg:a:1\ta.jar\n{hash}\tg:b:1\ta.jar").as_bytes()).is_err());
        assert!(minecraft_records(b"bad\tg:a:1\ta.jar").is_err());
    }

    #[test]
    fn inventory_rejects_untrusted_links_and_detects_change() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(root.join("file"), b"abc").unwrap();
        let inventory = scan(root).unwrap();
        assert!(inventory.stable(root).is_ok());
        fs::write(root.join("file"), b"def").unwrap();
        assert!(inventory.stable(root).is_err());
        fs::hard_link(root.join("file"), root.join("hardlink")).unwrap();
        assert!(scan(root).is_err());
        fs::remove_file(root.join("hardlink")).unwrap();
        symlink("file", root.join("symlink")).unwrap();
        assert!(scan(root).is_err());
        fs::remove_file(root.join("symlink")).unwrap();
        fs::set_permissions(root.join("file"), fs::Permissions::from_mode(0o666)).unwrap();
        assert!(scan(root).is_err());
    }
}
