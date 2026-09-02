from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def edit(path, old, new):
    target = ROOT / path
    text = target.read_text()
    if old not in text:
        raise SystemExit(f"expected text not found in {path}: {old[:160]!r}")
    target.write_text(text.replace(old, new, 1))


def append(path, text):
    target = ROOT / path
    current = target.read_text()
    if text.strip() not in current:
        target.write_text(current + text)


# Shared dependency: swarm-cli and the excluded swarm-provider bridge both compile the shared Modrinth source.
edit(
    "crates/swarm-cli/Cargo.toml",
    "postcard.workspace = true\n",
    "postcard.workspace = true\nreqwest.workspace = true\n",
)
edit(
    "crates/swarm-provider/Cargo.toml",
    "serde_json = \"1.0\"\n",
    "serde_json = \"1.0\"\nreqwest = { version = \"=0.12.22\", default-features = false, features = [\"blocking\", \"rustls-tls\"] }\n",
)

# ---------------------------------------------------------------------------
# Shared Modrinth provider: in-process HTTP, same-origin redirects, bounded
# metadata headers/body/shape, and portable filenames.
# ---------------------------------------------------------------------------
edit(
    "crates/swarm-cli/src/package_provider/modrinth.rs",
    "use serde::de::DeserializeOwned;\nuse serde::Deserialize;\n",
    "use reqwest::{blocking::Client, redirect::Policy, Url};\nuse serde::de::DeserializeOwned;\nuse serde::Deserialize;\nuse serde_json::Value;\n",
)
edit(
    "crates/swarm-cli/src/package_provider/modrinth.rs",
    """    fs::{self, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
""",
    """    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
""",
)
edit(
    "crates/swarm-cli/src/package_provider/modrinth.rs",
    """pub const ABSOLUTE_MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
""",
    """pub const ABSOLUTE_MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_PROVIDER_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROVIDER_METADATA_HEADERS: usize = 128;
const MAX_PROVIDER_METADATA_HEADER_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_PROVIDER_METADATA_DEPTH: usize = 32;
const MAX_PROVIDER_METADATA_ARRAY_ITEMS: usize = 2048;
const MAX_PROVIDER_METADATA_OBJECT_ENTRIES: usize = 512;
const MAX_PROVIDER_METADATA_STRING_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_METADATA_NODES: usize = 50_000;
""",
)
start = '''#[derive(Debug, Clone)]
pub struct CurlTransport {
    user_agent: String,
}

impl CurlTransport {
    pub fn new(user_agent: impl Into<String>) -> ProviderResult<Self> {
        let user_agent = user_agent.into();
        if user_agent.trim().is_empty() {
            return Err(failure(ProviderFailureKind::InvalidRequest, "Modrinth User-Agent must identify SwarmCraft"));
        }
        Ok(Self { user_agent })
    }
}

impl ModrinthTransport for CurlTransport {
    fn get(&self, url: &str) -> ProviderResult<HttpResponse> {
        trusted_https(url, &["api.modrinth.com"])?;
        let headers_path = temporary_path("modrinth-headers");
        let body_path = temporary_path("modrinth-body");
        let output = Command::new("curl")
            .args(["-sS", "-L", "--proto", "=https", "--connect-timeout", "15", "--max-time", "60", "-A"])
            .arg(&self.user_agent)
            .arg("-D")
            .arg(&headers_path)
            .arg("-o")
            .arg(&body_path)
            .arg("--write-out")
            .arg("%{http_code}")
            .arg(url)
            .output()
            .map_err(|error| {
                failure(ProviderFailureKind::Unavailable, format!("cannot start curl for Modrinth: {error}"))
            })?;

        let status_text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !output.status.success() {
            let _ = fs::remove_file(&headers_path);
            let _ = fs::remove_file(&body_path);
            return Err(failure(
                ProviderFailureKind::Unavailable,
                format!("Modrinth request failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            ));
        }
        let status = status_text.parse::<u16>().map_err(|_| {
            failure(
                ProviderFailureKind::MalformedResponse,
                format!("curl returned an invalid HTTP status for Modrinth: {status_text}"),
            )
        })?;
        let headers = fs::read_to_string(&headers_path).map(|text| parse_headers(&text)).unwrap_or_default();
        let body = fs::read(&body_path).map_err(|error| {
            failure(ProviderFailureKind::Io, format!("cannot read Modrinth response body: {error}"))
        })?;
        let _ = fs::remove_file(&headers_path);
        let _ = fs::remove_file(&body_path);
        Ok(HttpResponse { status, headers, body })
    }

    fn download(&self, url: &str, destination: &Path, max_bytes: u64) -> ProviderResult<()> {
        trusted_https(url, &["cdn.modrinth.com"])?;
        let output = Command::new("curl")
            .args(["-sS", "-L", "--proto", "=https", "--connect-timeout", "15", "--max-time", "900", "--max-filesize"])
            .arg(max_bytes.to_string())
            .arg("-A")
            .arg(&self.user_agent)
            .arg("-o")
            .arg(destination)
            .arg("--write-out")
            .arg("%{http_code}")
            .arg(url)
            .output()
            .map_err(|error| {
                failure(ProviderFailureKind::DownloadInterrupted, format!("cannot start Modrinth download: {error}"))
            })?;

        if !output.status.success() {
            let _ = fs::remove_file(destination);
            return Err(failure(
                ProviderFailureKind::DownloadInterrupted,
                format!("Modrinth artifact download failed: {}", String::from_utf8_lossy(&output.stderr).trim()),
            ));
        }
        let status = String::from_utf8_lossy(&output.stdout).trim().parse::<u16>().map_err(|_| {
            failure(
                ProviderFailureKind::MalformedResponse,
                "Modrinth artifact download returned an invalid HTTP status",
            )
        })?;
        if !(200..300).contains(&status) {
            let _ = fs::remove_file(destination);
            return Err(http_failure(status, &BTreeMap::new(), "Modrinth artifact"));
        }
        let size = fs::metadata(destination)
            .map_err(|error| {
                failure(ProviderFailureKind::Io, format!("cannot inspect downloaded Modrinth artifact: {error}"))
            })?
            .len();
        if size > max_bytes {
            let _ = fs::remove_file(destination);
            return Err(failure(
                ProviderFailureKind::DownloadInterrupted,
                format!("Modrinth artifact exceeded the {max_bytes}-byte download bound"),
            ));
        }
        Ok(())
    }
}
'''
replacement = '''#[derive(Debug, Clone)]
pub struct CurlTransport {
    // Kept under the historical public type name for API compatibility. HTTP is
    // now in-process so redirect and response limits are enforceable per hop.
    client: Client,
}

impl CurlTransport {
    pub fn new(user_agent: impl Into<String>) -> ProviderResult<Self> {
        let user_agent = user_agent.into();
        if user_agent.trim().is_empty() {
            return Err(failure(ProviderFailureKind::InvalidRequest, "Modrinth User-Agent must identify SwarmCraft"));
        }
        let client = Client::builder()
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(15))
            .timeout(std::time::Duration::from_secs(900))
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many Modrinth redirects")
                } else if attempt
                    .previous()
                    .first()
                    .is_some_and(|origin| same_https_origin(origin, attempt.url()))
                {
                    attempt.follow()
                } else {
                    attempt.error("Modrinth redirect left the original trusted origin")
                }
            }))
            .user_agent(user_agent)
            .build()
            .map_err(|error| {
                failure(ProviderFailureKind::Unavailable, format!("cannot initialize Modrinth HTTP client: {error}"))
            })?;
        Ok(Self { client })
    }
}

impl ModrinthTransport for CurlTransport {
    fn get(&self, url: &str) -> ProviderResult<HttpResponse> {
        trusted_https(url, &["api.modrinth.com"])?;
        let mut response = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .send()
            .map_err(|error| failure(ProviderFailureKind::Unavailable, format!("Modrinth request failed: {error}")))?;
        let status = response.status().as_u16();
        let headers = bounded_headers(response.headers())?;
        let body = read_bounded_response(&mut response, MAX_PROVIDER_METADATA_BYTES, "Modrinth metadata")?;
        Ok(HttpResponse { status, headers, body })
    }

    fn download(&self, url: &str, destination: &Path, max_bytes: u64) -> ProviderResult<()> {
        trusted_https(url, &["cdn.modrinth.com"])?;
        let mut response = self
            .client
            .get(url)
            .header("Accept", "application/octet-stream")
            .send()
            .map_err(|error| {
                failure(ProviderFailureKind::DownloadInterrupted, format!("Modrinth artifact download failed: {error}"))
            })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(http_failure(status, &BTreeMap::new(), "Modrinth artifact"));
        }
        if response.content_length().is_some_and(|length| length > max_bytes) {
            return Err(failure(
                ProviderFailureKind::DownloadInterrupted,
                format!("Modrinth artifact exceeded the {max_bytes}-byte download bound"),
            ));
        }
        let result = (|| -> ProviderResult<()> {
            let mut output = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(destination)
                .map_err(|error| {
                    failure(ProviderFailureKind::Io, format!("cannot create Modrinth temporary artifact: {error}"))
                })?;
            let mut total = 0u64;
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let read = response.read(&mut buffer).map_err(|error| {
                    failure(ProviderFailureKind::DownloadInterrupted, format!("Modrinth artifact read failed: {error}"))
                })?;
                if read == 0 {
                    break;
                }
                total = total.checked_add(read as u64).ok_or_else(|| {
                    failure(ProviderFailureKind::DownloadInterrupted, "Modrinth artifact size counter overflowed")
                })?;
                if total > max_bytes {
                    return Err(failure(
                        ProviderFailureKind::DownloadInterrupted,
                        format!("Modrinth artifact exceeded the {max_bytes}-byte download bound"),
                    ));
                }
                output.write_all(&buffer[..read]).map_err(|error| {
                    failure(ProviderFailureKind::Io, format!("cannot write Modrinth temporary artifact: {error}"))
                })?;
            }
            output.sync_all().map_err(|error| {
                failure(ProviderFailureKind::Io, format!("cannot sync Modrinth temporary artifact: {error}"))
            })?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(destination);
        }
        result
    }
}
'''
edit("crates/swarm-cli/src/package_provider/modrinth.rs", start, replacement)

