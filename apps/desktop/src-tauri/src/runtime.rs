use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::AppHandle;
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

const CONNECTIVITY_DIAGNOSTICS_JSON_ENV: &str = "SWARMCRAFT_CONNECTIVITY_DIAGNOSTICS_JSON";

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
            connectivity_json: std::env::temp_dir().join(format!(
                "swarmcraft-connectivity-{}.json",
                std::process::id()
            )),
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
        spawn(app, "swarmcraft-host", arguments, &self.host, "Authority host")
    }

    pub fn start_managed_host(&self, app: &AppHandle, world: String) -> Result<u32, String> {
        spawn(
            app,
            "swarmcraft-runtime",
            vec!["launch".into(), world],
            &self.host,
            "Managed authority host",
        )
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

    let slot = slot.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            if matches!(event, CommandEvent::Terminated(_)) {
                if let Ok(mut state) = slot.state.lock() {
                    state.clear_if_generation(generation);
                }
                break;
            }
        }
    });
    Ok(pid)
}

fn stop(slot: &ProcessSlot, label: &str) -> Result<(), String> {
    let child = slot
        .state
        .lock()
        .map_err(|_| format!("{label} process state is poisoned"))?
        .take();
    match child {
        Some(child) => child.kill().map_err(|error| error.to_string()),
        None => Err(format!("{label} is not owned by this Desktop process")),
    }
}

#[cfg(test)]
mod tests {
    use super::SlotState;

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
}
