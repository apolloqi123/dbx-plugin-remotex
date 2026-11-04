//! 会话运行时：每活会话一条 entry（generation 守卫重连、Zeroizing 凭据、
//! 证书/剪贴板协作件），worker 沿 vendored 引擎线程泵帧/输入/指针事件，
//! 引擎配置在每代构建（凭据只在 with_password 边界物化一次）。
use super::cert::{CertPromptBroker, KnownCertStore, RdpCertificateVerifier};
use super::clipboard::{ClipboardStage, TextClipboardBackendFactory};
use super::error::{
    classify_connector_error_text, classify_session_error_text, connector_error_message,
    reconnect_delay, session_error_message, GenerationEnd, RdpErrorKind, ReconnectBudget,
};
use super::file_clipboard::{is_plugin_temp_path, FileClipboardStage, CLIPBOARD_FILE_CHUNK_BYTES};
use super::frame::RdpFrameState;
use super::input::send_wire_input;
use super::limits::{
    validate_clipboard_text, validate_desktop_size, MAX_DESKTOP_HEIGHT, MAX_DESKTOP_WIDTH,
};
use super::security::{resolve_security, CertPolicy};
use super::wire::{
    ClipboardFileEntry, RdpStartRequest, RdpWireInput, MAX_DOMAIN_LEN, MAX_HOST_LEN,
    MAX_UNICODE_TEXT_CHARS, MAX_USERNAME_LEN,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use dbx_plugin_sdk::PluginEmitter;
use ironrdp::client::config::{
    ClipboardType, Config as IronRdpConfig, ConfigBuilder as IronRdpConfigBuilder,
    Destination as IronRdpDestination, TransportKind as IronRdpTransportKind,
};
use ironrdp::client::rdp::{
    RdpClient as IronRdpClient, RdpInputEvent as IronRdpInputEvent, RdpOutputEvent,
};
use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, FileContentsRequest, FileDescriptor,
};
use ironrdp::input::Database as IronRdpInputDatabase;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

mod config;
mod runtime;
mod types;
mod worker;

#[allow(unused_imports)]
pub(crate) use config::*;
#[allow(unused_imports)]
pub(crate) use runtime::*;
#[allow(unused_imports)]
pub(crate) use types::*;
#[allow(unused_imports)]
pub(crate) use worker::*;
