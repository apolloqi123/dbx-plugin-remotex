//! Reads the remote X11 clipboard's file list through the transfer session.
//!
//! RFB has no file clipboard: copying a file inside a VNC desktop only updates
//! the server's X selection. With an SSH companion session we can ask the
//! remote `xclip`/`xsel` for the URI-list target and download those files,
//! which makes "copy in VNC → paste in Finder" behave like a real file copy.
//! The command is fixed and read-only; no caller input is interpolated.

use serde_json::{json, Value};
use std::time::Duration;

use crate::core::clipboard_uri::parse_uri_list;

const CLIPTOOL_MARKER: &str = "__DBX_CLIPTOOL__";
const CLIPTOOL_NONE_MARKER: &str = "__DBX_CLIPTOOL_NONE__";

/// Shell probe: find a VNC/X display, then read the URI-list target. A bare
/// `xclip -o` waits forever when nothing owns the selection, so every read is
/// bounded by `timeout`; hosts without coreutils simply report no tool.
pub(crate) fn clipboard_command() -> &'static str {
    r#"tool=""
if command -v xclip >/dev/null 2>&1; then tool=xclip
elif command -v xsel >/dev/null 2>&1; then tool=xsel
fi
if [ -z "$tool" ]; then printf '__DBX_CLIPTOOL_NONE__\n'; exit 0; fi
printf '__DBX_CLIPTOOL__\n'
to=""
if command -v timeout >/dev/null 2>&1; then to="timeout 2"; fi
det=$(ps -eo args= 2>/dev/null | grep -E "[X]tigervnc|[X]vnc|[X]vfb|[x]11vnc" | grep -oE ":[0-9]+" | head -n 1)
for d in "$det" :0 :1 :2; do
  [ -n "$d" ] || continue
  if [ "$tool" = xclip ]; then
    [ -n "$to" ] || continue
    out=$(DISPLAY=$d $to xclip -selection clipboard -o -t x-special/gnome-copied-files 2>/dev/null) || out=""
    if [ -z "$out" ]; then out=$(DISPLAY=$d $to xclip -selection clipboard -o -t text/uri-list 2>/dev/null) || out=""; fi
  else
    out=$(DISPLAY=$d $to xsel --clipboard --output 2>/dev/null) || out=""
  fi
  if [ -n "$out" ]; then printf "%s" "$out" | head -c 65536; exit 0; fi
done
"#
}

pub(crate) async fn read(
    ssh: &crate::sessions::ssh::SshRuntime,
    session_id: &str,
) -> Result<Value, String> {
    let outcome = ssh
        .files_session_exec(session_id, clipboard_command(), Duration::from_secs(10))
        .await?;
    let supported =
        outcome.output.contains(CLIPTOOL_MARKER) && !outcome.output.contains(CLIPTOOL_NONE_MARKER);
    let paths = if supported {
        parse_uri_list(&outcome.output)
    } else {
        Vec::new()
    };
    Ok(json!({ "supported": supported, "paths": paths }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_only_targets_uri_list_reads() {
        let command = clipboard_command();
        assert!(command.contains("text/uri-list"));
        assert!(command.contains("x-special/gnome-copied-files"));
        assert!(command.contains("timeout"));
    }
}
