//! Telnet layer.

use super::*;

/// Parser states. `Ground` passes payload bytes through; the rest consume an
/// in-flight IAC sequence that may span chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ParseState {
    #[default]
    Ground,
    /// IAC seen; the next byte selects the sequence shape.
    Iac,
    /// WILL/WONT/DO/DONT seen; `verb` holds the request byte.
    Negotiation { verb: u8 },
    /// Inside `SB <option> ... SE` — every byte until the closing `SE` is
    /// swallowed (the plugin never inspects subnegotiation content).
    Subnegotiation,
    /// IAC inside a subnegotiation: `SE` closes, `IAC IAC` is a literal
    /// 0xFF value byte, anything else is malformed and resynchronizes.
    SubnegotiationIac,
}

/// Streaming IAC stripper. Feeds wire chunks in, yields application payload
/// bytes plus negotiation reply bytes. The state machine lives across
/// `feed` calls, so sequences broken across TCP segments parse correctly.
#[derive(Debug, Default)]
pub struct TelnetParser {
    state: ParseState,
}

impl TelnetParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consumes one wire chunk; returns `(payload, replies)`.
    pub fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut payload = Vec::with_capacity(chunk.len());
        let mut replies = Vec::new();
        for &byte in chunk {
            match self.state {
                ParseState::Ground => match byte {
                    IAC => self.state = ParseState::Iac,
                    other => payload.push(other),
                },
                ParseState::Iac => {
                    self.state = match byte {
                        IAC => {
                            // IAC IAC → literal 0xFF payload byte.
                            payload.push(IAC);
                            ParseState::Ground
                        }
                        WILL | WONT | DO | DONT => ParseState::Negotiation { verb: byte },
                        SB => ParseState::Subnegotiation,
                        // NOP / other one-byte commands: swallowed silently.
                        _ => ParseState::Ground,
                    };
                }
                ParseState::Negotiation { verb } => {
                    self.state = ParseState::Ground;
                    let option = byte;
                    match (verb, option) {
                        (WILL, OPT_ECHO) | (WILL, OPT_SGA) => {
                            replies.extend_from_slice(&[IAC, DO, option]);
                        }
                        (WILL, _) => replies.extend_from_slice(&[IAC, DONT, option]),
                        (DO, OPT_NAWS) => replies.extend_from_slice(&[IAC, WILL, option]),
                        (DO, _) => replies.extend_from_slice(&[IAC, WONT, option]),
                        // Peer refusals (WONT/DONT) need no answer.
                        _ => {}
                    }
                }
                ParseState::Subnegotiation => {
                    if byte == IAC {
                        self.state = ParseState::SubnegotiationIac;
                    }
                }
                ParseState::SubnegotiationIac => {
                    self.state = match byte {
                        SE => ParseState::Ground,
                        IAC => ParseState::Subnegotiation,
                        // Malformed: resynchronize at ground state.
                        _ => ParseState::Ground,
                    };
                }
            }
        }
        (payload, replies)
    }
}
