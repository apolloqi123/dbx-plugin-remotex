//! Session import from third-party SSH clients (parity wave P2-2, M7 four
//! source additions). Seven source formats are parsed into a common
//! in-memory shape:
//!
//! - **MobaXterm** `.mxtsessions`: a plain INI where every `Bookmarks*`
//!   section lists `SessionName=#109#0%host%port%user%...` pipe-separated
//!   connection strings. No password material exists in the file.
//! - **Xshell** `.xts`: a ZIP of per-session `.xsh` INI files (GBK entry
//!   names on Chinese Windows installs). Only session metadata lives in
//!   `.xsh`; a key file is referenced by name, never embedded.
//! - **WindTerm** `.sessions` + `user.config`: JSON; `session.autoLogin`
//!   holds either a plaintext credential JSON or a base64 AES-256-CBC blob
//!   whose key/IV are PBKDF2-HMAC-SHA3-512 derived from the master password
//!   and the `application.fingerprint` salt.
//! - **SecureCRT** `.xml` session export: a tree of `<key name="...">`
//!   frames with `<string>/<dword>` leaves (dwords are zero-padded 8-digit
//!   hex). The export only ever contains encrypted password blobs, which are
//!   never decoded — sessions keep password semantics with no material.
//! - **FinalShell** conn directory packed as a ZIP (or a bare
//!   `*_connect_config.json`): `folder.json` builds the group tree; the
//!   per-connection JSON uses FinalShell's own DES scheme for passwords,
//!   which stays unread and flagged like SecureCRT.
//! - **Electerm** bookmarks JSON: a bare array, or the `{bookmarks,
//!   bookmarkGroups}` object; groups are resolved through `bookmarkIds` /
//!   `bookmarkGroupIds` parent chains.
//! - **Termius** export JSON: `hosts` + `groups` / `ssh_configs` /
//!   `identities` / `keys` under an optional `data` wrapper. Secret fields
//!   that arrive as encrypted sync blobs (`BA…` base64) are dropped, never
//!   decrypted; plaintext passwords and PEM private keys are only flagged as
//!   present (`has_secret`) and never carried into the preview model.
//! - **OpenSSH config** (`~/.ssh/config`, WT-3, the 8th source): plain text.
//!   `Host` (wildcard patterns carry defaults only and never become entries),
//!   `Hostname`, `User`, `Port`, `IdentityFile` (the path is mapped, never
//!   read — no key material can enter the preview model), `UserKnownHostsFile`
//!   and `Match host/user` (all criteria must be `host`/`user`) are supported;
//!   first-value-wins semantics apply across blocks. The pipeline receives
//!   uploaded bytes only, so `Include` cannot be followed: the directive is
//!   annotated "(not followed)" instead of reading further files (there is no
//!   recursion, hence no cycle risk). `ProxyCommand` / `ProxyJump` are only
//!   recognized and annotated "needs manual mapping" — external commands are
//!   never executed, mirroring the tssh Expect trust model (ciphertext and
//!   external commands are not run automatically). `Match` blocks with other
//!   criteria degrade to a skipped-with-reason note on every imported row.
//!
//! Parsed secrets exist only for the duration of one preview request. The
//! streaming transport retains source bytes only until `import/preview/finish`
//! returns a sanitized preview; no imported connection or credential is ever
//! persisted by this plugin.
//!
//! All parsers are written for hostile input: no panicking indexing (only
//! `get`/`strip_*`/checked parsing), per-file and aggregate size caps on
//! archives, and malformed entries degrade to "skipped" instead of failing
//! the whole import.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

mod common;
mod electerm;
mod finalshell;
mod moba;
mod securecrt;
mod ssh_config;
mod stream;
mod termius;
mod windterm;
mod xshell;

#[cfg(test)]
mod tests;

use common::*;
use electerm::*;
use finalshell::*;
use moba::*;
use securecrt::*;
use ssh_config::*;
#[allow(unused_imports)]
use stream::*;
pub(crate) use stream::{
    parse_uploaded, read_session_file, remove_legacy_store, session_field_values, ImportStream,
};
use termius::*;
use windterm::*;
use xshell::*;

