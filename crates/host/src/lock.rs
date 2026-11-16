//! One host per settings directory.
//!
//! Two hosts over the same folder would share its displays and its active
//! theme, so the second refuses to start while the first is alive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::config::write_atomic;
use crate::config::write_json;

pub const LOCK_FILE: &str = "host.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Locked {
    pid: u32,
    port: u16,
}

/// The folder belongs to a host that is still running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldError {
    pub pid: u32,
    pub port: u16,
    pub settings_dir: PathBuf,
}

impl HeldError {
    pub fn message(&self) -> String {
        format!(
            "{} is held by a host on port {} (pid {}), so this one would share its displays and its active theme. \
             Stop that host, or run this one with a different app dir.",
            self.settings_dir.display(),
            self.port,
            self.pid
        )
    }
}

/// Proof that this process owns the folder; removing it on drop releases the
/// folder.
#[derive(Debug)]
pub struct HostLock {
    path: PathBuf,
    pid: u32,
}

impl HostLock {
    pub fn acquire(settings_dir: &Path, port: u16) -> Result<Self, HeldError> {
        let path = settings_dir.join(LOCK_FILE);
        if let Some(held) = read_lock(&path)
            && held.pid != std::process::id()
            && pid_is_alive(held.pid)
        {
            return Err(HeldError {
                pid: held.pid,
                port: held.port,
                settings_dir: settings_dir.to_path_buf(),
            });
        }
        let pid = std::process::id();
        // A folder that cannot be written is a folder whose lock cannot be
        // read either; the host still runs, unguarded.
        write_json(&path, &Locked { pid, port }).ok();
        Ok(Self { path, pid })
    }

    pub fn release(&self) {
        if read_lock(&self.path).is_some_and(|held| held.pid == self.pid) {
            std::fs::remove_file(&self.path).ok();
        }
    }
}

impl Drop for HostLock {
    fn drop(&mut self) {
        self.release();
    }
}

fn read_lock(path: &Path) -> Option<Locked> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether a process with this id exists. Only a positive answer needs
/// evidence; an unreadable process table counts as alive so a host is never
/// started over one that might be running.
fn pid_is_alive(pid: u32) -> bool {
    let Ok(mut system) = sysinfo::System::new() else {
        return true;
    };
    let wanted = [sysinfo::Pid::from_u32(pid)];
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&wanted), true);
    system.process(sysinfo::Pid::from_u32(pid)).is_some()
}

/// Record a new port after the listener found one free, without losing the
/// pid check.
pub fn reacquire(settings_dir: &Path, port: u16) -> HostLock {
    let path = settings_dir.join(LOCK_FILE);
    let pid = std::process::id();
    write_json(&path, &Locked { pid, port }).ok();
    HostLock { path, pid }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_other_pid() -> Option<u32> {
        let Ok(mut system) = sysinfo::System::new() else {
            return None;
        };
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        system
            .processes()
            .keys()
            .map(|pid| pid.as_u32())
            .find(|pid| *pid != std::process::id())
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunari-lock-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn a_live_host_holds_its_folder() {
        let dir = dir("live");
        let Some(other) = live_other_pid() else {
            // A process table with only this test in it cannot show what a
            // second host looks like.
            return;
        };
        std::fs::write(
            dir.join(LOCK_FILE),
            format!(r#"{{"pid":{other},"port":5229}}"#),
        )
        .unwrap();
        let refusal = HostLock::acquire(&dir, 5227).unwrap_err();
        assert_eq!(refusal.pid, other);
        assert_eq!(refusal.port, 5229);
        assert!(refusal.message().contains("held by a host on port 5229"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_dead_pid_is_replaced() {
        let dir = dir("stale");
        std::fs::write(dir.join(LOCK_FILE), r#"{"pid":999999999,"port":5227}"#).unwrap();
        let lock = HostLock::acquire(&dir, 5227).expect("a stale lock is taken over");
        let text = std::fs::read_to_string(dir.join(LOCK_FILE)).unwrap();
        assert!(text.contains(&std::process::id().to_string()));
        drop(lock);
        assert!(!dir.join(LOCK_FILE).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn release_leaves_another_hosts_lock_alone() {
        let dir = dir("other");
        std::fs::write(dir.join(LOCK_FILE), r#"{"pid":999999999,"port":5227}"#).unwrap();
        let lock = HostLock::acquire(&dir, 5227).expect("a stale lock is taken over");
        std::fs::write(dir.join(LOCK_FILE), r#"{"pid":999999999,"port":5228}"#).unwrap();
        lock.release();
        assert!(dir.join(LOCK_FILE).exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_atomic_replaces_the_whole_file() {
        let dir = dir("atomic");
        let path = dir.join("part");
        write_atomic(&path, b"complete").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete");
        std::fs::remove_dir_all(&dir).ok();
    }
}
