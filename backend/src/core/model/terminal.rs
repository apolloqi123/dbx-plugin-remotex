//! Model terminal types.

use super::*;

#[derive(Debug, Clone)]
pub struct TerminalFrame {
    pub sequence: u64,
    pub stream: TerminalStream,
    pub data: Vec<u8>,
}

impl TerminalFrame {
    pub fn encode(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(9 + self.data.len());
        encoded.push(self.stream as u8);
        encoded.extend_from_slice(&self.sequence.to_be_bytes());
        encoded.extend_from_slice(&self.data);
        encoded
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TerminalStream {
    Stdout = 0,
    Stderr = 1,
    State = 2,
    /// Serial B1 inbound write frames (`serial/terminal/in/{id}`) only; the
    /// value skips `State = 2`, which the local terminal already uses for
    /// in-band state frames. Decoders that meet an unknown tag must drop the
    /// frame silently and count it (docs/architecture/SERIAL_ENHANCE_DESIGN.zh-CN.md §2).
    Stdin = 3,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpenRequest {
    pub connection_id: String,
    #[serde(default)]
    pub workbench_id: String,
    /// Open another PTY channel over a live authenticated SSH transport.
    /// False keeps the long-standing "new session" reconnect semantics.
    #[serde(default)]
    pub reuse_authenticated_transport: bool,
    /// Session whose authenticated transport should be reused. Newer
    /// workbenches send this explicitly so "duplicate" follows the window
    /// the user clicked instead of an arbitrary same-connection session.
    #[serde(default)]
    pub reuse_authenticated_session_id: Option<String>,
    /// Optional caller-provided id lets the frontend subscribe to terminal output
    /// before the long-running open RPC returns. Older callers keep server UUIDs.
    #[serde(default)]
    pub requested_session_id: Option<String>,
    /// WT-4 (WezTerm `spawn` parity): run this command on the new channel
    /// (independent PTY) instead of a shell. One-shot open parameter — the
    /// contract lives in docs/architecture/PROTOCOL.zh-CN.md「同 transport 命令会话」;
    /// absent/blank keeps the ordinary shell or connection `remote_command`.
    #[serde(default)]
    pub spawn_command: Option<String>,
    #[serde(default = "default_cols")]
    pub cols: u32,
    #[serde(default = "default_rows")]
    pub rows: u32,
}

pub(super) fn default_cols() -> u32 {
    120
}

pub(super) fn default_rows() -> u32 {
    32
}