edit(
    "crates/swarm-cli/src/package_provider/modrinth.rs",
    '''        let parsed = serde_json::from_slice(&response.body).map_err(|error| {
            failure(ProviderFailureKind::MalformedResponse, format!("{label} returned malformed JSON: {error}"))
        })?;
        Ok((parsed, response.headers))
''',
    '''        validate_metadata_headers_map(&response.headers)?;
        if response.body.len() > MAX_PROVIDER_METADATA_BYTES {
            return Err(metadata_limit_failure(format!(
                "{label} exceeded the {MAX_PROVIDER_METADATA_BYTES}-byte metadata response bound"
            )));
        }
        let value: Value = serde_json::from_slice(&response.body).map_err(|error| {
            failure(ProviderFailureKind::MalformedResponse, format!("{label} returned malformed JSON: {error}"))
        })?;
        validate_metadata_value(&value)?;
        let parsed = serde_json::from_value(value).map_err(|error| {
            failure(ProviderFailureKind::MalformedResponse, format!("{label} returned invalid metadata: {error}"))
        })?;
        Ok((parsed, response.headers))
''',
)

old_helpers = '''fn parse_headers(text: &str) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    headers
}

fn header_u64(headers: &BTreeMap<String, String>, name: &str) -> Option<u64> {
    headers.get(name).and_then(|value| value.trim().parse().ok())
}

fn safe_filename(filename: &str) -> ProviderResult<()> {
    let path = Path::new(filename);
    let file_name = path.file_name().and_then(|value| value.to_str());
    if file_name != Some(filename) || filename.is_empty() {
        return Err(failure(ProviderFailureKind::MalformedResponse, "Modrinth returned an unsafe artifact filename"));
    }
    Ok(())
}
'''
new_helpers = '''fn header_u64(headers: &BTreeMap<String, String>, name: &str) -> Option<u64> {
    headers.get(name).and_then(|value| value.trim().parse().ok())
}

fn metadata_limit_failure(message: impl Into<String>) -> ProviderFailure {
    failure(ProviderFailureKind::MalformedResponse, format!("response_too_large: {}", message.into()))
}

fn bounded_headers(headers: &reqwest::header::HeaderMap) -> ProviderResult<BTreeMap<String, String>> {
    if headers.len() > MAX_PROVIDER_METADATA_HEADERS {
        return Err(metadata_limit_failure("Modrinth returned too many response headers"));
    }
    let mut mapped = BTreeMap::new();
    let mut total = 0usize;
    for (name, value) in headers {
        let bytes = value.as_bytes();
        total = total.saturating_add(name.as_str().len()).saturating_add(bytes.len());
        if bytes.len() > MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES || total > MAX_PROVIDER_METADATA_HEADER_BYTES {
            return Err(metadata_limit_failure("Modrinth response headers exceeded the metadata header budget"));
        }
        let value = value.to_str().map_err(|_| {
            failure(ProviderFailureKind::MalformedResponse, "Modrinth returned a non-text metadata header")
        })?;
        mapped.insert(name.as_str().to_ascii_lowercase(), value.to_owned());
    }
    validate_metadata_headers_map(&mapped)?;
    Ok(mapped)
}

fn validate_metadata_headers_map(headers: &BTreeMap<String, String>) -> ProviderResult<()> {
    if headers.len() > MAX_PROVIDER_METADATA_HEADERS {
        return Err(metadata_limit_failure("Modrinth returned too many response headers"));
    }
    let total = headers.iter().try_fold(0usize, |total, (name, value)| {
        if value.len() > MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES {
            return Err(metadata_limit_failure("Modrinth metadata header value is too large"));
        }
        Ok(total.saturating_add(name.len()).saturating_add(value.len()))
    })?;
    if total > MAX_PROVIDER_METADATA_HEADER_BYTES {
        return Err(metadata_limit_failure("Modrinth response headers exceeded the metadata header budget"));
    }
    Ok(())
}

fn read_bounded_response(
    response: &mut reqwest::blocking::Response,
    limit: usize,
    label: &str,
) -> ProviderResult<Vec<u8>> {
    if response.content_length().is_some_and(|length| length > limit as u64) {
        return Err(metadata_limit_failure(format!("{label} declared a body larger than {limit} bytes")));
    }
    let mut body = Vec::new();
    response
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|error| failure(ProviderFailureKind::Unavailable, format!("cannot read {label}: {error}")))?;
    if body.len() > limit {
        return Err(metadata_limit_failure(format!("{label} exceeded the {limit}-byte response bound")));
    }
    Ok(body)
}

fn validate_metadata_value(value: &Value) -> ProviderResult<()> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> ProviderResult<()> {
        if depth > MAX_PROVIDER_METADATA_DEPTH {
            return Err(metadata_limit_failure("Modrinth metadata nesting is too deep"));
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_PROVIDER_METADATA_NODES {
            return Err(metadata_limit_failure("Modrinth metadata contains too many values"));
        }
        match value {
            Value::String(text) if text.len() > MAX_PROVIDER_METADATA_STRING_BYTES => {
                Err(metadata_limit_failure("Modrinth metadata string is too large"))
            }
            Value::Array(items) => {
                if items.len() > MAX_PROVIDER_METADATA_ARRAY_ITEMS {
                    return Err(metadata_limit_failure("Modrinth metadata array is too large"));
                }
                for item in items {
                    visit(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            Value::Object(entries) => {
                if entries.len() > MAX_PROVIDER_METADATA_OBJECT_ENTRIES {
                    return Err(metadata_limit_failure("Modrinth metadata object has too many fields"));
                }
                for (key, item) in entries {
                    if key.len() > 256 {
                        return Err(metadata_limit_failure("Modrinth metadata key is too large"));
                    }
                    visit(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    let mut nodes = 0usize;
    visit(value, 0, &mut nodes)
}

fn safe_filename(filename: &str) -> ProviderResult<()> {
    let path = Path::new(filename);
    let stem = filename.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let windows_reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    let portable = filename.len() > 4
        && filename == filename.trim()
        && filename.len() <= 255
        && !path.is_absolute()
        && path.components().count() == 1
        && !filename.contains(['/', '\\\\', ':', '\\0'])
        && filename != "."
        && filename != ".."
        && !filename.ends_with(['.', ' '])
        && !windows_reserved
        && filename.to_ascii_lowercase().ends_with(".jar");
    if portable {
        Ok(())
    } else {
        Err(failure(ProviderFailureKind::MalformedResponse, "Modrinth returned an unsafe artifact filename"))
    }
}
'''
edit("crates/swarm-cli/src/package_provider/modrinth.rs", old_helpers, new_helpers)

