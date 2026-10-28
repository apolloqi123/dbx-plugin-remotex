//! SSH support.

use super::*;

pub(crate) enum TerminalCommand {
    Input(Vec<u8>),
    Resize {
        cols: u32,
        rows: u32,
    },
    DirectoryTracking {
        enabled: bool,
    },
    /// Session-open arming of the per-prompt hook (issue t8y2/dbx#10750): the
    /// shell type was probed off the read loop and is already resolved, so the
    /// handler never awaits the network. Non-bash/zsh shells are skipped
    /// silently — unlike the user-facing DirectoryTracking toggle there is no
    /// `directory-tracking-unavailable` event for shells that cannot host the
    /// hook (a fish/nushell user must not get a warning toast on every open).
    InstallPromptHook {
        shell: RemoteShell,
    },
    Close,
}

/// Enqueue keyboard input on the bounded per-session queue without parking an
/// SDK worker thread for long. A full queue is transient overload: retry for a
/// short bounded budget, then drop the chunk. Only a closed queue is an error
/// (the handler mirrors it as `ssh/terminal/error`, which the workbench treats
/// as session death); a dropped keystroke must never tear down a healthy
/// session.
pub(crate) fn enqueue_terminal_input(
    sender: &mpsc::Sender<TerminalCommand>,
    data: Vec<u8>,
) -> Result<(), String> {
    crate::sessions::input_queue::enqueue_bounded(sender, TerminalCommand::Input(data), "SSH")
}

/// Deterministic pick among a connection's live sessions: the oldest one (the
/// connection's primary workbench), by monotonic creation sequence.
/// Candidates are `(session_id, created_seq, connected)`; disconnected
/// sessions never win. Purity is the point — the caller feeds it from a
/// HashMap whose iteration order is randomized per process, and the chosen
/// session must not depend on that order (a created_at tie-break could not
/// save it: two workbenches often open within the same second).
pub(crate) fn select_primary_session<I>(candidates: I) -> Option<String>
where
    I: IntoIterator<Item = (String, u64, bool)>,
{
    candidates
        .into_iter()
        .filter(|(_, _, connected)| *connected)
        .min_by(|(a_id, a_created, _), (b_id, b_created, _)| {
            a_created.cmp(b_created).then_with(|| a_id.cmp(b_id))
        })
        .map(|(id, _, _)| id)
}

/// Terminal activity keepalive payload: space + backspace. Net-zero on a
/// shell prompt (an empty line never enters history), movement-only in
/// full-screen apps; protocol-level keepalives don't count as keyboard
/// activity, and idle policies that watch the PTY (TMOUT, bastion audits)
/// only real input resets.
pub(crate) const TERMINAL_KEEPALIVE_INPUT: &[u8] = b" \x7f";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteShell {
    Bash,
    Zsh,
    /// Shell capability has not been probed yet; keep the terminal usable.
    Unknown,
    Other,
}

/// Emitted once the handshake marker is found. Everything before the marker
/// (the echoed install line, including its CRLF) is suppressed, so without a
/// line reset the first prompt after arming would be drawn at the column the
/// pre-arming prompt left the cursor at — two prompts glued on one line.
///
/// The leading CR is load-bearing: the frontend prompt-newline transformer
/// (see `terminalPromptNewline.ts`) treats it as "at line start" and therefore
/// does not insert a second CRLF before the prompt hook's OSC 7 frame.
pub(crate) const HANDSHAKE_LINE_CLEAR: &[u8] = b"\r\x1b[2K";

#[derive(Default)]
pub(crate) struct DirectoryHandshakeFilter {
    pub(crate) marker: Option<Vec<u8>>,
    pub(crate) buffered: Vec<u8>,
    pub(crate) started_at: Option<Instant>,
    pub(crate) failed: bool,
}

impl DirectoryHandshakeFilter {
    pub(crate) fn begin(&mut self, marker: Vec<u8>) {
        self.marker = Some(marker);
        self.buffered.clear();
        self.started_at = Some(Instant::now());
        self.failed = false;
    }

