//! VNC remote-desktop sessions (RFB 6143 client, nyaterm-parity P2 2d).
//!
//! Runtime layout mirrors `telnet_session.rs`: one entry per live session in
//! the runtime's own table (a VNC connection carries its own TCP stream and
//! can never live on the SSH `SessionEntry`), structured input arrives on the
//! `vnc/input` (alias `vnc/write`) JSON method with the keysym already mapped
//! by the frontend, and framebuffer updates answer as 44-byte-header RGBA
//! patch frames on the binary channel `vnc/frame/{sessionId}` — the same
//! patch protocol NyaTerm uses (`sequence u64 LE`, desktop W/H, x/y/w/h,
//! stride, pixel format, payload length, all `u32 LE`). The wire contract
//! (field order, little-endian byte order, stride semantics, validation
//! invariants) is documented in `docs/architecture/PROTOCOL.zh-CN.md` § VNC 帧补丁.
//! Lifecycle changes
//! surface as `vnc/session/state` events (`connecting`/`connected`/
//! `closed`/`error`).
//!
//! Engine: upstream `vnc-rs 0.6` (HsuJv/vnc-rs, MIT OR Apache-2.0) with
//! `PixelFormat::rgba()`, encodings ZRLE + Raw (+ DesktopSizePseudo) only —
//! Tight is deliberately not advertised, and a server that still sends a
//! Tight/JPEG rectangle fails the session with a readable error. Due
//! diligence (why route A, what the upstream limits cover) lives in
//! `docs/plans/SPIKE_VNC_SESSION.zh-CN.md`.
//!
//! Bounds (tighter than upstream's per-message decoder limits):
//! - framebuffer at most 3840x2160 (a `SetResolution` outside the range, or
//!   a rect outside the framebuffer, fails the session);
//! - a patch payload is inherently ≤ 3840*2160*4 < 64 MiB and the encoder
//!   still re-validates stride/rect/payload on every frame;
//! - classic VNC-Auth passwords are rejected above 8 bytes at `vnc/start`
//!   (the protocol truncates silently otherwise);
//! - clipboard text is Latin-1 and ≤ 1 MiB in both directions.
//!
//! Reconnect: every worker run captures a `generation` counter; `vnc/
//! reconnect` (or the automatic retry loop) bumps it, the superseded worker
//! exits without emitting, and the new run repaints from the retained
//! framebuffer plus a full-refresh request so a reattached viewer gets a
//! complete frame. Transport failures retry up to `reconnectAttempts`
//! (default 3); authentication and protocol failures never retry.
//!
//! Safety note: classic VNC authentication is a 8-byte DES challenge and
//! None is cleartext — the plugin surfaces the "trusted networks only"
//! warning in the UI, keeps the password in `Zeroizing`, and never logs
//! passwords, challenge responses or clipboard text.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dbx_plugin_sdk::PluginEmitter;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, RwLock};
use vnc::{
    ClientKeyEvent, ClientMouseEvent, PixelFormat, Rect, Screen, VncClient, VncConnector,
    VncEncoding, VncError, VncEvent, X11Event,
};
use zeroize::Zeroizing;

mod frame;
mod runtime;
mod types;
mod worker;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use frame::*;
#[allow(unused_imports)]
pub(crate) use runtime::*;
#[allow(unused_imports)]
pub(crate) use types::*;
#[allow(unused_imports)]
pub(crate) use worker::*;
