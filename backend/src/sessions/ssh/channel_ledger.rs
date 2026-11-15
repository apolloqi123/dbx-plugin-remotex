//! Lightweight ledger of every SSH session channel this sidecar opens.
//!
//! Written to `<data_dir>/channel-debug.log` (rotated at 256 KiB) so a field
//! report can answer the one question a `MaxSessions` refusal leaves open:
//! *which* channels are still counted when the server starts refusing. Every
//! open records its label and id, every close at our close helpers removes it,
//! and a transient refusal dumps the live set with each channel's age.
//!
//! The ledger is best-effort: a missing/unwritable data dir simply disables
//! it, and logging never blocks an SSH operation on IO.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

const LIMIT_BYTES: u64 = 256 * 1024;
const LOG_NAME: &str = "channel-debug.log";

struct Ledger {
    file: Option<std::fs::File>,
    /// channel id → (label, opened-at ms)
    live: HashMap<u32, (String, u64)>,
}

static LEDGER: OnceLock<Mutex<Ledger>> = OnceLock::new();

pub(super) fn init(data_dir: &Path) {
    let path = data_dir.join(LOG_NAME);
    if std::fs::metadata(&path)
        .map(|meta| meta.len() > LIMIT_BYTES)
        .unwrap_or(false)
    {
        let _ = std::fs::remove_file(&path);
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    if let Some(file) = file.as_mut() {
        let _ = writeln!(
            file,
            "START version={} t={}",
            env!("CARGO_PKG_VERSION"),
            now_ms()
        );
        let _ = file.flush();
    }
    let _ = LEDGER.set(Mutex::new(Ledger {
        file,
        live: HashMap::new(),
    }));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn update(change: impl FnOnce(&mut Ledger, u64) -> String) {
    let Some(ledger) = LEDGER.get() else {
        return;
    };
    let mut ledger = ledger.lock().unwrap_or_else(|poison| poison.into_inner());
    let now = now_ms();
    let line = change(&mut ledger, now);
    if let Some(file) = ledger.file.as_mut() {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// Records a successfully opened session channel.
pub(super) fn opened(label: &str, id: u32) {
    let entry_label = label.to_string();
    update(move |ledger, now| {
        ledger.live.insert(id, (entry_label.clone(), now));
        format!(
            "OPEN id={id} label={entry_label} t={now} live={}",
            ledger.live.len()
        )
    });
}

/// Removes a channel from the inventory (idempotent).
pub(super) fn closed(id: u32) {
    update(|ledger, now| {
        ledger.live.remove(&id);
        format!("CLOSE id={id} t={now} live={}", ledger.live.len())
    });
}

/// Logs a transient refusal together with the live inventory and each
/// channel's age — the evidence needed to tell a client-side leak from a
/// server that still counts channels we already closed.
pub(super) fn refused(label: &str, error: &str) {
    update(|ledger, now| {
        let mut live: Vec<String> = ledger
            .live
            .iter()
            .map(|(id, (label, opened_at))| {
                format!("{id}:{label}:{}s", now.saturating_sub(*opened_at) / 1000)
            })
            .collect();
        live.sort();
        format!(
            "REFUSED label={label} error={error} live={} [{}]",
            live.len(),
            live.join(" ")
        )
    });
}
