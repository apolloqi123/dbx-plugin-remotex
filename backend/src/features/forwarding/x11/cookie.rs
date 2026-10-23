//! Metrics support.

use super::*;

/// One-shot fake cookie generated per armed session: it is sent to the SSH
/// server inside `x11-req` (and configured via xauth on the remote side),
/// and every incoming x11 channel's first packet must carry it before
/// anything is relayed to the local display. The real `.Xauthority` cookie
/// never leaves the machine (spike §3.3).
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FakeCookie {
    bytes: [u8; COOKIE_LEN],
    hex: String,
}

impl std::fmt::Debug for FakeCookie {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The cookie is a bearer secret: never print its value.
        f.debug_struct("FakeCookie").finish_non_exhaustive()
    }
}

impl FakeCookie {
    /// Draws a fresh cookie from the OS CSPRNG.
    pub(crate) fn generate() -> Result<Self, String> {
        let mut bytes = [0u8; COOKIE_LEN];
        getrandom::fill(&mut bytes).map_err(|error| format!("OS CSPRNG failure: {error}"))?;
        let mut hex = String::with_capacity(COOKIE_LEN * 2);
        for byte in bytes {
            hex.push_str(&format!("{byte:02x}"));
        }
        Ok(Self { bytes, hex })
    }

    /// The value to send in `x11-req` (lowercase hex, as xauth prints it).
    pub(crate) fn hex(&self) -> &str {
        &self.hex
    }

    pub(crate) fn bytes(&self) -> &[u8; COOKIE_LEN] {
        &self.bytes
    }
}

/// Family codes from Xau(3).
pub(crate) const FAMILY_LOCAL: u16 = 256;

pub(crate) const FAMILY_WILD: u16 = 65535;

/// One decoded `.Xauthority` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct XauthRecord {
    family: u16,
    number: String,
    name: Vec<u8>,
    data: Vec<u8>,
}

/// Decodes the binary xauth store (big-endian `u16` length-prefixed fields:
/// family, address, display number, name, data). A truncated trailing record
/// is dropped instead of failing the whole file — a torn store must degrade
/// to "no real cookie", never to a parse error.
pub(crate) fn parse_xauthority(bytes: &[u8]) -> Vec<XauthRecord> {
    let mut reader = XauthReader {
        bytes,
        cursor: 0usize,
    };
    let mut records = Vec::new();
    while let Some(family) = reader.u16() {
        // Each read borrows the reader, so own the bytes immediately to let
        // the next `bytes()` call take `&mut` again.
        let Some(_address) = reader.bytes().map(<[u8]>::to_vec) else {
            break;
        };
        let Some(number) = reader.bytes().map(<[u8]>::to_vec) else {
            break;
        };
        let Some(name) = reader.bytes().map(<[u8]>::to_vec) else {
            break;
        };
        let Some(data) = reader.bytes().map(<[u8]>::to_vec) else {
            break;
        };
        records.push(XauthRecord {
            family,
            number: String::from_utf8_lossy(&number).into_owned(),
            name: name.to_vec(),
            data: data.to_vec(),
        });
    }
    records
}

/// Length-prefixed field reader over the xauth binary format.
pub(crate) struct XauthReader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl XauthReader<'_> {
    fn u16(&mut self) -> Option<u16> {
        let slice = self.bytes.get(self.cursor..self.cursor + 2)?;
        self.cursor += 2;
        Some(u16::from_be_bytes([slice[0], slice[1]]))
    }

    fn bytes(&mut self) -> Option<&[u8]> {
        let length = self.u16()? as usize;
        let slice = self.bytes.get(self.cursor..self.cursor + length)?;
        self.cursor += length;
        Some(slice)
    }
}

/// Picks the real `MIT-MAGIC-COOKIE-1` for the display: family LOCAL or WILD
/// with the same display number. The address field is ignored on purpose —
/// local records differ across platforms and display managers (empty vs.
/// hostname), and xauth's own CLI lookup keys on family + number too.
pub(crate) fn find_real_cookie(records: &[XauthRecord], display_number: u32) -> Option<Vec<u8>> {
    let number = display_number.to_string();
    records
        .iter()
        .filter(|record| {
            matches!(record.family, FAMILY_LOCAL | FAMILY_WILD) && record.number == number
        })
        .find(|record| record.name == X11_AUTH_PROTOCOL.as_bytes())
        .map(|record| record.data.clone())
}

/// `$XAUTHORITY` or `$HOME/.Xauthority`; `None` when neither resolves.
pub(crate) fn xauthority_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("XAUTHORITY") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let home = home.trim();
    if home.is_empty() {
        return None;
    }
    Some(Path::new(home).join(".Xauthority"))
}

/// Looks up the real cookie for the display; `None` means "not found" and
/// the bridge falls back to relaying the fake cookie (which only works when
/// the local X server runs with access control disabled — the same fallback
/// OpenSSH takes on its "no xauth data" path).
pub(crate) fn load_real_cookie(display: &DisplayTarget) -> Option<Vec<u8>> {
    let bytes = std::fs::read(xauthority_path()?).ok()?;
    find_real_cookie(&parse_xauthority(&bytes), display.number)
}

/// OpenSSH's fake->real substitution: rewrites the validated fake cookie
/// inside a setup block with the display's real cookie. `None`, an
/// odd-length cookie, or an offset out of range relay the fake unchanged —
/// the same fallback OpenSSH takes when it has no xauth data (only works
/// against a local X server with access control disabled). Pure and
/// unit-tested; the caller feeds `load_real_cookie` as `real`.
pub(crate) fn substitute_cookie(setup: &mut [u8], cookie_offset: usize, real: Option<&[u8]>) {
    let Some(real) = real else {
        return;
    };
    if real.len() != COOKIE_LEN {
        return;
    }
    let end = match cookie_offset.checked_add(COOKIE_LEN) {
        Some(end) => end,
        None => return,
    };
    if setup.len() >= end {
        setup[cookie_offset..end].copy_from_slice(real);
    }
}