old_trust = '''fn trusted_https(url: &str, hosts: &[&str]) -> ProviderResult<()> {
    let rest = url.strip_prefix("https://").ok_or_else(|| {
        failure(ProviderFailureKind::RetrievalRestricted, "Modrinth provider requests and downloads must use HTTPS")
    })?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default().split(':').next().unwrap_or_default();
    if hosts.iter().any(|allowed| host.eq_ignore_ascii_case(allowed)) {
        Ok(())
    } else {
        Err(failure(
            ProviderFailureKind::RetrievalRestricted,
            format!("Modrinth URL host is outside the provider trust boundary: {host}"),
        ))
    }
}
'''
new_trust = '''fn same_https_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == "https"
        && right.scheme() == "https"
        && left.username().is_empty()
        && right.username().is_empty()
        && left.password().is_none()
        && right.password().is_none()
        && left.host_str().zip(right.host_str()).is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
        && left.port_or_known_default() == Some(443)
        && right.port_or_known_default() == Some(443)
}

fn trusted_https(url: &str, hosts: &[&str]) -> ProviderResult<()> {
    let parsed = Url::parse(url).map_err(|_| {
        failure(ProviderFailureKind::RetrievalRestricted, "Modrinth returned an invalid provider URL")
    })?;
    let host = parsed.host_str().unwrap_or_default();
    let trusted = parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port_or_known_default() == Some(443)
        && hosts.iter().any(|allowed| host.eq_ignore_ascii_case(allowed));
    if trusted {
        Ok(())
    } else {
        Err(failure(
            ProviderFailureKind::RetrievalRestricted,
            format!("Modrinth URL is outside the provider trust boundary: {url}"),
        ))
    }
}
'''
edit("crates/swarm-cli/src/package_provider/modrinth.rs", old_trust, new_trust)

