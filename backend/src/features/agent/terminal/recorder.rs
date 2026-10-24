//! Bounded terminal output recorder.

use super::*;

/// Capture states of a terminal recorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderState {
    Idle,
    Capturing,
    Settled,
}

/// Silence after the last output chunk before a seen prompt counts as
/// "command finished".
pub const SILENCE_DEBOUNCE: Duration = Duration::from_millis(300);

/// Fallback silence for shells without local echo: after this much quiet
/// following a prompt, the capture settles even without a seen echo.
pub const ECHO_FALLBACK_SILENCE: Duration = Duration::from_secs(2);

/// Cap of the raw capture buffer (newest output wins).
const RECORDER_BUFFER_CAP: usize = 1024 * 1024;
/// Cap of the rolling tail used for prompt detection, kept small so the
/// scan never walks the full buffer on every chunk.
const RECORDER_TAIL_CAP: usize = 8 * 1024;

/// Bounded output recorder installed on a session while an agent command
/// runs in the user's terminal. The PTY read loop feeds every stdout chunk
/// into [`TerminalRecorder::observe`]; the capture ends when a fresh shell
/// prompt has been seen and the terminal went silent for
/// [`SILENCE_DEBOUNCE`], or when the caller forces
/// [`TerminalRecorder::finish`] (timeout / denial paths).
///
/// Known limits (accepted by design): an ANSI sequence split across chunk
/// boundaries can leave residue in the buffer, multi-line commands execute
/// line by line, and full-screen TUIs never show a prompt (they run into
/// the timeout path and return partial output).
pub struct TerminalRecorder {
    state: RecorderState,
    buffer: String,
    tail: String,
    prompt_seen: bool,
    echo_seen: bool,
    last_chunk_at: Option<Instant>,
    silence_debounce: Duration,
    /// Fragment of the injected command whose echo proves the shell actually
    /// started executing it (guards against settling on the login banner
    /// prompt before the typed command was processed).
    echo_fragment: String,
}

impl Default for TerminalRecorder {
    fn default() -> Self {
        Self {
            state: RecorderState::Idle,
            buffer: String::new(),
            tail: String::new(),
            prompt_seen: false,
            echo_seen: false,
            last_chunk_at: None,
            silence_debounce: SILENCE_DEBOUNCE,
            echo_fragment: String::new(),
        }
    }
}

impl TerminalRecorder {
    /// Starts a fresh capture, discarding any previous content. The
    /// `echo_fragment` (a slice of the injected command) gates settling: a
    /// prompt-silence without the command's echo means the shell has not
    /// processed the injection yet.
    pub fn arm(&mut self, echo_fragment: &str) {
        self.state = RecorderState::Capturing;
        self.buffer.clear();
        self.tail.clear();
        self.prompt_seen = false;
        self.echo_seen = false;
        self.last_chunk_at = None;
        self.echo_fragment = echo_fragment.chars().take(32).collect();
    }

    /// Feeds one chunk of PTY output. Returns the recorder state; a chunk
    /// arriving after the capture already settled reports `Settled` before
    /// absorbing the straggler.
    pub fn observe(&mut self, chunk: &str) -> RecorderState {
        if self.state != RecorderState::Capturing {
            return self.state;
        }
        if self.is_settled() {
            self.state = RecorderState::Settled;
            return self.state;
        }
        push_bounded(&mut self.buffer, chunk, RECORDER_BUFFER_CAP);
        push_bounded(&mut self.tail, chunk, RECORDER_TAIL_CAP);
        // Prompt detection reuses exec.rs's shell-prompt heuristic on the
        // ANSI-stripped tail, so colored prompts still match.
        if exec::has_shell_prompt(&strip_ansi(&self.tail)) {
            self.prompt_seen = true;
        }
        if !self.echo_seen
            && !self.echo_fragment.is_empty()
            && self.tail.contains(&self.echo_fragment)
        {
            self.echo_seen = true;
        }
        self.last_chunk_at = Some(Instant::now());
        self.state
    }

    /// True when the command's echo was seen, a prompt followed and the
    /// terminal went silent for [`SILENCE_DEBOUNCE`]. As a fallback for
    /// shells without local echo, a long [`ECHO_FALLBACK_SILENCE`] silence
    /// after a prompt settles too; [`TerminalRecorder::finish`] forces the
    /// capture closed regardless.
    pub fn is_settled(&self) -> bool {
        match self.state {
            RecorderState::Settled => true,
            RecorderState::Capturing => {
                let silence = self
                    .last_chunk_at
                    .is_some_and(|at| at.elapsed() >= self.silence_debounce);
                let long_silence = self
                    .last_chunk_at
                    .is_some_and(|at| at.elapsed() >= ECHO_FALLBACK_SILENCE);
                self.prompt_seen && ((self.echo_seen && silence) || long_silence)
            }
            RecorderState::Idle => false,
        }
    }

    /// Forces the capture closed (timeout / denial path).
    pub fn finish(&mut self) {
        self.state = RecorderState::Settled;
    }

    /// Returns the captured output: ANSI-stripped text with, best-effort,
    /// the echoed command line and the trailing prompt line removed.
    pub fn take_output(&mut self, command: &str) -> String {
        self.state = RecorderState::Settled;
        let text = strip_ansi(&self.buffer);
        let mut lines: Vec<&str> = text.lines().collect();
        // The echoed command line contains a fragment of the command itself
        // (usually behind the prompt); drop exactly one such leading line.
        let fragment: String = command
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(24)
            .collect();
        if !fragment.is_empty() && lines.first().is_some_and(|line| line.contains(&fragment)) {
            lines.remove(0);
        }
        trim_trailing_blank_lines(&mut lines);
        if looks_like_prompt_line(lines.last().copied()) {
            lines.pop();
            trim_trailing_blank_lines(&mut lines);
        }
        lines.join("\n")
    }
}

/// Single-line prompt check mirroring the tail of `exec.rs::has_shell_prompt`
/// (line trimmed of trailing spaces ending in `$` or `#`); the exec helper
/// operates on whole texts, so the one-line shape is duplicated here.
fn looks_like_prompt_line(line: Option<&str>) -> bool {
    line.is_some_and(|line| {
        let line = line.trim_end();
        line.ends_with('$') || line.ends_with('#')
    })
}

fn trim_trailing_blank_lines(lines: &mut Vec<&str>) {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

/// Appends `chunk` keeping at most the newest `cap` bytes, cutting at a
/// char boundary so the buffer stays valid UTF-8.
fn push_bounded(buffer: &mut String, chunk: &str, cap: usize) {
    buffer.push_str(chunk);
    if buffer.len() > cap {
        let mut cut = buffer.len() - cap;
        while !buffer.is_char_boundary(cut) {
            cut += 1;
        }
        buffer.drain(..cut);
    }
}
