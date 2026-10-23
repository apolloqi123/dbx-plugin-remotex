//! Telnet layer.

use super::*;

/// Enter-key wire form: CRLF (default, most BBS/Unix line disciplines), bare
/// CR, or LF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EnterMode {
    #[default]
    Crlf,
    Cr,
    Lf,
}

impl EnterMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "crlf" => Some(Self::Crlf),
            "cr" => Some(Self::Cr),
            "lf" => Some(Self::Lf),
            _ => None,
        }
    }
}

/// Backspace wire form: DEL 0x7F (default, what xterm sends) or Ctrl+H 0x08
/// for hosts whose line discipline expects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackspaceMode {
    #[default]
    Del,
    CtrlH,
}

impl BackspaceMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "del" => Some(Self::Del),
            "ctrl_h" | "ctrlH" => Some(Self::CtrlH),
            _ => None,
        }
    }
}

/// Converts one keyboard payload into the wire form the host expects:
/// backspace first (`0x7F` → `0x08` under `CtrlH`), then Enter (`\r` per
/// mode; an already-paired `\r\n` collapses to one Enter so CRLF mode cannot
/// double it). Pure — unit-tested below.
pub fn transform_input(data: &[u8], enter: EnterMode, backspace: BackspaceMode) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + 8);
    let mut previous_cr = false;
    for &byte in data {
        let byte = match (backspace, byte) {
            (BackspaceMode::CtrlH, 0x7f) => 0x08,
            (_, other) => other,
        };
        match byte {
            // One Enter per CR; a CR already followed by LF stays one Enter.
            0x0d => {
                previous_cr = true;
                match enter {
                    EnterMode::Crlf => {
                        output.push(0x0d);
                        output.push(0x0a);
                    }
                    EnterMode::Cr => output.push(0x0d),
                    EnterMode::Lf => output.push(0x0a),
                }
            }
            0x0a if previous_cr => {
                previous_cr = false;
            }
            other => {
                previous_cr = false;
                output.push(other);
            }
        }
    }
    output
}

/// Builds an RFC 1073 NAWS subnegotiation: `IAC SB NAWS <cols hi lo> <rows
/// hi lo> IAC SE`, with `0xFF` value bytes escaped as `IAC IAC`. Pure.
pub fn naws_frame(cols: u16, rows: u16) -> Vec<u8> {
    let mut frame = vec![IAC, SB, OPT_NAWS];
    for half in [cols, rows] {
        let [hi, lo] = half.to_be_bytes();
        for byte in [hi, lo] {
            if byte == IAC {
                frame.push(IAC);
            }
            frame.push(byte);
        }
    }
    frame.extend_from_slice(&[IAC, SE]);
    frame
}

impl<'de> Deserialize<'de> for EnterMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        EnterMode::parse(&value)
            .ok_or_else(|| serde::de::Error::custom("enterMode must be crlf, cr or lf"))
    }
}

impl<'de> Deserialize<'de> for BackspaceMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        BackspaceMode::parse(&value)
            .ok_or_else(|| serde::de::Error::custom("backspaceMode must be del or ctrl_h"))
    }
}