# temporary_path was only needed by the old curl metadata transport.
edit(
    "crates/swarm-cli/src/package_provider/modrinth.rs",
    '''fn temporary_path(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}-{}", unique_suffix()))
}

''',
    "",
)
append(
    "crates/swarm-cli/src/package_provider/modrinth.rs",
    r'''

#[cfg(test)]
mod agent5_http_security_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn modrinth_urls_and_redirects_stay_on_exact_origins() {
        assert!(trusted_https("https://api.modrinth.com/v2/search", &["api.modrinth.com"]).is_ok());
        assert!(trusted_https("https://cdn.modrinth.com/data/x.jar", &["cdn.modrinth.com"]).is_ok());
        assert!(trusted_https("https://cdn.modrinth.com@attacker.invalid/x.jar", &["cdn.modrinth.com"]).is_err());
        assert!(trusted_https("https://127.0.0.1/x.jar", &["cdn.modrinth.com"]).is_err());
        let api = Url::parse("https://api.modrinth.com/v2/search").unwrap();
        let same = Url::parse("https://api.modrinth.com/v2/project/x").unwrap();
        let other = Url::parse("https://attacker.invalid/steal").unwrap();
        assert!(same_https_origin(&api, &same));
        assert!(!same_https_origin(&api, &other));
    }

    #[test]
    fn modrinth_metadata_shape_and_portable_filename_are_bounded() {
        assert!(validate_metadata_value(&json!({"items": ["ok"]})).is_ok());
        let huge = json!({"text": "x".repeat(MAX_PROVIDER_METADATA_STRING_BYTES + 1)});
        assert!(validate_metadata_value(&huge).is_err());
        for invalid in ["../evil.jar", "..\\evil.jar", "C:\\evil.jar", "\\\\server\\evil.jar", "CON.jar"] {
            assert!(safe_filename(invalid).is_err(), "accepted {invalid}");
        }
        assert!(safe_filename("safe-mod.jar").is_ok());
    }
}
''',
)

# Integration-level oversized Modrinth response through the actual client parser.
edit(
    "crates/swarm-cli/tests/modrinth_provider.rs",
    "modrinth::{HttpResponse, ModrinthClient, ModrinthTransport},",
    "modrinth::{HttpResponse, ModrinthClient, ModrinthTransport, MAX_PROVIDER_METADATA_BYTES},",
)
append(
    "crates/swarm-cli/tests/modrinth_provider.rs",
    r'''

#[test]
fn oversized_provider_metadata_fails_before_json_deserialization() {
    let transport = FixtureTransport::default();
    transport.raw(200, BTreeMap::new(), vec![b' '; MAX_PROVIDER_METADATA_BYTES + 1]);
    let error = client(transport).project("P1").unwrap_err();
    assert_eq!(error.kind, ProviderFailureKind::MalformedResponse);
    assert!(error.message.contains("response_too_large"));
}
''',
)

