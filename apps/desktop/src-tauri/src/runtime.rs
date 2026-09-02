use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

const CONNECTIVITY_DIAGNOSTICS_JSON_ENV: &str = "SWARMCRAFT_CONNECTIVITY_DIAGNOSTICS_JSON";
const MAX_RUNTIME_DIAGNOSTIC_BYTES: u64 = 4 * 1024 * 1024;

struct OwnedChild<T> {
    generation: u64,
    child: T,
}

struct SlotState<T> {
    next_generation: u64,
    owned: Option<OwnedChild<T>>,
}

impl<T> Default for SlotState<T> {
    fn default() -> Self {
        Self { next_generation: 0, owned: None }
    }
}

impl<T> SlotState<T> {
    fn existing_pid(&self, pid: impl FnOnce(&T) -> u32) -> Option<u32> {
        self.owned.as_ref().map(|owned| pid(&owned.child))
    }

    fn insert(&mut self, child: T) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let generation = self.next_generation;
        self.owned = Some(OwnedChild { generation, child });
        generation
    }

    fn clear_if_generation(&mut self, generation: u64) -> bool {
        if self.owned.as_ref().is_some_and(|owned| owned.generation == generation) {
            self.owned.take();
            true
        } else {
            false
        }
    }

    fn take(&mut self) -> Option<T> {
        self.owned.take().map(|owned| owned.child)
    }
}

#[derive(Clone, Default)]
struct ProcessSlot {
    state: Arc<Mutex<SlotState<CommandChild>>>,
}

#[derive(Clone)]
struct RuntimeDiagnostic {
    current: PathBuf,
    rotated: PathBuf,
}

pub struct RuntimeProcesses {
    daemon: ProcessSlot,
    host: ProcessSlot,
    connectivity_json: PathBuf,
}

impl Default for RuntimeProcesses {
    fn default() -> Self {
        Self {
            daemon: ProcessSlot::default(),
            host: ProcessSlot::default(),
            connectivity_json: std::env::temp_dir()
                .join(format!("swarmcraft-connectivity-{}.json", std::process::id())),
        }
    }
}

impl RuntimeProcesses {
    pub fn ensure_daemon_running(&self, app: &AppHandle, listen: String) -> Result<u32, String> {
        std::env::set_var(CONNECTIVITY_DIAGNOSTICS_JSON_ENV, &self.connectivity_json);
        spawn(
            app,
            "swarmcraft",
            vec!["daemon".into(), "--listen".into(), listen],
            &self.daemon,
            "Replication daemon",
            None,
        )
    }

    pub fn start_daemon(&self, app: &AppHandle, listen: String) -> Result<u32, String> {
        self.ensure_daemon_running(app, listen)
    }

    pub fn stop_daemon(&self) -> Result<(), String> {
        stop(&self.daemon, "Replication daemon")
    }

    pub fn connectivity_diagnostics_json(&self) -> Result<String, String> {
        fs::read_to_string(&self.connectivity_json).map_err(|error| {
            format!(
                "structured connectivity diagnostics are not available at {}: {error}",
                self.connectivity_json.display()
            )
        })
    }

    pub fn start_host(&self, app: &AppHandle, arguments: Vec<String>) -> Result<u32, String> {
        let world = arguments
            .windows(2)
            .find(|pair| pair[0] == "--world")
            .map(|pair| pair[1].clone())
            .ok_or_else(|| "Authority host arguments are missing --world for diagnostics ownership".to_owned())?;
        let diagnostic = runtime_diagnostic(app, &world)?;
        spawn(
            app,
            "swarmcraft-host",
            arguments,
            &self.host,
            "Authority host",
            Some(diagnostic),
        )
    }

    pub fn start_managed_host(&self, app: &AppHandle, world: String) -> Result<u32, String> {
        let diagnostic = runtime_diagnostic(app, &world)?;
        spawn(
            app,
            "swarmcraft-runtime",
            vec!["launch".into(), world],
            &self.host,
            "Managed authority host",
            Some(diagnostic),
        )
    }

