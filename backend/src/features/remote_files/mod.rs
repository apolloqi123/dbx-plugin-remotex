//! File transfer between the host machine and a remote desktop host.
//!
//! RFB (VNC) has no file clipboard and CLIPRDR (RDP) only carries files the
//! remote user puts on their clipboard. The remote-desktop toolbar therefore
//! exposes an explicit two-pane transfer window backed by a dedicated,
//! PTY-free SSH/SFTP connection. It reuses the existing SFTP upload/download
//! pipelines (1 MiB chunks, progress events, conflict handling) while keeping
//! its transport out of `ssh/sessions/list`.

pub(crate) mod clipboard;
pub(crate) mod copy;
pub(crate) mod credentials;
pub(crate) mod local;
pub(crate) mod remote_clipboard;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::sessions::ssh::SshRuntime;

struct CopyTask {
    session_id: String,
    cancel: Arc<AtomicBool>,
}

struct TrackedSession {
    workbench_id: String,
    last_seen: Instant,
}

pub struct RemoteFilesRuntime {
    pub(crate) ssh: Arc<SshRuntime>,
    tasks: Mutex<HashMap<String, CopyTask>>,
    sessions: Mutex<HashMap<String, TrackedSession>>,
}

impl RemoteFilesRuntime {
    pub fn new(ssh: Arc<SshRuntime>) -> Self {
        Self {
            ssh,
            tasks: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn register_task(&self, task_id: &str, session_id: &str) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        self.tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                task_id.to_string(),
                CopyTask {
                    session_id: session_id.to_string(),
                    cancel: cancel.clone(),
                },
            );
        cancel
    }

    pub(crate) fn finish_task(&self, task_id: &str) {
        self.tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(task_id);
    }

    /// Cancels a task only when it belongs to the caller's session. A stale
    /// task id from another workbench must not abort an unrelated transfer.
    pub(crate) fn cancel_task(&self, task_id: &str, session_id: &str) -> bool {
        let tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match tasks.get(task_id) {
            Some(task) if task.session_id == session_id => {
                task.cancel.store(true, Ordering::Release);
                true
            }
            _ => false,
        }
    }

    /// Drops every in-flight transfer when the panel's session goes away.
    pub(crate) fn cancel_session(&self, session_id: &str) {
        let tasks = self
            .tasks
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        for task in tasks.values() {
            if task.session_id == session_id {
                task.cancel.store(true, Ordering::Release);
            }
        }
    }

    pub(crate) fn track_session(&self, workbench_id: &str, session_id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                session_id.to_string(),
                TrackedSession {
                    workbench_id: workbench_id.to_string(),
                    last_seen: Instant::now(),
                },
            );
    }

    /// Removes and returns the files session previously opened by the same
    /// workbench (a retried/abandoned connect must not leak a second one).
    pub(crate) fn take_session_for_workbench(&self, workbench_id: &str) -> Option<String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let session_id = sessions
            .iter()
            .find(|(_, tracked)| tracked.workbench_id == workbench_id)
            .map(|(session_id, _)| session_id.clone())?;
        sessions.remove(&session_id);
        Some(session_id)
    }

    pub(crate) fn untrack_session(&self, session_id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(session_id);
    }

    /// Panel heartbeat: returns false when the session is no longer tracked
    /// (disconnected or reaped), which the UI treats as a lost connection.
    pub(crate) fn touch_session(&self, session_id: &str) -> bool {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match sessions.get_mut(session_id) {
            Some(tracked) => {
                tracked.last_seen = Instant::now();
                true
            }
            None => false,
        }
    }

    /// Drops sessions whose panel stopped heartbeating (closed webview,
    /// abandoned connect) and returns their ids for the caller to close.
    pub(crate) fn take_stale_sessions(&self, max_age: Duration) -> Vec<String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = Instant::now();
        let stale = sessions
            .iter()
            .filter(|(_, tracked)| now.duration_since(tracked.last_seen) >= max_age)
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();
        for session_id in &stale {
            sessions.remove(session_id);
        }
        stale
    }
}
