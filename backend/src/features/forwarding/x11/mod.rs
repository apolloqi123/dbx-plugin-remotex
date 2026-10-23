//! X11 forwarding protocol pieces (OpenSSH `-X` parity, MVP).
//!
//! Everything in this module is pure and unit-tested: DISPLAY parsing, the
//! one-shot fake cookie handed to the server, the first X-setup-packet
//! inspection (byte-order aware, OpenSSH-compatible), `.Xauthority` parsing
//! for the local display's real cookie, and the per-connection admission
//! gate. The async bridge relaying an accepted channel into the local X
//! server is wired from `ssh.rs` (see `server_channel_open_x11` there):
//! russh's default handler accepts every incoming x11 channel, so the
//! explicit fail-closed gate below is the security boundary
//! (docs/plans/SPIKE_X11_FORWARDING.zh-CN.md §3.4).
//!
//! No DISPLAY is probed anywhere in this module, so the tests never depend
//! on an X server being present (CI runners have none).

// The bridge is driven by `server_channel_open_x11` in `ssh.rs`; the pure
// pieces additionally carry their own unit tests below.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::AsyncWriteExt;

mod bridge;
mod cookie;
mod display;
mod gate;
mod setup;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use bridge::*;
#[allow(unused_imports)]
pub(crate) use cookie::*;
#[allow(unused_imports)]
pub(crate) use display::*;
#[allow(unused_imports)]
pub(crate) use gate::*;
#[allow(unused_imports)]
pub(crate) use setup::*;