    pub fn runtime_diagnostics_reference(&self, app: &AppHandle, world: &str) -> Result<String, String> {
        let diagnostic = runtime_diagnostic(app, world)?;
        serde_json::to_string(&serde_json::json!({
            "world": world,
            "current": diagnostic.current,
            "rotated": diagnostic.rotated,
            "currentExists": diagnostic.current.is_file(),
            "rotatedExists": diagnostic.rotated.is_file(),
            "maxBytesPerFile": MAX_RUNTIME_DIAGNOSTIC_BYTES,
            "retainedFiles": 2,
        }))
        .map_err(|error| error.to_string())
    }
}

impl Drop for RuntimeProcesses {
    fn drop(&mut self) {
        let _ = self.stop_daemon();
    }
}

fn spawn(
    app: &AppHandle,
    sidecar: &str,
    arguments: Vec<String>,
    slot: &ProcessSlot,
    label: &str,
    diagnostic: Option<RuntimeDiagnostic>,
) -> Result<u32, String> {
    let mut guard = slot.state.lock().map_err(|_| format!("{label} process state is poisoned"))?;
    if let Some(pid) = guard.existing_pid(CommandChild::pid) {
        return Ok(pid);
    }

    let (mut events, child) = app
        .shell()
        .sidecar(sidecar)
        .map_err(|error| error.to_string())?
        .args(arguments)
        .spawn()
        .map_err(|error| error.to_string())?;
    let pid = child.pid();
    let generation = guard.insert(child);
    drop(guard);

    if let Some(diagnostic) = diagnostic.as_ref() {
        let _ = append_runtime_diagnostic(diagnostic, "process", format!("started pid={pid}\n").as_bytes());
    }

    let slot = slot.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    if let Some(diagnostic) = diagnostic.as_ref() {
                        let _ = append_runtime_diagnostic(diagnostic, "stdout", &bytes);
                    }
                }
                CommandEvent::Stderr(bytes) => {
                    if let Some(diagnostic) = diagnostic.as_ref() {
                        let _ = append_runtime_diagnostic(diagnostic, "stderr", &bytes);
                    }
                }
                CommandEvent::Terminated(_) => {
                    if let Some(diagnostic) = diagnostic.as_ref() {
                        let _ = append_runtime_diagnostic(diagnostic, "process", b"terminated\n");
                    }
                    if let Ok(mut state) = slot.state.lock() {
                        state.clear_if_generation(generation);
                    }
                    break;
                }
                _ => {}
            }
        }
    });
    Ok(pid)
}

fn stop(slot: &ProcessSlot, label: &str) -> Result<(), String> {
    let child = slot.state.lock().map_err(|_| format!("{label} process state is poisoned"))?.take();
    match child {
        Some(child) => child.kill().map_err(|error| error.to_string()),
        None => Err(format!("{label} is not owned by this Desktop process")),
    }
}

fn runtime_diagnostic(app: &AppHandle, world: &str) -> Result<RuntimeDiagnostic, String> {
    let key = sanitize_world_key(world)?;
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("cannot resolve Desktop application data directory: {error}"))?
        .join("runtime-diagnostics");
    fs::create_dir_all(&root)
        .map_err(|error| format!("cannot create runtime diagnostics directory {}: {error}", root.display()))?;
    Ok(RuntimeDiagnostic {
        current: root.join(format!("{key}.log")),
        rotated: root.join(format!("{key}.log.1")),
    })
}

fn sanitize_world_key(world: &str) -> Result<String, String> {
    let value = world.trim();
    if value.is_empty() {
        return Err("World ID is required for runtime diagnostics".into());
    }
    if !value.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')) {
        return Err("World ID contains characters that are unsafe for a diagnostics filename".into());
    }
    Ok(value.to_owned())
}