# ---------------------------------------------------------------------------
# Desktop CurseForge: split authenticated API and unauthenticated artifact
# clients, exact-origin redirects for API, forgecdn-only artifact redirects,
# and bounded metadata parsing.
# ---------------------------------------------------------------------------
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    """const MAX_DEPENDENCY_PACKAGES: usize = 128;
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
""",
    """const MAX_DEPENDENCY_PACKAGES: usize = 128;
const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_METADATA_HEADERS: usize = 128;
const MAX_METADATA_HEADER_BYTES: usize = 64 * 1024;
const MAX_METADATA_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_METADATA_DEPTH: usize = 32;
const MAX_METADATA_ARRAY_ITEMS: usize = 2048;
const MAX_METADATA_OBJECT_ENTRIES: usize = 512;
const MAX_METADATA_STRING_BYTES: usize = 64 * 1024;
const MAX_METADATA_NODES: usize = 50_000;
""",
)
old_client = '''struct CurseForgeClient {
    http: reqwest::Client,
    api_key: Option<String>,
}

impl CurseForgeClient {
    fn from_environment() -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many redirects")
                } else if attempt.url().scheme() != "https" {
                    attempt.error("CurseForge download redirected away from HTTPS")
                } else {
                    attempt.follow()
                }
            }))
            .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " CurseForgeProvider"))
            .build()
            .map_err(|error| {
                ProviderError::new(
                    "unavailable",
                    "provider_initialization_failed",
                    format!("Could not initialize CurseForge HTTP client: {error}"),
                )
            })?;
        Ok(Self { http, api_key: normalize_api_key(env::var(API_KEY_ENV).ok()) })
    }
'''
new_client = '''struct CurseForgeClient {
    api_http: reqwest::Client,
    artifact_http: reqwest::Client,
    api_key: Option<String>,
}

impl CurseForgeClient {
    fn from_environment() -> Result<Self, ProviderError> {
        let common = || {
            reqwest::Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " CurseForgeProvider"))
        };
        let api_http = common()
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many CurseForge API redirects")
                } else if is_curseforge_api_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("authenticated CurseForge API redirect left api.curseforge.com")
                }
            }))
            .build()
            .map_err(|error| {
                ProviderError::new(
                    "unavailable",
                    "provider_initialization_failed",
                    format!("Could not initialize authenticated CurseForge HTTP client: {error}"),
                )
            })?;
        let artifact_http = common()
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("too many CurseForge artifact redirects")
                } else if is_curseforge_artifact_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("CurseForge artifact redirect left the Forge CDN trust boundary")
                }
            }))
            .build()
            .map_err(|error| {
                ProviderError::new(
                    "unavailable",
                    "provider_initialization_failed",
                    format!("Could not initialize CurseForge artifact HTTP client: {error}"),
                )
            })?;
        Ok(Self { api_http, artifact_http, api_key: normalize_api_key(env::var(API_KEY_ENV).ok()) })
    }
'''
edit("apps/desktop/src-tauri/src/curseforge.rs", old_client, new_client)
edit("apps/desktop/src-tauri/src/curseforge.rs", ".http\n            .get(format!(\"{API_BASE}{path}\"))", ".api_http\n            .get(format!(\"{API_BASE}{path}\"))")
edit("apps/desktop/src-tauri/src/curseforge.rs", ".http\n            .post(format!(\"{API_BASE}{path}\"))", ".api_http\n            .post(format!(\"{API_BASE}{path}\"))")
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    '''        let response = self
            .http
            .get(format!("{API_BASE}/v1/mods/{project_id}/files/{file_id}/download-url"))''',
    '''        let response = self
            .api_http
            .get(format!("{API_BASE}/v1/mods/{project_id}/files/{file_id}/download-url"))''',
)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    '''fn normalize_api_key(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
}
''',
    '''fn normalize_api_key(raw: Option<String>) -> Option<String> {
    raw.map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
}

fn is_curseforge_api_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("api.curseforge.com"))
}

fn is_curseforge_artifact_url(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && (host == "forgecdn.net" || host.ends_with(".forgecdn.net"))
}
''',
)
old_parse = '''async fn parse_json_response(response: reqwest::Response, missing: MissingResource) -> Result<Value, ProviderError> {
    if !response.status().is_success() {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        return Err(map_http_status(response.status().as_u16(), missing, retry_after));
    }
    response.json::<Value>().await.map_err(|_| malformed_response())
}
'''
new_parse = '''fn metadata_limit_error(message: impl Into<String>) -> ProviderError {
    ProviderError::new("error", "response_too_large", message)
}

fn validate_metadata_headers(headers: &reqwest::header::HeaderMap) -> Result<(), ProviderError> {
    if headers.len() > MAX_METADATA_HEADERS {
        return Err(metadata_limit_error("CurseForge returned too many metadata response headers"));
    }
    let mut total = 0usize;
    for (name, value) in headers {
        total = total.saturating_add(name.as_str().len()).saturating_add(value.as_bytes().len());
        if value.as_bytes().len() > MAX_METADATA_HEADER_VALUE_BYTES || total > MAX_METADATA_HEADER_BYTES {
            return Err(metadata_limit_error("CurseForge metadata response headers exceeded their byte budget"));
        }
    }
    Ok(())
}

fn validate_metadata_value(value: &Value) -> Result<(), ProviderError> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), ProviderError> {
        if depth > MAX_METADATA_DEPTH {
            return Err(metadata_limit_error("CurseForge metadata nesting is too deep"));
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_METADATA_NODES {
            return Err(metadata_limit_error("CurseForge metadata contains too many values"));
        }
        match value {
            Value::String(text) if text.len() > MAX_METADATA_STRING_BYTES => {
                Err(metadata_limit_error("CurseForge metadata string is too large"))
            }
            Value::Array(items) => {
                if items.len() > MAX_METADATA_ARRAY_ITEMS {
                    return Err(metadata_limit_error("CurseForge metadata array is too large"));
                }
                for item in items {
                    visit(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            Value::Object(entries) => {
                if entries.len() > MAX_METADATA_OBJECT_ENTRIES {
                    return Err(metadata_limit_error("CurseForge metadata object has too many fields"));
                }
                for (key, item) in entries {
                    if key.len() > 256 {
                        return Err(metadata_limit_error("CurseForge metadata key is too large"));
                    }
                    visit(item, depth + 1, nodes)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    let mut nodes = 0usize;
    visit(value, 0, &mut nodes)
}

fn parse_metadata_bytes(bytes: &[u8]) -> Result<Value, ProviderError> {
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(metadata_limit_error(format!(
            "CurseForge metadata exceeded the {MAX_METADATA_BYTES}-byte response bound"
        )));
    }
    let value = serde_json::from_slice::<Value>(bytes).map_err(|_| malformed_response())?;
    validate_metadata_value(&value)?;
    Ok(value)
}

async fn parse_json_response(mut response: reqwest::Response, missing: MissingResource) -> Result<Value, ProviderError> {
    validate_metadata_headers(response.headers())?;
    if !response.status().is_success() {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        return Err(map_http_status(response.status().as_u16(), missing, retry_after));
    }
    if response.content_length().is_some_and(|length| length > MAX_METADATA_BYTES as u64) {
        return Err(metadata_limit_error("CurseForge metadata Content-Length exceeded the response bound"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(map_request_error)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_METADATA_BYTES {
            return Err(metadata_limit_error(format!(
                "CurseForge metadata exceeded the {MAX_METADATA_BYTES}-byte response bound"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_metadata_bytes(&bytes)
}
'''
edit("apps/desktop/src-tauri/src/curseforge.rs", old_parse, new_parse)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    '''        _ => ProviderError::new("error", "provider_request_failed", format!("CurseForge returned HTTP {status}")),''',
    '''        300..=399 => ProviderError::new(
            "error",
            "redirect_rejected",
            "CurseForge redirect was rejected by the provider origin policy",
        ),
        _ => ProviderError::new("error", "provider_request_failed", format!("CurseForge returned HTTP {status}")),''',
)
old_validate_url = '''fn validate_download_url(url: &str) -> Result<String, ProviderError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ProviderError::new("download_failed", "untrusted_download_url", "CurseForge returned an invalid artifact URL")
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(ProviderError::new(
            "download_failed",
            "untrusted_download_url",
            "CurseForge automatic downloads must use provider-returned HTTPS URLs",
        ));
    }
    Ok(parsed.to_string())
}
'''
new_validate_url = '''fn validate_download_url(url: &str) -> Result<String, ProviderError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| {
        ProviderError::new("download_failed", "untrusted_download_url", "CurseForge returned an invalid artifact URL")
    })?;
    if !is_curseforge_artifact_url(&parsed) {
        return Err(ProviderError::new(
            "download_failed",
            "untrusted_download_url",
            "CurseForge automatic downloads must stay on the HTTPS forgecdn.net artifact boundary",
        ));
    }
    Ok(parsed.to_string())
}
'''
edit("apps/desktop/src-tauri/src/curseforge.rs", old_validate_url, new_validate_url)
edit(
    "apps/desktop/src-tauri/src/curseforge.rs",
    '''    let mut response =
        client.http.get(url).header("Accept", "application/octet-stream").send().await.map_err(map_download_error)?;''',
    '''    let mut response = client
        .artifact_http
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(map_download_error)?;''',
)
append(
    "apps/desktop/src-tauri/src/curseforge.rs",
    r'''

#[cfg(test)]
mod agent5_http_security_tests {
    use super::*;

    #[test]
    fn authenticated_api_and_artifact_origins_are_disjoint() {
        let api = reqwest::Url::parse("https://api.curseforge.com/v1/mods/1").unwrap();
        let second_origin = reqwest::Url::parse("https://attacker.invalid/steal").unwrap();
        let forge = reqwest::Url::parse("https://edge.forgecdn.net/files/example.jar").unwrap();
        let private = reqwest::Url::parse("https://127.0.0.1/example.jar").unwrap();
        assert!(is_curseforge_api_url(&api));
        assert!(!is_curseforge_api_url(&second_origin));
        assert!(is_curseforge_artifact_url(&forge));
        assert!(!is_curseforge_artifact_url(&second_origin));
        assert!(!is_curseforge_artifact_url(&private));
    }

    #[test]
    fn provider_metadata_bytes_and_shape_are_bounded() {
        assert_eq!(
            parse_metadata_bytes(&vec![b' '; MAX_METADATA_BYTES + 1]).unwrap_err().code,
            "response_too_large"
        );
        let huge = json!({"text": "x".repeat(MAX_METADATA_STRING_BYTES + 1)});
        assert_eq!(validate_metadata_value(&huge).unwrap_err().code, "response_too_large");
    }

    #[test]
    fn api_key_control_characters_are_rejected() {
        assert_eq!(normalize_api_key(Some("secret\\nheader".into())), None);
        assert_eq!(normalize_api_key(Some(" secret ".into())), Some("secret".into()));
    }
}
''',
)

