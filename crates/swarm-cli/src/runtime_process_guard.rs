use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};
use swarm_core::DataPaths;
use swarm_protocol::WorldId;

const RUNTIME_PROCESS_RECORD_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeProcessRecord {
    version: u16,
    controller_pid: u32,
    java_pid: Option<u32>,
}

/// Persistent fail-closed ownership record for the Java process associated
/// with one authority runtime.
///
/// The advisory `authority-runtime.lock` is owned by the Rust controller and
/// disappears when that controller is hard-killed. This record deliberately
/// survives such a death. A later controller must prove the previous Java PID
/// is gone before it is allowed to reset `runtime/<world>`.
pub struct RuntimeProcessGuard {
    path: PathBuf,
    record: RuntimeProcessRecord,
}

impl RuntimeProcessGuard {
    pub fn begin(paths: &DataPaths, world: WorldId) -> Result<Self> {
        let path = record_path(paths, world);
        inspect_previous_record(&path)?;
        let record = RuntimeProcessRecord {
            version: RUNTIME_PROCESS_RECORD_VERSION,
            controller_pid: std::process::id(),
            java_pid: None,
        };
        atomic_write(&path, &record)?;
        Ok(Self { path, record })
    }

    /// Persist the child PID immediately after spawn. If this write fails the
    /// caller must terminate/reap the child before returning the error.
    pub fn record_java(&mut self, pid: u32) -> Result<()> {
        self.record.java_pid = Some(pid);
        atomic_write(&self.path, &self.record)
            .with_context(|| format!("cannot persist Java runtime ownership for PID {pid}"))
    }
}

impl Drop for RuntimeProcessGuard {
    fn drop(&mut self) {
        let safe_to_remove = match self.record.java_pid {
            None => true,
            Some(pid) => matches!(process_alive(pid), Ok(false)),
        };
        if safe_to_remove {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn inspect_previous_record(path: &PathBuf) -> Result<()> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("cannot read runtime process record {}", path.display())),
    };
    let record: RuntimeProcessRecord = serde_json::from_slice(&bytes)
        .with_context(|| format!("runtime process record is malformed at {}; refusing unsafe runtime reset", path.display()))?;
    if record.version != RUNTIME_PROCESS_RECORD_VERSION {
        bail!("runtime process record version is unsupported; refusing unsafe runtime reset");
    }
    match record.java_pid {
        Some(pid) => {
            if process_alive(pid).with_context(|| format!("cannot prove whether previous Java PID {pid} is still alive"))? {
                bail!(
                    "previous Minecraft Java process PID {pid} is still alive; refusing to reset its runtime directory"
                );
            }
            fs::remove_file(path)
                .with_context(|| format!("cannot clear stale runtime process record {}", path.display()))?;
            Ok(())
        }
        None => {
            // A hard controller death between creating the pending record and
            // durably recording the spawned Java PID is intentionally
            // ambiguous. Deleting the runtime in that state could race a Java
            // process whose PID was never committed, so recovery is fail-closed.
            bail!(
                "a previous authority launch ended before Java ownership was durably recorded; refusing to reset the runtime directory until the ambiguous launch is investigated"
            )
        }
    }
}

fn record_path(paths: &DataPaths, world: WorldId) -> PathBuf {
    paths.root.join("control").join(world.to_hex()).join("runtime-process.json")
}

fn atomic_write(path: &PathBuf, record: &RuntimeProcessRecord) -> Result<()> {
    let parent = path.parent().context("runtime process record has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("cannot create runtime process record directory {}", parent.display()))?;
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record)?;
    fs::write(&temporary, bytes)
        .with_context(|| format!("cannot stage runtime process record {}", temporary.display()))?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("cannot replace runtime process record {}", path.display()))?;
    }
    fs::rename(&temporary, path)
        .with_context(|| format!("cannot publish runtime process record {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn process_alive(pid: u32) -> Result<bool> {
    const ESRCH: i32 = 3;
    const EPERM: i32 = 1;
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    if pid > i32::MAX as u32 {
        return Ok(false);
    }
    let result = unsafe { kill(pid as i32, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(ESRCH) => Ok(false),
        Some(EPERM) => Ok(true),
        _ => Err(error.into()),
    }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> Result<bool> {
    type Handle = *mut std::ffi::c_void;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;
    const ERROR_INVALID_PARAMETER: u32 = 87;
    unsafe extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        fn GetExitCodeProcess(process: Handle, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
        fn GetLastError() -> u32;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        let error = unsafe { GetLastError() };
        if error == ERROR_INVALID_PARAMETER {
            return Ok(false);
        }
        // Access denied or another indeterminate result must fail closed.
        return Ok(true);
    }
    let mut exit_code = 0_u32;
    let success = unsafe { GetExitCodeProcess(handle, &mut exit_code) };
    unsafe { CloseHandle(handle) };
    if success == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(exit_code == STILL_ACTIVE)
}

#[cfg(not(any(unix, windows)))]
fn process_alive(_pid: u32) -> Result<bool> {
    bail!("process-liveness proof is not implemented on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> WorldId {
        WorldId([0x66; 32])
    }

    #[test]
    fn current_process_record_blocks_runtime_reset() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        atomic_write(
            &path,
            &RuntimeProcessRecord {
                version: RUNTIME_PROCESS_RECORD_VERSION,
                controller_pid: std::process::id(),
                java_pid: Some(std::process::id()),
            },
        )
        .unwrap();
        let error = RuntimeProcessGuard::begin(&paths, world()).err().expect("live PID must block");
        assert!(error.to_string().contains("still alive"));
    }

    #[test]
    fn ambiguous_pending_record_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        atomic_write(
            &path,
            &RuntimeProcessRecord {
                version: RUNTIME_PROCESS_RECORD_VERSION,
                controller_pid: 1,
                java_pid: None,
            },
        )
        .unwrap();
        let error = RuntimeProcessGuard::begin(&paths, world()).err().expect("ambiguous record must block");
        assert!(error.to_string().contains("before Java ownership was durably recorded"));
    }

    #[test]
    fn clean_unspawned_guard_removes_pending_record() {
        let temp = tempfile::tempdir().unwrap();
        let paths = DataPaths::from_root(temp.path().to_path_buf());
        let path = record_path(&paths, world());
        {
            let _guard = RuntimeProcessGuard::begin(&paths, world()).unwrap();
            assert!(path.is_file());
        }
        assert!(!path.exists());
    }
}