/// Hard caps for Xshell ZIP archives (zip-bomb protection).
pub const MAX_ZIP_ENTRIES: usize = 10_000;
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
/// Total byte budget across the main export and optional WindTerm user.config.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
/// Binary chunks stay well below the host's JSON message limit and match SFTP's
/// bounded, acknowledged upload pattern.
pub const IMPORT_CHUNK_LIMIT: usize = 256 * 1024;
/// Bound the JSON preview/export response to remain below the SDK 8 MiB limit.
pub const MAX_PREVIEW_SESSIONS: usize = 1000;
const MAX_PREVIEW_TEXT_BYTES: usize = 4096;
/// At most this many unfinished imports may retain process memory at once.
pub const MAX_IMPORT_TASKS: usize = 4;
/// Declared input budgets across every live task share one process cap.
pub const MAX_IMPORT_MEMORY_BYTES: u64 = MAX_INPUT_BYTES as u64;
/// Unfinished preview uploads are discarded after this period.
pub const IMPORT_TASK_TTL: Duration = Duration::from_secs(5 * 60);
const LEGACY_STORE_FILE: &str = "imported-connections.json";
/// Error returned when WindTerm encryption is detected (master-password
/// switch on in `user.config`) but the caller did not supply the password.
pub const WINDTERM_MASTER_PASSWORD_REQUIRED: &str = "WindTerm master password is required";

// OpenSSH config caps (WT-3): hostile configs degrade instead of blowing up.
/// Line budget for one config file; a bigger file fails closed.
pub const MAX_SSH_CONFIG_LINES: usize = 100_000;
/// Per-directive argument budget; oversized arguments degrade (directive
/// skipped) so giant values cannot flow into annotations.
pub const MAX_SSH_CONFIG_ARG_BYTES: usize = 4096;
/// Distinct unsupported-`Match` reasons carried into row descriptions.
pub const MAX_SSH_CONFIG_NOTES: usize = 3;
/// Byte-op budget for pattern matching across one parse: the two-pointer glob
/// is O(pattern × candidate), so the budget bounds star-heavy hostile inputs
/// without ever exceeding a fraction of a second of CPU.
pub const MAX_SSH_MATCH_WORK: u64 = 64 * 1024 * 1024;
/// Error returned when the pattern-matching budget is exhausted.
pub const SSH_CONFIG_MATCH_WORK_EXCEEDED: &str =
    "OpenSSH config pattern matching exceeds the complexity limit";

/// Preview secret-note codes: why an imported session has no credential
/// material despite password semantics. The frontend maps them to localized
/// copy (see `importWizard.note.*` in `lib/i18n.ts`); no import output is
/// persisted.
pub const SECRET_NOTE_ENCRYPTED: &str = "encrypted";
pub const SECRET_NOTE_NOT_CARRIED: &str = "not-carried";
/// OpenSSH config `IdentityFile`: only the key file path was mapped, the key
/// itself is never read or carried (WT-3).
pub const SECRET_NOTE_KEY_PATH_ONLY: &str = "key-path-only";

/// WindTerm KDF parameters: PBKDF2-HMAC-SHA3-512 over the master password,
/// salted with the raw `application.fingerprint` bytes, producing 48 bytes
/// (AES-256 key + CBC IV).
const WINDTERM_PBKDF2_ROUNDS: u32 = 100_000;
const WINDTERM_DERIVED_BYTES: usize = 48;

// ---------------------------------------------------------------------------
// MobaXterm (.mxtsessions)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Shared INI scanning (MobaXterm + Xshell .xsh)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Xshell (.xts ZIP of .xsh INI files)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// WindTerm (.sessions + user.config)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// SecureCRT (XML session export)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// FinalShell (conn directory packed as a ZIP, or a bare connection JSON)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Electerm (bookmarks JSON)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Termius (exported JSON)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// OpenSSH config (~/.ssh/config) — the 8th source (WT-3)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Streaming preview protocol
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