# ---------------------------------------------------------------------------
# Runtime CurseForge: remove curl entirely for authenticated requests so the
# API key cannot appear in argv; use split in-process clients with the same
# origin policies and bounded metadata/artifact streaming.
# ---------------------------------------------------------------------------
edit(
    "crates/swarm-cli/src/provider_runtime.rs",
    "use serde_json::{json, Value};\n",
    "use reqwest::{blocking::Client, redirect::Policy, Url};\nuse serde_json::{json, Value};\n",
)
edit(
    "crates/swarm-cli/src/provider_runtime.rs",
    '''    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
''',
    '''    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
''',
)
edit(
    "crates/swarm-cli/src/provider_runtime.rs",
    '''const CURSEFORGE_API_KEY_ENV: &str = "SWARMCRAFT_CURSEFORGE_API_KEY";
''',
    '''const CURSEFORGE_API_KEY_ENV: &str = "SWARMCRAFT_CURSEFORGE_API_KEY";
const MAX_PROVIDER_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_PROVIDER_METADATA_HEADERS: usize = 128;
const MAX_PROVIDER_METADATA_HEADER_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES: usize = 16 * 1024;
const MAX_PROVIDER_METADATA_DEPTH: usize = 32;
const MAX_PROVIDER_METADATA_ARRAY_ITEMS: usize = 2048;
const MAX_PROVIDER_METADATA_OBJECT_ENTRIES: usize = 512;
const MAX_PROVIDER_METADATA_STRING_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_METADATA_NODES: usize = 50_000;
''',
)
edit(
    "crates/swarm-cli/src/provider_runtime.rs",
    '''    let api_key = env::var(CURSEFORGE_API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "CurseForge runtime acquisition requires the machine-local {CURSEFORGE_API_KEY_ENV} environment variable"
            )
        })?;
''',
    '''    let api_key = env::var(CURSEFORGE_API_KEY_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "CurseForge runtime acquisition requires the machine-local {CURSEFORGE_API_KEY_ENV} environment variable"
            )
        })?;
    validate_api_key(&api_key)?;
''',
)
edit(
    "crates/swarm-cli/src/provider_runtime.rs",
    '''    if !download_url.starts_with("https://") || download_url.chars().any(char::is_whitespace) {
        bail!("CurseForge returned an untrusted non-HTTPS artifact URL");
    }

    let destination = staging.join(&artifact.file_name);
    let status = curl_download(&download_url, &destination)?;
''',
    '''    validate_curseforge_artifact_url(&download_url)?;

    let destination = staging.join(&artifact.file_name);
    let status = download_curseforge_artifact(&download_url, &destination)?;
''',
)
edit(
    "crates/swarm-cli/src/provider_runtime.rs",
    '''    if file_name != artifact.file_name {
''',
    '''    safe_filename(file_name)?;
    if file_name != artifact.file_name {
''',
)
old_runtime_http = '''fn curseforge_json(method: &str, url: &str, api_key: &str, body: Option<Value>) -> Result<(u16, Value)> {
    let output_path = temporary_path("curseforge-json");
    let mut command = Command::new("curl");
    command.args([
        "-sS",
        "-L",
        "--proto",
        "=https",
        "--connect-timeout",
        "15",
        "--max-time",
        "120",
        "-H",
        "Accept: application/json",
    ]);
    command.arg("-H").arg(format!("x-api-key: {api_key}"));
    if method == "POST" {
        command.args(["-X", "POST", "-H", "Content-Type: application/json"]);
        command.arg("--data").arg(serde_json::to_string(&body.unwrap_or(Value::Null))?);
    }
    let output = command
        .arg("-o")
        .arg(&output_path)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg(url)
        .output()
        .with_context(|| format!("cannot start curl for CurseForge {method} request"))?;
    let body_bytes = fs::read(&output_path).unwrap_or_default();
    let _ = fs::remove_file(&output_path);
    if !output.status.success() {
        bail!("CurseForge request failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    let status = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .context("curl returned an invalid CurseForge HTTP status")?;
    let value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).context("CurseForge returned malformed JSON")?
    };
    Ok((status, value))
}
'''
new_runtime_http = '''fn validate_api_key(api_key: &str) -> Result<()> {
    if api_key.is_empty() || api_key.chars().any(char::is_control) {
        bail!("{CURSEFORGE_API_KEY_ENV} contains invalid control characters");
    }
    Ok(())
}

fn is_curseforge_api_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.host_str().is_some_and(|host| host.eq_ignore_ascii_case("api.curseforge.com"))
}

fn is_curseforge_artifact_url(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && (host == "forgecdn.net" || host.ends_with(".forgecdn.net"))
}

fn validate_curseforge_api_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("CurseForge API URL is invalid")?;
    if !is_curseforge_api_url(&parsed) {
        bail!("CurseForge authenticated API URL left the exact api.curseforge.com origin");
    }
    Ok(parsed)
}

fn validate_curseforge_artifact_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url).context("CurseForge artifact URL is invalid")?;
    if !is_curseforge_artifact_url(&parsed) {
        bail!("CurseForge artifact URL left the HTTPS forgecdn.net trust boundary");
    }
    Ok(parsed)
}

fn curseforge_api_client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many CurseForge API redirects")
            } else if is_curseforge_api_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("authenticated CurseForge API redirect left api.curseforge.com")
            }
        }))
        .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " RuntimeCurseForge"))
        .build()
        .context("cannot initialize authenticated CurseForge HTTP client")
}

fn curseforge_artifact_client() -> Result<Client> {
    Client::builder()
        .https_only(true)
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(900))
        .redirect(Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many CurseForge artifact redirects")
            } else if is_curseforge_artifact_url(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("CurseForge artifact redirect left the Forge CDN trust boundary")
            }
        }))
        .user_agent(concat!("SwarmCraft/", env!("CARGO_PKG_VERSION"), " RuntimeCurseForge"))
        .build()
        .context("cannot initialize CurseForge artifact HTTP client")
}

fn validate_metadata_headers(headers: &reqwest::header::HeaderMap) -> Result<()> {
    if headers.len() > MAX_PROVIDER_METADATA_HEADERS {
        bail!("response_too_large: CurseForge returned too many metadata headers");
    }
    let mut total = 0usize;
    for (name, value) in headers {
        total = total.saturating_add(name.as_str().len()).saturating_add(value.as_bytes().len());
        if value.as_bytes().len() > MAX_PROVIDER_METADATA_HEADER_VALUE_BYTES
            || total > MAX_PROVIDER_METADATA_HEADER_BYTES
        {
            bail!("response_too_large: CurseForge metadata headers exceeded their byte budget");
        }
    }
    Ok(())
}

fn validate_metadata_value(value: &Value) -> Result<()> {
    fn visit(value: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
        if depth > MAX_PROVIDER_METADATA_DEPTH {
            bail!("response_too_large: CurseForge metadata nesting is too deep");
        }
        *nodes = nodes.saturating_add(1);
        if *nodes > MAX_PROVIDER_METADATA_NODES {
            bail!("response_too_large: CurseForge metadata contains too many values");
        }
        match value {
            Value::String(text) if text.len() > MAX_PROVIDER_METADATA_STRING_BYTES => {
                bail!("response_too_large: CurseForge metadata string is too large")
            }
            Value::Array(items) => {
                if items.len() > MAX_PROVIDER_METADATA_ARRAY_ITEMS {
                    bail!("response_too_large: CurseForge metadata array is too large");
                }
                for item in items {
                    visit(item, depth + 1, nodes)?;
                }
            }
            Value::Object(entries) => {
                if entries.len() > MAX_PROVIDER_METADATA_OBJECT_ENTRIES {
                    bail!("response_too_large: CurseForge metadata object has too many fields");
                }
                for (key, item) in entries {
                    if key.len() > 256 {
                        bail!("response_too_large: CurseForge metadata key is too large");
                    }
                    visit(item, depth + 1, nodes)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut nodes = 0usize;
    visit(value, 0, &mut nodes)
}

fn read_curseforge_metadata(mut response: reqwest::blocking::Response) -> Result<Value> {
    validate_metadata_headers(response.headers())?;
    if response.content_length().is_some_and(|length| length > MAX_PROVIDER_METADATA_BYTES as u64) {
        bail!("response_too_large: CurseForge metadata Content-Length exceeded the response bound");
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PROVIDER_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("cannot read CurseForge metadata response")?;
    if bytes.len() > MAX_PROVIDER_METADATA_BYTES {
        bail!("response_too_large: CurseForge metadata exceeded the response byte bound");
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    let value: Value = serde_json::from_slice(&bytes).context("CurseForge returned malformed JSON")?;
    validate_metadata_value(&value)?;
    Ok(value)
}

fn curseforge_json(method: &str, url: &str, api_key: &str, body: Option<Value>) -> Result<(u16, Value)> {
    validate_api_key(api_key)?;
    let url = validate_curseforge_api_url(url)?;
    let client = curseforge_api_client()?;
    let request = match method {
        "GET" => client.get(url),
        "POST" => client.post(url).json(&body.unwrap_or(Value::Null)),
        _ => bail!("unsupported CurseForge runtime HTTP method: {method}"),
    };
    let response = request
        .header("Accept", "application/json")
        .header("x-api-key", api_key)
        .send()
        .with_context(|| format!("CurseForge {method} request failed"))?;
    let status = response.status().as_u16();
    let value = if (200..300).contains(&status) {
        read_curseforge_metadata(response)?
    } else {
        validate_metadata_headers(response.headers())?;
        Value::Null
    };
    Ok((status, value))
}
'''
edit("crates/swarm-cli/src/provider_runtime.rs", old_runtime_http, new_runtime_http)
old_curl_download = '''fn curl_download(url: &str, destination: &Path) -> Result<u16> {
    let output = Command::new("curl")
        .args(["-sS", "-L", "--proto", "=https", "--connect-timeout", "15", "--max-time", "900", "--max-filesize"])
        .arg(MAX_PROVIDER_ARTIFACT_BYTES.to_string())
        .arg("-o")
        .arg(destination)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg(url)
        .output()
        .context("cannot start curl for CurseForge artifact download")?;
    if !output.status.success() {
        let _ = fs::remove_file(destination);
        bail!("CurseForge artifact download failed: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u16>()
        .context("curl returned an invalid CurseForge artifact HTTP status")
}

fn safe_filename(value: &str) -> Result<()> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || path.is_absolute()
        || path.components().count() != 1
        || !value.to_ascii_lowercase().ends_with(".jar")
    {
        bail!("provider artifact filename is not a safe JAR basename: {value}");
    }
    Ok(())
}
'''
new_curl_download = '''fn download_curseforge_artifact(url: &str, destination: &Path) -> Result<u16> {
    let url = validate_curseforge_artifact_url(url)?;
    let client = curseforge_artifact_client()?;
    let mut response = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .context("CurseForge artifact download failed")?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Ok(status);
    }
    if response.content_length().is_some_and(|length| length > MAX_PROVIDER_ARTIFACT_BYTES) {
        bail!("CurseForge artifact exceeded the provider download byte bound");
    }
    let result = (|| -> Result<()> {
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .with_context(|| format!("cannot create provider artifact {}", destination.display()))?;
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer).context("CurseForge artifact stream failed")?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64).ok_or_else(|| anyhow!("CurseForge artifact size overflow"))?;
            if total > MAX_PROVIDER_ARTIFACT_BYTES {
                bail!("CurseForge artifact exceeded the provider download byte bound");
            }
            output.write_all(&buffer[..read]).context("cannot write CurseForge provider artifact")?;
        }
        output.sync_all().context("cannot sync CurseForge provider artifact")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result?;
    Ok(status)
}

fn safe_filename(value: &str) -> Result<()> {
    let path = Path::new(value);
    let stem = value.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let windows_reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    let portable = value.len() > 4
        && value == value.trim()
        && value.len() <= 255
        && !path.is_absolute()
        && path.components().count() == 1
        && !value.contains(['/', '\\\\', ':', '\\0'])
        && value != "."
        && value != ".."
        && !value.ends_with(['.', ' '])
        && !windows_reserved
        && value.to_ascii_lowercase().ends_with(".jar");
    if !portable {
        bail!("provider artifact filename is not a safe portable JAR basename: {value}");
    }
    Ok(())
}
'''
edit("crates/swarm-cli/src/provider_runtime.rs", old_curl_download, new_curl_download)
append(
    "crates/swarm-cli/src/provider_runtime.rs",
    r'''

#[cfg(test)]
mod agent5_runtime_http_security_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn curseforge_runtime_origins_reject_second_origin_and_private_targets() {
        assert!(validate_curseforge_api_url("https://api.curseforge.com/v1/mods/files").is_ok());
        assert!(validate_curseforge_api_url("https://attacker.invalid/steal").is_err());
        assert!(validate_curseforge_artifact_url("https://mediafilez.forgecdn.net/files/example.jar").is_ok());
        assert!(validate_curseforge_artifact_url("https://127.0.0.1/example.jar").is_err());
        assert!(validate_curseforge_artifact_url("https://api.curseforge.com/example.jar").is_err());
    }

    #[test]
    fn runtime_metadata_and_filename_limits_fail_closed() {
        let huge = json!({"text": "x".repeat(MAX_PROVIDER_METADATA_STRING_BYTES + 1)});
        assert!(validate_metadata_value(&huge).unwrap_err().to_string().contains("response_too_large"));
        for invalid in ["../evil.jar", "..\\evil.jar", "C:\\evil.jar", "\\\\server\\evil.jar", "NUL.jar"] {
            assert!(safe_filename(invalid).is_err(), "accepted {invalid}");
        }
        assert!(safe_filename("safe-runtime.jar").is_ok());
    }

    #[test]
    fn runtime_api_key_rejects_header_injection() {
        assert!(validate_api_key("secret").is_ok());
        assert!(validate_api_key("secret\\nforwarded: value").is_err());
    }
}
''',
)

print("Agent 5 milestone 2 patch applied")
