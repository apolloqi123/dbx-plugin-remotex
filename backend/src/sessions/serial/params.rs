//! Serial session layer.

use super::*;

/// Erase-byte mapping shared with the telnet session (`BackspaceMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackspaceMode {
    Del,
    CtrlH,
}

impl BackspaceMode {
    pub(crate) fn parse(value: Option<&String>) -> Self {
        match value.map(String::as_str) {
            Some("ctrl_h") => Self::CtrlH,
            _ => Self::Del,
        }
    }

    pub(crate) fn rewrite(&self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Del => data.to_vec(),
            Self::CtrlH => data
                .iter()
                .map(|byte| if *byte == 0x7F { 0x08 } else { *byte })
                .collect(),
        }
    }
}

/// Line parameters resolved from a start request. Validation is strict:
/// unknown strings are rejected before any open attempt, because silently
/// falling back to 8N1 masks misconfiguration (a device wired for 7E1 opened
/// as 8N1 just garbles output with no hint of the wrong parameter).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LineParams {
    pub(crate) data_bits: serialport::DataBits,
    pub(crate) parity: serialport::Parity,
    pub(crate) stop_bits: serialport::StopBits,
    pub(crate) flow_control: serialport::FlowControl,
}

pub(crate) fn parse_data_bits(value: Option<&String>) -> Result<serialport::DataBits, String> {
    match value.map(String::as_str) {
        None => Ok(serialport::DataBits::Eight),
        Some("5") => Ok(serialport::DataBits::Five),
        Some("6") => Ok(serialport::DataBits::Six),
        Some("7") => Ok(serialport::DataBits::Seven),
        Some("8") => Ok(serialport::DataBits::Eight),
        Some(other) => Err(format!(
            "serial/start: invalid dataBits \"{other}\" (expected 5, 6, 7 or 8)"
        )),
    }
}

pub(crate) fn parse_parity(value: Option<&String>) -> Result<serialport::Parity, String> {
    match value.map(String::as_str) {
        None | Some("none") => Ok(serialport::Parity::None),
        Some("even") => Ok(serialport::Parity::Even),
        Some("odd") => Ok(serialport::Parity::Odd),
        Some(other) => Err(format!(
            "serial/start: invalid parity \"{other}\" (expected none, even or odd)"
        )),
    }
}

pub(crate) fn parse_stop_bits(value: Option<&String>) -> Result<serialport::StopBits, String> {
    match value.map(String::as_str) {
        None | Some("1") => Ok(serialport::StopBits::One),
        Some("2") => Ok(serialport::StopBits::Two),
        Some(other) => Err(format!(
            "serial/start: invalid stopBits \"{other}\" (expected 1 or 2)"
        )),
    }
}

pub(crate) fn parse_flow_control(
    value: Option<&String>,
) -> Result<serialport::FlowControl, String> {
    match value.map(String::as_str) {
        None | Some("none") => Ok(serialport::FlowControl::None),
        Some("rts_cts") => Ok(serialport::FlowControl::Hardware),
        Some("xon_xoff") => Ok(serialport::FlowControl::Software),
        Some(other) => Err(format!(
            "serial/start: invalid flowControl \"{other}\" (expected none, rts_cts or xon_xoff)"
        )),
    }
}

pub(crate) fn resolve_line_params(request: &SerialStartRequest) -> Result<LineParams, String> {
    Ok(LineParams {
        data_bits: parse_data_bits(request.data_bits.as_ref())?,
        parity: parse_parity(request.parity.as_ref())?,
        stop_bits: parse_stop_bits(request.stop_bits.as_ref())?,
        flow_control: parse_flow_control(request.flow_control.as_ref())?,
    })
}

/// Baud is driver-defined, so any in-range rate passes (including custom
/// ones); only nonsense outside the same bounds the MVP clamped to is
/// rejected — now with an error instead of a silent clamp.
pub(crate) fn checked_baud_rate(baud_rate: u32) -> Result<u32, String> {
    if (50..=4_000_000).contains(&baud_rate) {
        Ok(baud_rate)
    } else {
        Err(format!(
            "serial/start: baudRate {baud_rate} out of range (50..=4000000)"
        ))
    }
}
