//! Trigger send layer.

use super::*;

/// Parses a `sendText` answer: `\r` `\n` `\t` become control characters, `\|`
/// starts a new segment (the read loop pauses `sleepMs` between segments),
/// any other backslash stays literal.
pub fn parse_send_text(text: &str) -> Vec<String> {
    let mut segments: Vec<String> = vec![String::new()];
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.peek().copied() {
                Some('r') => {
                    chars.next();
                    segments
                        .last_mut()
                        .expect("segments never empty")
                        .push('\r');
                }
                Some('n') => {
                    chars.next();
                    segments
                        .last_mut()
                        .expect("segments never empty")
                        .push('\n');
                }
                Some('t') => {
                    chars.next();
                    segments
                        .last_mut()
                        .expect("segments never empty")
                        .push('\t');
                }
                Some('|') => {
                    chars.next();
                    segments.push(String::new());
                }
                _ => segments
                    .last_mut()
                    .expect("segments never empty")
                    .push('\\'),
            }
        } else {
            segments
                .last_mut()
                .expect("segments never empty")
                .push(character);
        }
    }
    segments
}

/// Builds the send plan for a text answer: `\|` segments with `sleep_ms`
/// pauses before every segment but the first.
pub(super) fn text_segments(text: &str, sleep_ms: u64) -> Vec<(Vec<u8>, u64)> {
    parse_send_text(text)
        .into_iter()
        .enumerate()
        .map(|(index, segment)| (segment.into_bytes(), u64::from(index > 0) * sleep_ms))
        .collect()
}

/// Builds the send plan for a secret/command answer: Enter is appended, and
/// `passSleep` controls the pacing (`none` single write, `each` char-by-char,
/// `enter` answer-then-pause-then-Enter).
pub fn pass_sleep_segments(
    answer: &str,
    sleep_ms: u64,
    pass_sleep: PassSleep,
) -> Vec<(Vec<u8>, u64)> {
    match pass_sleep {
        PassSleep::None => vec![(format!("{answer}\r").into_bytes(), 0)],
        PassSleep::Each => {
            let total = answer.chars().count();
            if total == 0 {
                return vec![(b"\r".to_vec(), 0)];
            }
            answer
                .chars()
                .enumerate()
                .map(|(index, character)| {
                    let mut bytes = character.to_string().into_bytes();
                    if index + 1 == total {
                        bytes.push(b'\r');
                    }
                    (bytes, u64::from(index > 0) * sleep_ms)
                })
                .collect()
        }
        PassSleep::Enter => {
            let mut segments = Vec::new();
            if !answer.is_empty() {
                segments.push((answer.as_bytes().to_vec(), 0));
            }
            segments.push((b"\r".to_vec(), sleep_ms));
            segments
        }
    }
}

/// Builds the send plan plus decision kind for a resolved stage answer.
/// `now_ms` timestamps TOTP generation (the code must be fresh at match time).
pub(super) fn answer_segments(
    answer: &StageAnswer,
    sleep_ms: u64,
    pass_sleep: PassSleep,
    now_ms: u64,
) -> AnswerPlan {
    match answer {
        StageAnswer::Text(text) => (TriggerKind::Text, text_segments(text, sleep_ms), None),
        StageAnswer::Secret(value) => (
            TriggerKind::Secret,
            pass_sleep_segments(value, sleep_ms, pass_sleep),
            None,
        ),
        StageAnswer::Totp(secret) => (
            TriggerKind::Secret,
            pass_sleep_segments(&totp_code(secret, now_ms), sleep_ms, pass_sleep),
            None,
        ),
        StageAnswer::Command(command) => {
            // The caller runs the command locally and turns the (trimmed)
            // stdout into a passSleep-shaped plan; segments stay empty until
            // then so no unresolved placeholder ever reaches the wire.
            (TriggerKind::Command, Vec::new(), Some(command.clone()))
        }
    }
}
