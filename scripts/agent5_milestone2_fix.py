from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

runtime = ROOT / "crates/swarm-cli/src/provider_runtime.rs"
text = runtime.read_text()
old = '''    let request = match method {
        "GET" => client.get(url),
        "POST" => client.post(url).json(&body.unwrap_or(Value::Null)),
        _ => bail!("unsupported CurseForge runtime HTTP method: {method}"),
    };
'''
new = '''    let request = match method {
        "GET" => client.get(url),
        "POST" => {
            let body = serde_json::to_string(&body.unwrap_or(Value::Null))?;
            client.post(url).header("Content-Type", "application/json").body(body)
        }
        _ => bail!("unsupported CurseForge runtime HTTP method: {method}"),
    };
'''
if old not in text:
    raise SystemExit("runtime reqwest JSON request anchor not found")
text = text.replace(old, new, 1)
text = text.replace(
    "fn read_curseforge_metadata(mut response: reqwest::blocking::Response) -> Result<Value> {",
    "fn read_curseforge_metadata(response: reqwest::blocking::Response) -> Result<Value> {",
    1,
)
old_fixture = r'validate_api_key("secret\\nforwarded: value")'
new_fixture = r'validate_api_key("secret\nforwarded: value")'
if old_fixture not in text:
    raise SystemExit("runtime newline-injection fixture anchor not found")
text = text.replace(old_fixture, new_fixture, 1)
runtime.write_text(text)

desktop = ROOT / "apps/desktop/src-tauri/src/curseforge.rs"
text = desktop.read_text()
old_fixture = r'normalize_api_key(Some("secret\\nheader".into()))'
new_fixture = r'normalize_api_key(Some("secret\nheader".into()))'
if old_fixture not in text:
    raise SystemExit("desktop newline-injection fixture anchor not found")
text = text.replace(old_fixture, new_fixture, 1)
desktop.write_text(text)

print("Agent 5 milestone 2 request body, injection fixtures, and lint corrected")
