//! Metrics support.

use super::*;

/// The only authorization protocol this implementation produces and accepts.
pub(crate) const X11_AUTH_PROTOCOL: &str = "MIT-MAGIC-COOKIE-1";

/// Fake-cookie size in bytes; OpenSSH generates the same 16-byte value.
pub(crate) const COOKIE_LEN: usize = 16;

/// Verdict for the first bytes a remote X application writes into an x11
/// channel (the X connection setup block). Mirrors OpenSSH's client-side
/// check: a 12-byte fixed header, then the authorization protocol name and
/// data, each padded to a 4-byte boundary; byte order comes from the first
/// byte (`0x6c` LSB / `0x42` MSB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SetupInspection {
    /// Not enough bytes yet; buffer and read again.
    Incomplete,
    /// Authorized: rewrite `COOKIE_LEN` bytes at `cookie_offset` with the
    /// local display's real cookie (OpenSSH substitutes fake -> real here)
    /// and relay the whole buffer as-is.
    Accept { cookie_offset: usize },
    /// Malformed or carrying the wrong authorization data: the channel must
    /// be dropped, never relayed.
    Reject(&'static str),
}

/// Inspects the first setup bytes against the cookie issued for the session.
pub(crate) fn inspect_setup(buffer: &[u8], expected: &FakeCookie) -> SetupInspection {
    if buffer.len() < 12 {
        return SetupInspection::Incomplete;
    }
    let least_significant = buffer[0] == 0x6c;
    if !least_significant && buffer[0] != 0x42 {
        return SetupInspection::Reject("X setup has a bad byte-order byte");
    }
    let u16_at = |offset: usize| -> u16 {
        if least_significant {
            u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
        } else {
            u16::from_be_bytes([buffer[offset], buffer[offset + 1]])
        }
    };
    let protocol_len = u16_at(6) as usize;
    let data_len = u16_at(8) as usize;
    let cookie_offset = 12 + pad4(protocol_len);
    let total_len = cookie_offset + pad4(data_len);
    if buffer.len() < total_len {
        return SetupInspection::Incomplete;
    }
    if protocol_len != X11_AUTH_PROTOCOL.len()
        || &buffer[12..12 + protocol_len] != X11_AUTH_PROTOCOL.as_bytes()
    {
        return SetupInspection::Reject("X setup uses a different authorization protocol");
    }
    if data_len != COOKIE_LEN {
        return SetupInspection::Reject("X setup carries an unexpected cookie length");
    }
    if buffer[cookie_offset..cookie_offset + COOKIE_LEN] != *expected.bytes() {
        return SetupInspection::Reject("X setup cookie does not match the issued fake cookie");
    }
    SetupInspection::Accept { cookie_offset }
}

/// X protocol fields are padded to 4-byte boundaries.
pub(crate) fn pad4(len: usize) -> usize {
    (len + 3) & !3
}

/// Bounds the bytes buffered while an x11 channel's setup block is still
/// pending. A well-formed setup is a few hundred bytes; a peer that keeps
/// writing past this bound is malformed or hostile and must be cut off.
pub(crate) const MAX_SETUP_BYTES: usize = 16 * 1024;

/// Accumulates the first bytes of an accepted x11 channel until the X
/// connection setup block is complete. Pure and unit-tested; the
/// `server_channel_open_x11` handler in `ssh.rs` feeds it every incoming
/// `ChannelMsg::Data` and relays nothing until it yields a final verdict,
/// so a wrong cookie never reaches the local display.
#[derive(Default)]
pub(crate) struct SetupBuffer {
    bytes: Vec<u8>,
}

impl SetupBuffer {
    /// Appends a data chunk and re-evaluates the setup block. Memory stays
    /// bounded: once the pending bytes exceed [`MAX_SETUP_BYTES`] without a
    /// verdict, the channel is rejected outright.
    pub(crate) fn push(&mut self, data: &[u8], expected: &FakeCookie) -> SetupInspection {
        if self.bytes.len() + data.len() > MAX_SETUP_BYTES {
            return SetupInspection::Reject("X setup block exceeds the size bound");
        }
        self.bytes.extend_from_slice(data);
        inspect_setup(&self.bytes, expected)
    }

    /// The accumulated bytes, relayed verbatim on `Accept`.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
