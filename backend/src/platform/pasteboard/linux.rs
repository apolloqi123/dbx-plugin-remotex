//! Linux file clipboard through the standard clipboard helpers
//! (`xclip` for X11/XWayland, `wl-clipboard` for non-GNOME Wayland).
//!
//! File managers publish copied files as `text/uri-list` (KDE, XFCE, ...) or
//! as GNOME's `x-special/gnome-copied-files`. Reading and writing those two
//! targets needs a process that owns the live selection (X11 selections are
//! request/response), which is exactly what these helpers do; doing it by hand
//! would mean a full X11 event loop. We therefore shell out with fixed argv and
//! feed paths on stdin (never interpolated into a shell).
//!
//! GNOME (X11 or Wayland) does not expose wlr-data-control, so `wl-clipboard`
//! cannot work there; GNOME Wayland is served through the XWayland clipboard
//! bridge with `xclip` instead. Other Wayland desktops use `wl-clipboard`.
//!
//! Polling runs on its own thread and caches the last payload, so the RDP
//! worker's `change_count()` stays cheap and never blocks the engine loop. The
//! thread exits after [`IDLE_AFTER`] without a request and is restarted on the
//! next one.
//!
//! This module is compiled on every host so its pure helpers stay unit-tested
//! on macOS CI; `backend()` only selects it on Linux.

use super::uri_list;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const READ_TIMEOUT: Duration = Duration::from_millis(700);
const WRITE_TIMEOUT: Duration = Duration::from_millis(2_000);
const POLL_INTERVAL: Duration = Duration::from_millis(1_500);
const IDLE_AFTER: Duration = Duration::from_secs(45);
const GNOME_MIME: &str = "x-special/gnome-copied-files";
const URI_LIST_MIME: &str = "text/uri-list";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Xclip,
    WlClipboard,
}

fn has_command(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        if !candidate.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            candidate
                .metadata()
                .map(|meta| meta.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

fn choose_backend(wayland: bool, gnome: bool, wl_clipboard: bool, xclip: bool) -> Option<Backend> {
    // GNOME's compositor does not expose wlr-data-control, so wl-clipboard
    // cannot own or read the clipboard there; only the XWayland bridge (xclip)
    // works. Claiming support through wl-clipboard on GNOME would make the UI
    // promise Ctrl+C/V that silently does nothing.
    if wayland && !gnome && wl_clipboard {
        return Some(Backend::WlClipboard);
    }
    if xclip {
        return Some(Backend::Xclip);
    }
    None
}

fn backend() -> Option<Backend> {
    static DETECTED: OnceLock<Option<Backend>> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        if !cfg!(target_os = "linux") {
            return None;
        }
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let gnome = is_gnome_desktop(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref());
        let wl_clipboard = has_command("wl-paste") && has_command("wl-copy");
        let xclip = std::env::var_os("DISPLAY").is_some() && has_command("xclip");
        choose_backend(wayland, gnome, wl_clipboard, xclip)
    })
}

pub(crate) fn supported() -> bool {
    backend().is_some()
}

struct State {
    counter: i64,
    hash: u64,
    paths: Vec<PathBuf>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            counter: 0,
            hash: 0,
            paths: Vec::new(),
        })
    })
}

