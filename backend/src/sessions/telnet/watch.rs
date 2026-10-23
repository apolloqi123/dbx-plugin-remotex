//! Telnet layer.

use super::*;

/// What the declarative supervision decided after one output chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// Success regex matched after at least one prompt answer: the login is
    /// complete, supervision and answering stop.
    Success,
    /// Failure regex matched within budget: the host will re-prompt and the
    /// engine re-answers; `attempt` is the 1-based re-send round.
    Retry { attempt: u8 },
    /// Failure hit past the retry budget: the session closes into the exit
    /// overlay with a readable reason.
    Exhausted,
}

/// Success/failure/retry supervision for the declarative form (NyaTerm
/// `TelnetAutoLogin` semantics with prompt answering delegated to the shared
/// expect engine). A small rolling tail gives anchored success patterns like
/// `[$#>]\s*$` a stable end-of-buffer to match against; failure detection
/// runs regardless of whether an answer went out, success only after one did
/// (NyaTerm's `sent_username || sent_password` guard).
pub struct DeclarativeWatch {
    success: Regex,
    failure: Regex,
    max_retries: u8,
    retries: u8,
    tail: String,
    state: WatchState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WatchState {
    Active,
    Completed,
    Failed,
}

impl fmt::Debug for DeclarativeWatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The tail holds host output only, but it is still terminal content —
        // report its size, never its text.
        formatter
            .debug_struct("DeclarativeWatch")
            .field("max_retries", &self.max_retries)
            .field("retries", &self.retries)
            .field("tail_chars", &self.tail.chars().count())
            .field("state", &self.state)
            .finish()
    }
}

impl DeclarativeWatch {
    pub(crate) fn new(success: Regex, failure: Regex, max_retries: u8) -> Self {
        Self {
            success,
            failure,
            max_retries,
            retries: 0,
            tail: String::new(),
            state: WatchState::Active,
        }
    }

    /// Feeds one output chunk. `answered` tells whether at least one prompt
    /// answer went out this session (arms the success check).
    pub fn observe(&mut self, text: &str, answered: bool) -> Option<WatchEvent> {
        if self.state != WatchState::Active {
            return None;
        }
        self.push_tail(text);
        // Failure takes precedence over success/prompts (NyaTerm order): a
        // rejection banner that happens to end in a prompt must count as a
        // failure, not re-trigger an answer into a dead session.
        if self.failure.is_match(&self.tail) {
            if self.retries < self.max_retries {
                self.retries += 1;
                // Clear the tail so the same failure text cannot re-count on
                // every subsequent chunk (NyaTerm clears its window too).
                self.tail.clear();
                return Some(WatchEvent::Retry {
                    attempt: self.retries,
                });
            }
            self.state = WatchState::Failed;
            return Some(WatchEvent::Exhausted);
        }
        if answered && self.success.is_match(&self.tail) {
            self.state = WatchState::Completed;
            return Some(WatchEvent::Success);
        }
        None
    }

    /// Initial attempt plus the re-send rounds used so far; the readable
    /// close reason reports this when the budget is exhausted.
    pub fn attempts_used(&self) -> u8 {
        self.retries.saturating_add(1)
    }

    fn push_tail(&mut self, text: &str) {
        self.tail.push_str(text);
        let overflow = self
            .tail
            .chars()
            .count()
            .saturating_sub(DECL_WATCH_TAIL_CHARS);
        if overflow > 0 {
            self.tail = self.tail.chars().skip(overflow).collect();
        }
    }
}