fn append_runtime_diagnostic(diagnostic: &RuntimeDiagnostic, stream: &str, bytes: &[u8]) -> Result<(), String> {
    let redacted = redact_runtime_output(bytes);
    if redacted.is_empty() {
        return Ok(());
    }
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let entry = format!("[{timestamp}] [{stream}] {redacted}");
    rotate_runtime_diagnostic_if_needed(diagnostic, entry.len() as u64)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&diagnostic.current)
        .map_err(|error| format!("cannot open runtime diagnostics {}: {error}", diagnostic.current.display()))?;
    file.write_all(entry.as_bytes())
        .map_err(|error| format!("cannot append runtime diagnostics {}: {error}", diagnostic.current.display()))?;
    if !entry.ends_with('\n') {
        file.write_all(b"\n")
            .map_err(|error| format!("cannot terminate runtime diagnostics line: {error}"))?;
    }
    Ok(())
}

fn rotate_runtime_diagnostic_if_needed(diagnostic: &RuntimeDiagnostic, incoming: u64) -> Result<(), String> {
    let existing = fs::metadata(&diagnostic.current).map(|metadata| metadata.len()).unwrap_or(0);
    if existing == 0 || existing.saturating_add(incoming) <= MAX_RUNTIME_DIAGNOSTIC_BYTES {
        return Ok(());
    }
    if diagnostic.rotated.exists() {
        fs::remove_file(&diagnostic.rotated).map_err(|error| {
            format!("cannot remove old runtime diagnostics {}: {error}", diagnostic.rotated.display())
        })?;
    }
    fs::rename(&diagnostic.current, &diagnostic.rotated).map_err(|error| {
        format!(
            "cannot rotate runtime diagnostics {} -> {}: {error}",
            diagnostic.current.display(),
            diagnostic.rotated.display()
        )
    })?;
    Ok(())
}

fn redact_runtime_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut output = String::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let sensitive = lower.contains("swarmcraft_ipc_token")
            || lower.contains("authorization:")
            || lower.contains("authorization=")
            || lower.contains("bearer ")
            || lower.contains("ipc_token=")
            || lower.contains("ipc-token=");
        if sensitive {
            output.push_str("[REDACTED]");
        } else {
            output.push_str(line);
        }
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{redact_runtime_output, sanitize_world_key, SlotState};

    #[test]
    fn existing_owned_process_is_idempotent() {
        let mut state = SlotState::default();
        let generation = state.insert(41_u32);

        assert_eq!(state.existing_pid(|pid| *pid), Some(41));
        assert_eq!(generation, 1);
        assert_eq!(state.next_generation, 1, "idempotent lookup must not reserve another process generation");
    }

    #[test]
    fn stale_termination_cannot_clear_newer_owned_process() {
        let mut state = SlotState::default();
        let old_generation = state.insert(10_u32);
        assert_eq!(state.take(), Some(10));
        let new_generation = state.insert(20_u32);

        assert!(!state.clear_if_generation(old_generation));
        assert_eq!(state.existing_pid(|pid| *pid), Some(20));
        assert!(state.clear_if_generation(new_generation));
        assert_eq!(state.existing_pid(|pid| *pid), None);
    }

    #[test]
    fn stop_path_has_no_handle_for_external_processes() {
        let mut state = SlotState::<u32>::default();
        assert_eq!(state.take(), None, "an unrelated daemon must never appear as Desktop-owned");
    }

    #[test]
    fn runtime_diagnostics_filename_is_world_scoped_and_path_safe() {
        assert_eq!(sanitize_world_key("abcd-1234").unwrap(), "abcd-1234");
        assert!(sanitize_world_key("../escape").is_err());
        assert!(sanitize_world_key("world/other").is_err());
    }

    #[test]
    fn runtime_diagnostics_redact_controller_credentials() {
        let output = redact_runtime_output(
            b"normal line\nSWARMCRAFT_IPC_TOKEN=super-secret\nAuthorization: Bearer secret\nipc_token=secret\n",
        );
        assert!(output.contains("normal line"));
        assert!(!output.contains("super-secret"));
        assert!(!output.contains("Bearer secret"));
        assert!(!output.contains("ipc_token=secret"));
        assert_eq!(output.matches("[REDACTED]").count(), 3);
    }
}
