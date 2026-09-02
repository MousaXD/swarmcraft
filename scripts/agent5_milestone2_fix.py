from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
path = ROOT / "crates/swarm-cli/src/provider_runtime.rs"
text = path.read_text()
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
path.write_text(text.replace(old, new, 1))
print("Agent 5 milestone 2 runtime body encoding corrected")
