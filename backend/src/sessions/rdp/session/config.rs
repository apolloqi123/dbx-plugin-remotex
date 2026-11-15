//! Metrics support.

use super::*;

// —— engine config builder ——

pub(crate) fn build_ironrdp_config(
    session_id: &str,
    entry: &Arc<RdpSessionEntry>,
    generation: u64,
    emitter: &PluginEmitter,
) -> Result<IronRdpConfig, String> {
    let verifier = Arc::new(RdpCertificateVerifier {
        emitter: emitter.clone(),
        broker: entry.broker.clone(),
        known_certs: entry.known_certs.clone(),
        session_id: session_id.to_string(),
        policy: entry.cert_policy,
        generation: entry.generation.clone(),
        own_generation: generation,
        cert_wait: entry.cert_wait.clone(),
    });
    let mut builder = IronRdpConfigBuilder::new()
        .with_destination(IronRdpDestination::from_parts(
            entry.host.clone(),
            entry.port,
        ))
        .with_transport(IronRdpTransportKind::Direct)
        .with_username(entry.username.clone())
        .with_domain(entry.domain.clone())
        // Credential boundary: the Zeroizing-held password is materialized
        // once here per generation (vendored storage is RDP-1 scope); our
        // side never keeps a plaintext copy.
        .with_password(
            entry
                .password
                .as_ref()
                .map(|p| p.as_str())
                .unwrap_or_default(),
        )
        .with_desktop_width(entry.width.load(Ordering::Acquire))
        .with_desktop_height(entry.height.load(Ordering::Acquire))
        .with_desktop_scale_factor(100)
        .with_color_depth(32)
        .with_credssp(entry.use_nla)
        // TLS is always on: CredSSP rides on TLS and the legacy "standard
        // RDP security" path is never advertised (checklist §3-C).
        .with_tls(true)
        .with_autologon(true)
        // NyaTerm: IronRDP 0.17 decodes compressed FastPath bitmaps
        // inconsistently after reactivation — do not advertise bulk
        // compression.
        .with_compression(false)
        .with_server_pointer(true)
        .with_client_build(client_build())
        .with_client_dir("C:\\Windows\\System32\\mstscax.dll")
        .with_client_name(client_name())
        .with_platform(current_platform())
        .with_server_certificate_verifier(verifier);

    if entry.clipboard_enabled {
        builder = builder
            .with_clipboard(ClipboardType::Enable)
            .with_cliprdr_factory({
                let session_id = session_id.to_string();
                let stage = entry.clipboard_stage.clone();
                let files = entry.clipboard_files.clone();
                let emitter = emitter.clone();
                move |proxy| {
                    Box::new(TextClipboardBackendFactory {
                        session_id: session_id.clone(),
                        stage: stage.clone(),
                        files: files.clone(),
                        emitter: emitter.clone(),
                        proxy: Arc::new(StdMutex::new(proxy)),
                    })
                }
            });
    } else {
        entry.clipboard_stage.clear();
        entry.clipboard_files.clear();
        builder = builder.with_clipboard(ClipboardType::Disable);
    }

    builder
        .build()
        .map_err(|error| format!("Unable to build RDP config: {error}"))
}

pub(crate) fn client_build() -> u32 {
    env!("CARGO_PKG_VERSION")
        .split('.')
        .take(3)
        .fold(0_u32, |acc, part| {
            let number = part
                .split(['-', '+'])
                .next()
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0);
            acc.saturating_mul(100).saturating_add(number)
        })
}

pub(crate) fn client_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "DBX-SSH".to_string())
}

pub(crate) fn current_platform() -> MajorPlatformType {
    #[cfg(target_os = "windows")]
    {
        MajorPlatformType::WINDOWS
    }
    #[cfg(target_os = "macos")]
    {
        MajorPlatformType::MACINTOSH
    }
    #[cfg(target_os = "ios")]
    {
        MajorPlatformType::IOS
    }
    #[cfg(target_os = "android")]
    {
        MajorPlatformType::ANDROID
    }
    #[cfg(all(
        not(target_os = "windows"),
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    ))]
    {
        MajorPlatformType::UNIX
    }
}
