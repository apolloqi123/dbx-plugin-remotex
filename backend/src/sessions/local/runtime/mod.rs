//! Local terminal runtime support.
//!
//! The runtime is split by responsibility: `lifecycle` owns spawn, the pump
//! and exit teardown; `io` owns output frames, replay and keyboard input;
//! `control` owns the resize/close RPCs. The public surface is unchanged —
//! `local/mod.rs` glob-imports this module.

use super::*;

mod control;
mod io;
mod lifecycle;

/// Recorder feed used by the module's tests (it lives in `io`, next to the
/// publisher that produces the bursts it observes). Test builds only —
/// production code reaches it straight from `lifecycle`'s pump.
#[cfg(test)]
pub(crate) use io::observe_local_recording;

impl LocalTerminalRuntime {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub(super) async fn session(&self, session_id: &str) -> Result<Arc<LocalSession>, String> {
        self.sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| "Local session was not found".to_string())
    }

    /// Read-only inventory of live local shells (a session only exists while
    /// its child runs), used by the workbench to reattach after a reload.
    pub async fn list(&self) -> Value {
        let sessions = self.sessions.read().await;
        let mut list: Vec<Value> = sessions
            .iter()
            .map(|(session_id, session)| {
                json!({
                    "sessionId": session_id,
                    "workbenchId": session.workbench_id,
                    "shell": session.shell,
                    "createdAt": session.created_at_secs,
                })
            })
            .collect();
        list.sort_by(|a, b| {
            a["createdAt"]
                .as_u64()
                .cmp(&b["createdAt"].as_u64())
                .then_with(|| a["sessionId"].as_str().cmp(&b["sessionId"].as_str()))
        });
        json!({ "sessions": list })
    }
}

impl LocalTerminalRuntime {
    /// PR-A4 generic launch-options contract (host dock "+"): the default
    /// "local terminal" entry first — carried WITHOUT a `group` so the host
    /// pins it top-level as the quickest action — followed by one entry per
    /// discovered shell sharing the localized `group` label, which the host
    /// renders as one collapsible section right below. The default entry
    /// opens with the configured `localShell` preference (auto-detect chain
    /// as fallback) — resolved by `resolve_default_shell` on the host call
    /// path — while the per-shell entries pin an explicit shell via their
    /// context fragment. `configured_shell` (the raw `localShell` preference,
    /// via `resolve_configured_shell`) collapses the discovery group: once
    /// the user picked a default there is nothing to choose among, so the
    /// picker carries just the one configured entry. Business meaning stays
    /// on the plugin side; the host renders labels and never interprets the
    /// contexts. `locale` is the host UI language (the host-side
    /// `options_action` contract); older hosts omit it and get English.
    pub fn launch_options(
        &self,
        locale: &str,
        default_shell: &str,
        configured_shell: Option<&str>,
    ) -> Value {
        let (label, group, description_prefix) = launch_option_copy(locale);
        let mut entries = vec![json!({
            "label": label,
            "description": format!("{description_prefix}{default_shell}"),
            "context": { "plugin": { "mode": "local-terminal" } },
        })];
        if configured_shell.is_some() {
            return json!({ "entries": entries });
        }
        for shell in discover_shells(current_platform()) {
            if shell.program.is_empty() {
                continue;
            }
            entries.push(json!({
                "label": shell.name,
                "description": shell.program,
                "context": { "plugin": { "mode": "local-terminal", "shell": shell.program } },
                "group": group,
            }));
        }
        json!({ "entries": entries })
    }

    /// Read-only shell inventory for the workbench's shell picker.
    pub fn shells(&self) -> Value {
        let platform = current_platform();
        json!({
            "platform": match platform {
                Platform::MacOS => "macos",
                Platform::Linux => "linux",
                Platform::Windows => "windows",
            },
            "shells": discover_shells(platform)
                .iter()
                .map(|entry| json!({
                    "program": entry.program,
                    "name": entry.name,
                    "isDefault": entry.is_default,
                    "isUserShell": entry.is_user_shell,
                    "injectable": entry.injectable,
                }))
                .collect::<Vec<_>>(),
        })
    }
}