static LAST_REQUEST_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn run_capture(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    wait_with_timeout(&mut child, timeout)?;
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn read_payload() -> String {
    let Some(backend) = backend() else {
        return String::new();
    };
    for target in [GNOME_MIME, URI_LIST_MIME] {
        let read = match backend {
            Backend::Xclip => run_capture(
                "xclip",
                &["-selection", "clipboard", "-t", target, "-o"],
                READ_TIMEOUT,
            ),
            Backend::WlClipboard => run_capture("wl-paste", &["--type", target], READ_TIMEOUT),
        };
        if let Some(output) = read {
            if !output.trim().is_empty() {
                return output;
            }
        }
    }
    String::new()
}

fn poll_loop() {
    loop {
        let idle = now_ms().saturating_sub(LAST_REQUEST_MS.load(Ordering::Relaxed));
        if idle > IDLE_AFTER.as_millis() as u64 {
            return;
        }
        let text = read_payload();
        let hash = fnv1a(text.as_bytes());
        let paths = uri_list::paths_from_uri_text(&text);
        if let Ok(mut state) = state().lock() {
            if state.hash != hash {
                state.hash = hash;
                state.paths = paths;
                state.counter = state.counter.wrapping_add(1);
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn ensure_poller() {
    static POLLER: OnceLock<Mutex<Option<std::thread::JoinHandle<()>>>> = OnceLock::new();
    let slot = POLLER.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = slot.lock() else {
        return;
    };
    if guard.as_ref().is_some_and(|handle| !handle.is_finished()) {
        return;
    }
    *guard = std::thread::Builder::new()
        .name("dbx-clipboard-poll".to_string())
        .spawn(poll_loop)
        .ok();
}

/// Cheap change counter; the actual clipboard read happens on the poll thread.
pub(crate) fn change_count() -> i64 {
    if cfg!(test) {
        return 0;
    }
    LAST_REQUEST_MS.store(now_ms(), Ordering::Relaxed);
    ensure_poller();
    state().lock().map(|state| state.counter).unwrap_or(0)
}

pub(crate) fn has_file_urls() -> bool {
    if cfg!(test) {
        return false;
    }
    state()
        .lock()
        .map(|state| !state.paths.is_empty())
        .unwrap_or(false)
}

pub(crate) fn file_urls() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    state()
        .lock()
        .map(|state| state.paths.clone())
        .unwrap_or_default()
}

fn is_gnome_desktop(value: Option<&str>) -> bool {
    let Some(value) = value else {
        return false;
    };
    let value = value.to_ascii_lowercase();
    ["gnome", "unity", "budgie", "cinnamon", "pantheon"]
        .iter()
        .any(|name| value.contains(name))
}

fn write_payload(payload: &str, gnome: bool) -> Result<(), String> {
    let Some(backend) = backend() else {
        return Err("No X11/Wayland clipboard helper is installed".to_string());
    };
    let target = if gnome { GNOME_MIME } else { URI_LIST_MIME };
    let mut child = match backend {
        Backend::Xclip => Command::new("xclip")
            .args(["-selection", "clipboard", "-t", target, "-i"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
        Backend::WlClipboard => Command::new("wl-copy")
            .args(["--type", target])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn(),
    }
    .map_err(|error| format!("Unable to start the clipboard helper: {error}"))?;

    let writer = child
        .stdin
        .take()
        .map(|stdin| spawn_writer(stdin, payload.to_string()));
    let status = wait_with_timeout(&mut child, WRITE_TIMEOUT);
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    let Some(status) = status else {
        return Err("The clipboard helper did not accept the file list".to_string());
    };
    if !status.success() {
        return Err("The clipboard helper rejected the file list".to_string());
    }
    Ok(())
}

fn spawn_writer(mut stdin: ChildStdin, payload: String) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = stdin.write_all(payload.as_bytes());
    })
}

pub(crate) fn write_file_urls(paths: &[PathBuf]) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let absolute: Vec<PathBuf> = paths
        .iter()
        .filter(|path| path.is_absolute())
        .cloned()
        .collect();
    if absolute.is_empty() {
        return Err("No absolute paths to place on the clipboard".to_string());
    }
    let gnome = is_gnome_desktop(std::env::var("XDG_CURRENT_DESKTOP").ok().as_deref());
    let payload = if gnome {
        uri_list::gnome_copied_files_from_paths(&absolute)
    } else {
        uri_list::uri_list_from_paths(&absolute)
    };
    write_payload(&payload, gnome)?;
    // Reflect our own write immediately: pasteboard_references_dir() must keep
    // the downloaded temp files alive before the poll thread sees the change.
    if let Ok(mut state) = state().lock() {
        state.hash = fnv1a(payload.as_bytes());
        state.paths = absolute;
        state.counter = state.counter.wrapping_add(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_desktops_are_detected_case_insensitively() {
        assert!(is_gnome_desktop(Some("GNOME")));
        assert!(is_gnome_desktop(Some("ubuntu:GNOME")));
        assert!(is_gnome_desktop(Some("X-Cinnamon")));
        assert!(!is_gnome_desktop(Some("KDE")));
        assert!(!is_gnome_desktop(None));
    }

    #[test]
    fn hash_changes_with_content() {
        assert_eq!(fnv1a(b"same"), fnv1a(b"same"));
        assert_ne!(fnv1a(b"one"), fnv1a(b"two"));
    }

    #[test]
    fn backend_choice_avoids_wl_clipboard_on_gnome() {
        // GNOME Wayland: wl-clipboard cannot work; only xclip via XWayland.
        assert_eq!(choose_backend(true, true, true, false), None);
        assert_eq!(choose_backend(true, true, true, true), Some(Backend::Xclip));
        // Other Wayland desktops prefer wl-clipboard.
        assert_eq!(
            choose_backend(true, false, true, false),
            Some(Backend::WlClipboard)
        );
        assert_eq!(
            choose_backend(true, false, true, true),
            Some(Backend::WlClipboard)
        );
        // X11 (GNOME or not) uses xclip.
        assert_eq!(
            choose_backend(false, true, false, true),
            Some(Backend::Xclip)
        );
        assert_eq!(
            choose_backend(false, false, false, true),
            Some(Backend::Xclip)
        );
        assert_eq!(choose_backend(false, false, true, false), None);
    }
}
