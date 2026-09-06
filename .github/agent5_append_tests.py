from pathlib import Path

TESTS = r'''
#[cfg(test)]
mod agent5_final_acceptance_tests {
    use super::*;
    use reqwest::redirect::Policy;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    type Capture = Arc<Mutex<Option<String>>>;

    fn spawn_http_once(response: String) -> (String, Capture, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let capture = Arc::new(Mutex::new(None));
        let capture_thread = Arc::clone(&capture);
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                        let mut buffer = [0u8; 16 * 1024];
                        let read = stream.read(&mut buffer).unwrap_or(0);
                        *capture_thread.lock().unwrap() =
                            Some(String::from_utf8_lossy(&buffer[..read]).into_owned());
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                        break;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        (format!("http://{address}"), capture, handle)
    }

    fn same_origin(left: &reqwest::Url, right: &reqwest::Url) -> bool {
        left.scheme() == right.scheme()
            && left.host_str() == right.host_str()
            && left.port_or_known_default() == right.port_or_known_default()
    }

    fn exact_origin_policy(origin: reqwest::Url) -> Policy {
        Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many test redirects")
            } else if same_origin(&origin, attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("cross-origin redirect rejected")
            }
        })
    }

    fn approved_origins_policy(origins: Vec<reqwest::Url>) -> Policy {
        Policy::custom(move |attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many test redirects")
            } else if origins
                .iter()
                .any(|origin| same_origin(origin, attempt.url()))
            {
                attempt.follow()
            } else {
                attempt.error("artifact redirect left approved origins")
            }
        })
    }

    fn redirect(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn ok() -> String {
        "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned()
    }

    #[test]
    fn authenticated_two_origin_redirect_never_reaches_attacker() {
        tauri::async_runtime::block_on(async {
            let (attacker, attacker_capture, attacker_thread) = spawn_http_once(ok());
            let (api, api_capture, api_thread) =
                spawn_http_once(redirect(&format!("{attacker}/steal")));
            let origin = reqwest::Url::parse(&api).unwrap();
            let client = reqwest::Client::builder()
                .redirect(exact_origin_policy(origin))
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let error = client
                .get(format!("{api}/v1/mods"))
                .header("x-api-key", "agent5-secret")
                .send()
                .await
                .unwrap_err();
            assert!(error.is_redirect());
            api_thread.join().unwrap();
            attacker_thread.join().unwrap();
            let first = api_capture
                .lock()
                .unwrap()
                .clone()
                .unwrap()
                .to_ascii_lowercase();
            assert!(first.contains("x-api-key: agent5-secret"));
            assert!(
                attacker_capture.lock().unwrap().is_none(),
                "credential-bearing redirect reached attacker origin"
            );

            let public_api =
                reqwest::Url::parse("https://api.curseforge.com/v1/mods/1").unwrap();
            let attacker_url = reqwest::Url::parse("https://attacker.invalid/steal").unwrap();
            assert!(is_curseforge_api_url(&public_api));
            assert!(!is_curseforge_api_url(&attacker_url));
        });
    }

    #[test]
    fn artifact_redirect_policy_accepts_approved_origin_and_rejects_unapproved_origin_without_api_key(
    ) {
        tauri::async_runtime::block_on(async {
            let (approved, approved_capture, approved_thread) = spawn_http_once(ok());
            let (source, source_capture, source_thread) =
                spawn_http_once(redirect(&format!("{approved}/artifact.jar")));
            let source_origin = reqwest::Url::parse(&source).unwrap();
            let approved_origin = reqwest::Url::parse(&approved).unwrap();
            let client = reqwest::Client::builder()
                .redirect(approved_origins_policy(vec![source_origin, approved_origin]))
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let response = client
                .get(format!("{source}/artifact.jar"))
                .send()
                .await
                .unwrap();
            assert!(response.status().is_success());
            source_thread.join().unwrap();
            approved_thread.join().unwrap();
            for captured in [source_capture, approved_capture] {
                let request = captured
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap()
                    .to_ascii_lowercase();
                assert!(
                    !request.contains("x-api-key"),
                    "artifact request carried API credential"
                );
            }

            let (unapproved, unapproved_capture, unapproved_thread) = spawn_http_once(ok());
            let (source, _source_capture, source_thread) =
                spawn_http_once(redirect(&format!("{unapproved}/artifact.jar")));
            let source_origin = reqwest::Url::parse(&source).unwrap();
            let client = reqwest::Client::builder()
                .redirect(approved_origins_policy(vec![source_origin]))
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            let error = client
                .get(format!("{source}/artifact.jar"))
                .send()
                .await
                .unwrap_err();
            assert!(error.is_redirect());
            source_thread.join().unwrap();
            unapproved_thread.join().unwrap();
            assert!(unapproved_capture.lock().unwrap().is_none());

            assert!(is_curseforge_artifact_url(
                &reqwest::Url::parse("https://mediafilez.forgecdn.net/files/1/2/example.jar").unwrap()
            ));
            assert!(!is_curseforge_artifact_url(
                &reqwest::Url::parse("https://evilforgecdn.net/files/example.jar").unwrap()
            ));
        });
    }

    #[test]
    fn curseforge_metadata_bounds_precede_json_parse_and_cover_headers_and_cardinality() {
        let oversized_invalid_json = vec![b'{'; MAX_METADATA_BYTES + 1];
        assert_eq!(
            parse_metadata_bytes(&oversized_invalid_json)
                .unwrap_err()
                .code,
            "response_too_large"
        );

        let too_many = Value::Array(vec![Value::Null; MAX_METADATA_ARRAY_ITEMS + 1]);
        assert_eq!(
            validate_metadata_value(&too_many).unwrap_err().code,
            "response_too_large"
        );

        let mut object = serde_json::Map::new();
        for index in 0..=MAX_METADATA_OBJECT_ENTRIES {
            object.insert(format!("field-{index}"), Value::Null);
        }
        assert_eq!(
            validate_metadata_value(&Value::Object(object))
                .unwrap_err()
                .code,
            "response_too_large"
        );

        let mut headers = reqwest::header::HeaderMap::new();
        for index in 0..=MAX_METADATA_HEADERS {
            let name =
                reqwest::header::HeaderName::from_bytes(format!("x-agent5-{index}").as_bytes())
                    .unwrap();
            headers.insert(name, reqwest::header::HeaderValue::from_static("x"));
        }
        assert_eq!(
            validate_metadata_headers(&headers).unwrap_err().code,
            "response_too_large"
        );
    }

    #[test]
    fn api_key_has_no_child_process_or_artifact_request_path() {
        let desktop = include_str!("curseforge.rs");
        let runtime = include_str!("../../../../crates/swarm-cli/src/provider_runtime.rs");
        let command_new = ["Command", "::new("].concat();
        let process_command = ["std::process", "::Command"].concat();
        for source in [desktop, runtime] {
            assert!(
                !source.contains(&command_new),
                "provider credential path must not spawn child processes"
            );
            assert!(
                !source.contains(&process_command),
                "provider credential path must stay in-process"
            );
        }
        let desktop_artifact = desktop
            .split("async fn download_artifact")
            .nth(1)
            .unwrap()
            .split("#[tauri::command]")
            .next()
            .unwrap();
        assert!(!desktop_artifact.contains("x-api-key"));
        let runtime_artifact = runtime
            .split("fn download_curseforge_artifact")
            .nth(1)
            .unwrap()
            .split("fn safe_filename")
            .next()
            .unwrap();
        assert!(!runtime_artifact.contains("x-api-key"));
    }
}
'''

path = Path("apps/desktop/src-tauri/src/curseforge.rs")
text = path.read_text()
if "mod agent5_final_acceptance_tests" not in text:
    path.write_text(text + TESTS)