    pub(crate) fn filter(&mut self, data: &[u8]) -> Option<Vec<u8>> {
        let Some(marker) = self.marker.as_ref() else {
            return Some(data.to_vec());
        };
        self.buffered.extend_from_slice(data);
        if self.buffered.len() > DIRECTORY_HANDSHAKE_LIMIT {
            return self.fail_open();
        }
        let index = find_bytes(&self.buffered, marker)?;
        let hidden = index > 0;
        let result = self.buffered[index + marker.len()..].to_vec();
        self.marker = None;
        self.buffered.clear();
        self.started_at = None;
        if !hidden && result.is_empty() {
            return None;
        }
        let mut cleared = Vec::with_capacity(HANDSHAKE_LINE_CLEAR.len() + result.len());
        cleared.extend_from_slice(HANDSHAKE_LINE_CLEAR);
        cleared.extend_from_slice(&result);
        Some(cleared)
    }

    pub(crate) fn flush_if_timed_out(&mut self) -> Option<Vec<u8>> {
        if self
            .started_at
            .is_some_and(|started| started.elapsed() >= DIRECTORY_HANDSHAKE_TIMEOUT)
        {
            return self.fail_open();
        }
        None
    }

    pub(crate) fn take_failed(&mut self) -> bool {
        std::mem::take(&mut self.failed)
    }

    pub(crate) fn fail_open(&mut self) -> Option<Vec<u8>> {
        self.marker = None;
        self.started_at = None;
        self.failed = true;
        let buffered = std::mem::take(&mut self.buffered);
        (!buffered.is_empty()).then_some(buffered)
    }
}

/// Monotonic terminal output journal shared by the SSH and local-terminal
/// session loops: assigns each chunk a sequence, keeps a bounded tail for
/// `replay`, and survives transport gaps. The byte budget defaults to the
/// shared 2 MiB terminal limit; serial sessions build a smaller buffer via
/// [`ReplayBuffer::with_byte_limit`] (128 KiB, design doc §3).
pub(crate) struct ReplayBuffer {
    pub(crate) frames: VecDeque<TerminalFrame>,
    pub(crate) bytes: usize,
    pub(crate) sequence: u64,
    pub(crate) byte_limit: usize,
}

impl Default for ReplayBuffer {
    fn default() -> Self {
        Self::with_byte_limit(TERMINAL_REPLAY_LIMIT)
    }
}

impl ReplayBuffer {
    pub(crate) fn with_byte_limit(byte_limit: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            bytes: 0,
            sequence: 0,
            byte_limit,
        }
    }

    pub(crate) fn push(&mut self, stream: TerminalStream, data: Vec<u8>) -> TerminalFrame {
        self.sequence += 1;
        let frame = TerminalFrame {
            sequence: self.sequence,
            stream,
            data,
        };
        self.bytes += frame.data.len();
        self.frames.push_back(frame.clone());
        while self.bytes > self.byte_limit {
            let Some(removed) = self.frames.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.data.len());
        }
        frame
    }

    pub(crate) fn after(&self, sequence: u64) -> Vec<TerminalFrame> {
        self.frames
            .iter()
            .filter(|frame| frame.sequence > sequence)
            .cloned()
            .collect()
    }

    pub(crate) fn first_sequence(&self) -> u64 {
        self.frames
            .front()
            .map(|frame| frame.sequence)
            .unwrap_or(self.sequence.saturating_add(1))
    }

    /// Highest assigned sequence (0 until the first push).
    pub(crate) fn tail_sequence(&self) -> u64 {
        self.sequence
    }

    /// Tail of recent terminal output (State frames excluded) up to
    /// `max_bytes`, newest bytes kept. Used to seed a recording with the
    /// screen already on display when recording starts on a live session.
    pub(crate) fn output_tail(&self, max_bytes: usize) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        for frame in self.frames.iter().rev() {
            if frame.stream == TerminalStream::State || frame.data.is_empty() {
                continue;
            }
            let remaining = max_bytes.saturating_sub(out.len());
            if remaining == 0 {
                break;
            }
            let take = remaining.min(frame.data.len());
            let mut chunk = frame.data[frame.data.len() - take..].to_vec();
            chunk.extend_from_slice(&out);
            out = chunk;
            if out.len() >= max_bytes {
                break;
            }
        }
        out
    }
}
