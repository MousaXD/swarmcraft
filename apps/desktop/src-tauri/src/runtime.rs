use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

#[derive(Clone, Default)]
struct ProcessSlot {
    child: Arc<Mutex<Option<CommandChild>>>,
}

#[derive(Clone, Default)]
pub struct RuntimeProcesses {
    daemon: ProcessSlot,
    host: ProcessSlot,
}

impl RuntimeProcesses {
    pub fn start_daemon(&self, app: &AppHandle, listen: String) -> Result<u32, String> {
        spawn(
            app,
            "swarmcraft",
            vec!["daemon".into(), "--listen".into(), listen],
            &self.daemon,
            "Replication daemon",
        )
    }

    pub fn stop_daemon(&self) -> Result<(), String> {
        stop(&self.daemon, "Replication daemon")
    }

    pub fn start_host(&self, app: &AppHandle, arguments: Vec<String>) -> Result<u32, String> {
        spawn(app, "swarmcraft-host", arguments, &self.host, "Authority host")
    }

    pub fn stop_host(&self) -> Result<(), String> {
        stop(&self.host, "Authority host")
    }
}

fn spawn(
    app: &AppHandle,
    sidecar: &str,
    arguments: Vec<String>,
    slot: &ProcessSlot,
    label: &str,
) -> Result<u32, String> {
    let mut guard = slot.child.lock().map_err(|_| format!("{label} process state is poisoned"))?;
    if guard.is_some() {
        return Err(format!("{label} is already running"));
    }

    let (mut events, child) = app
        .shell()
        .sidecar(sidecar)
        .map_err(|error| error.to_string())?
        .args(arguments)
        .spawn()
        .map_err(|error| error.to_string())?;
    let pid = child.pid();
    *guard = Some(child);
    drop(guard);

    let slot = slot.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            if matches!(event, CommandEvent::Terminated(_)) {
                if let Ok(mut child) = slot.child.lock() {
                    child.take();
                }
                break;
            }
        }
    });
    Ok(pid)
}

fn stop(slot: &ProcessSlot, label: &str) -> Result<(), String> {
    let child = slot
        .child
        .lock()
        .map_err(|_| format!("{label} process state is poisoned"))?
        .take();
    match child {
        Some(child) => child.kill().map_err(|error| error.to_string()),
        None => Err(format!("{label} is not running")),
    }
}
