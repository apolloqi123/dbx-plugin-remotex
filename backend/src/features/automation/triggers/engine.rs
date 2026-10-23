//! Trigger engine layer.

use super::*;

/// Per-session trigger state machine: rolling normalized buffer (D2), stage
/// cursor, per-stage timer (seconds, `now` injected), prompt reset (D3).
pub struct TriggerEngine {
    config: TriggersConfig,
    placeholders: CommandPlaceholders,
    buffer: String,
    /// 0-based index of the stage currently being waited for.
    pub(super) stage_cursor: usize,
    /// Millisecond timestamp (caller-injected) when the current stage wait
    /// started; `None` until the first chunk arms the timer.
    stage_since_ms: Option<u64>,
}

impl TriggerEngine {
    pub fn new(config: TriggersConfig, placeholders: CommandPlaceholders) -> Self {
        Self {
            config,
            placeholders,
            buffer: String::new(),
            stage_cursor: 0,
            stage_since_ms: None,
        }
    }

    pub fn placeholders(&self) -> &CommandPlaceholders {
        &self.placeholders
    }

    /// Pacing knobs for command answers: their stdout is resolved by the
    /// caller (after `observe` returned), so the caller needs the config's
    /// `sleepMs`/`passSleep` to build the final segment plan via
    /// [`pass_sleep_segments`]. Returned as `(sleep_ms, pass_sleep)`.
    pub fn pacing(&self) -> (u64, PassSleep) {
        (self.config.sleep_ms, self.config.pass_sleep)
    }

    /// Feeds one terminal output chunk (`now_ms` = Unix milliseconds). Returns
    /// the decision to act on: a stage/case answer (with its segment plan), a
    /// command to run locally, or a timeout reset. Shell prompts reset the
    /// stage cursor to stage 1 (D3) and produce no decision.
    pub fn observe(&mut self, chunk: &str, now_ms: u64) -> Option<TriggerDecision> {
        if self.config.stages.is_empty() {
            return None;
        }
        let normalized = normalize_auth_prompt_text(chunk);
        if !normalized.is_empty() {
            self.append_to_buffer(&normalized);
        }
        if self.stage_since_ms.is_none() {
            self.stage_since_ms = Some(now_ms);
        }
        // D3: a shell prompt means the previous round finished; re-arm at
        // stage 1 (tssh "expect re-arms after the login sequence").
        if has_shell_prompt(&self.buffer) {
            self.reset_stage(now_ms);
            return None;
        }
        // Case pre-match answers without advancing the cursor (contract §0).
        if let Some(decision) = self.try_case_match(now_ms) {
            return Some(decision);
        }
        if let Some(decision) = self.try_stage_match(now_ms) {
            return Some(decision);
        }
        self.check_timeout(now_ms)
    }

    fn append_to_buffer(&mut self, normalized: &str) {
        self.buffer.push_str(normalized);
        let overflow = self.buffer.len().saturating_sub(MAX_BUFFER_BYTES);
        if overflow > 0 {
            let mut cut = overflow;
            while !self.buffer.is_char_boundary(cut) {
                cut += 1;
            }
            self.buffer.drain(..cut);
        }
    }

    fn consume_buffer(&mut self, through: usize) {
        self.buffer.drain(..through.min(self.buffer.len()));
    }

    fn reset_stage(&mut self, now_ms: u64) {
        self.stage_cursor = 0;
        self.stage_since_ms = Some(now_ms);
    }

    /// Re-arms the engine at stage 1 immediately. The Telnet declarative
    /// auto-login calls this on a failure-retry round (NyaTerm resets its
    /// `sent_username`/`sent_password` flags): after a rejection the host
    /// re-prompts from the first stage, so the next prompt must answer with
    /// the username again instead of waiting out the mid-sequence timeout.
    pub fn reset(&mut self, now_ms: u64) {
        self.reset_stage(now_ms);
    }

    fn try_case_match(&mut self, now_ms: u64) -> Option<TriggerDecision> {
        let case = self.config.stages[self.stage_cursor].case.as_ref()?;
        let matched = case.pattern.find(&self.buffer)?;
        let (kind, segments, command) = answer_segments(
            &case.answer,
            self.config.sleep_ms,
            self.config.pass_sleep,
            now_ms,
        );
        // Consume the matched text so the same case text cannot answer twice;
        // the stage cursor intentionally stays put.
        self.consume_buffer(matched.end());
        Some(TriggerDecision {
            stage: self.stage_cursor + 1,
            kind,
            segments,
            command,
        })
    }

    fn try_stage_match(&mut self, now_ms: u64) -> Option<TriggerDecision> {
        // Copy the match span and answer out first: `consume_buffer` mutates
        // the buffer the regex match borrows.
        let (match_end, answer) = {
            let stage = &self.config.stages[self.stage_cursor];
            let matched = stage.pattern.find(&self.buffer)?;
            (matched.end(), stage.answer.clone())
        };
        // D2: consume the matched text so later chunks continue past it.
        self.consume_buffer(match_end);
        let (kind, segments, command) = answer_segments(
            &answer,
            self.config.sleep_ms,
            self.config.pass_sleep,
            now_ms,
        );
        let stage_number = self.stage_cursor + 1;
        // Advance (wrapping so a completed sequence can run again without a
        // shell prompt in between) and restart the next stage's timer.
        self.stage_cursor = (self.stage_cursor + 1) % self.config.stages.len();
        self.stage_since_ms = Some(now_ms);
        Some(TriggerDecision {
            stage: stage_number,
            kind,
            segments,
            command,
        })
    }

    fn check_timeout(&mut self, now_ms: u64) -> Option<TriggerDecision> {
        // Timeout only applies mid-sequence: once a stage has been answered
        // and the engine waits for the next one. Waiting for stage 1 is an
        // event-driven, open-ended wait — an idle session must not spam
        // timeout decisions every `timeoutSecs`.
        if self.stage_cursor == 0 {
            return None;
        }
        let since = self.stage_since_ms?;
        if now_ms.saturating_sub(since) < self.config.timeout_secs.saturating_mul(1000) {
            return None;
        }
        let stage = self.stage_cursor + 1;
        // D3: a timeout zeroes the cursor and reports; the session keeps
        // running and the next window starts from stage 1.
        self.stage_cursor = 0;
        self.stage_since_ms = Some(now_ms);
        Some(TriggerDecision {
            stage,
            kind: TriggerKind::Timeout,
            segments: Vec::new(),
            command: None,
        })
    }
}
