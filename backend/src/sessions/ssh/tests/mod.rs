use super::*;
use russh::{cipher, kex, mac};

mod connection;
mod forwarding;
mod security;
mod terminal;
mod transfer;
mod tree;

fn auth_failure(remaining: &[MethodKind], partial_success: bool) -> AuthResult {
    let mut methods = russh::MethodSet::empty();
    for kind in remaining {
        methods.push(*kind);
    }
    AuthResult::Failure {
        remaining_methods: methods,
        partial_success,
    }
}

fn method_set(kinds: &[MethodKind]) -> MethodSet {
    MethodSet::from(kinds)
}

fn key_connection(path: &str, content: &str) -> StoredConnection {
    StoredConnection::from_lifecycle_params(&json!({
        "connection": {
            "id": "key-resolve",
            "host": "example.com",
            "port": 22,
            "username": "user",
            "external_config": {
                "authentication": "private-key",
                "private_key_path": path
            },
            "connection_secrets": { "private_key": content }
        }
    }))
    .unwrap()
}

/// 废弃测试密钥(本机为编写测试生成,无对应主机)。
const OPENSSH_ED25519_PLAIN: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACDkhRLM2wxmY826/LvkQeMRNf9pptlryFMSddhmTmhSrwAAAJiaGi/pmhov
6QAAAAtzc2gtZWQyNTUxOQAAACDkhRLM2wxmY826/LvkQeMRNf9pptlryFMSddhmTmhSrw
AAAEA41GJvRrU3mTnSyjUyLinDInc6VUNdHcvGr1te4YW7S+SFEszbDGZjzbr8u+RB4xE1
/2mm2WvIUxJ12GZOaFKvAAAADm1hdHJpeC1lZDI1NTE5AQIDBAUGBw==
-----END OPENSSH PRIVATE KEY-----";

/// 同上,口令为 "test-phrase"。
const OPENSSH_ED25519_ENCRYPTED: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABAd8e+k64
ZPDgMURR64zer1AAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIMcQDHXhSWOHA0pR
VYE3X8bEgjf0QVAcOzZ+6mZKzIa/AAAAoLKviiOSbTqOH2s7O+Y2AxZzne4kYYb0uZhpHH
eGHpS7VjhQLlN/B1cgyfNWqdROswIXdPbDL1uC5nzv8MAvH4lEobnNgZWVaxKoIAyTP3v+
c/3nQJQxJ1klouO/ojHcFFumbY4C+lRMGRhPIvtDj1alq5lvefD/3VmtK/jbeA8zuXcdv9
1QXkLgYJ5A3wpFPs7x/jgDn/v3e6jAwfZoZLU=
-----END OPENSSH PRIVATE KEY-----";

const ENCRYPTED_TEST_PASSPHRASE: &str = "test-phrase";

/// ssh-key 上游公开测试 fixture(user@example.com,无口令)。
const PPK3_ED25519_PLAIN: &str = "\
PuTTY-User-Key-File-3: ssh-ed25519
Encryption: none
Comment: user@example.com
Public-Lines: 2
AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XF
Sqti
Private-Lines: 1
AAAAILYGwiLRDBba4WxwpNRRc0cuxhfgXGVpINJuVsCPtZHt
Private-MAC: 94140d0344fad6aa1bf7b71e9c93db11ccac8a232f8a51e11c024869d608c82d";

/// 同一上游 fixture,口令为 "123"。
const PPK3_ED25519_ENCRYPTED: &str = "\
PuTTY-User-Key-File-3: ssh-ed25519
Encryption: aes256-cbc
Comment: user@example.com
Public-Lines: 2
AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XF
Sqti
Key-Derivation: Argon2id
Argon2-Memory: 8192
Argon2-Passes: 34
Argon2-Parallelism: 1
Argon2-Salt: 63d1d43f7bf7700720496646a2f5ec17
Private-Lines: 1
DyWtExZ3dxFutnb12tIwXBC6kWdozrvP+r6faHKBGDb4+qEar9XBiC0BmGySMHUi
Private-MAC: 52fd00d4ef47ebc506e4e709486c0c6bc0606e24fe2c6cb1b3d168f4da238a66";

/// 废弃测试密钥(SEC1 传统 EC PEM,russh 不支持此封装)。
const EC_SEC1_PEM: &str = "\
-----BEGIN EC PRIVATE KEY-----
MIIBaAIBAQQgOtPbXkrREPIS68niEPbISV5VZmM4655BvEie7U9k/4CggfowgfcC
AQEwLAYHKoZIzj0BAQIhAP////8AAAABAAAAAAAAAAAAAAAA////////////////
MFsEIP////8AAAABAAAAAAAAAAAAAAAA///////////////8BCBaxjXYqjqT57Pr
vVV2mIa8ZR0GsMxTsPY7zjw+J9JgSwMVAMSdNgiG5wSTamZ44ROdJreBn36QBEEE
axfR8uEsQkf4vOblY6RA8ncDfYEt6zOg9KE5RdiYwpZP40Li/hp/m47n60p8D54W
K84zV2sxXs7LtkBoN79R9QIhAP////8AAAAA//////////+85vqtpxeehPO5ysL8
YyVRAgEBoUQDQgAERtRtbHHreGHq8c0a0GvFRsZ+3BfCcILOVeIpInh5O9ddBZQI
Q/TyBph+JFB6VjJb6neA04jwsdD13REn2e7/og==
-----END EC PRIVATE KEY-----";

/// 废弃测试密钥(PKCS#8 + PBES2/hmacWithSHA256,口令 "test-phrase";
/// russh 的 PKCS#5 解析不支持该 PRF)。
const PKCS8_PBES2_ENCRYPTED: &str = "\
-----BEGIN ENCRYPTED PRIVATE KEY-----
MIICzzBJBgkqhkiG9w0BBQ0wPDAbBgkqhkiG9w0BBQwwDgQIOcKjFZoZ/fsCAggA
MB0GCWCGSAFlAwQBAgQQTfxxn8bDf+3cTfXy9sHVtgSCAoBF4F4/74FYKIDBbFxE
BXPvQRgnRTL6FWGD983/u9CueKi26cAty4N75cDEPwbq4xk/DMGhQTiSymv4SOK3
MBgZ/hb8jHWIp8SrZ2IvP14uM/EsVAvLgmnlu8JH/Q4BfYLbo4tvY88Wwx93GXdW
Ik6S30w+iDo/BTZ7by9mFITWD3DcIYpsrCgmn/MhQ/cRy7Eoe4wkMruMsY4aG4Tb
K1gDN/dLL/jrafcwDx1sjA71BmVf0bNKR9CBk1sZ5i5d4D0YSFRI4t8TG15cjb/l
NQwi9ykch3t6ojtQvHT645Z8kiTYjfPEyqgEWtSZd+69QAJ0Gl1XEqb+kH7eyMOL
oyMOfoX+Z3cEHCNY5/FZvFUTLmVX+OjzYY34LvzoSICeCr7HE+cnkRBEdpbLgyw9
LPlnpp7RWOXr8pidnr7vg7tN6sWnqM8JwPubDgl2gE1jUiqKHSWXHq6kEVEVA9Y4
uRBKE24j+FrPYGRjoKVXyPX0gjhqp66GDGaBCWs7LzdFO10IPxDKM6GJ8AxKLk9X
UVXqyvDfC9lh6YAHmTKmNYnKkEijdKkP7Uw57n8aWr1tw5xLyhABLZEGhPLcyazN
z7fnjJ+3LAEqD5dlZaczncsdFmZh1naKswCmSrwZOEbAeGbaZvdsHqspYwXGBTna
Q+KW8/vygV6WQZLnVu36rIkJq56Hzo3JFo3p2GOY/Pk1o82xFTm/fS3Pm3FWoapa
SiK9nBQ20ok8efOFN535UDWrmOMKcZTF/ZgpjKPCPFwaDFlpgtbWbUW+1ABMYv4g
GkWbTKmDiaAreK3qjVWf20UuzxzSFk/QS1dxTHoNv8NsvWAJp7AZv2WV6Bj+k9Is
sfwQ
-----END ENCRYPTED PRIVATE KEY-----";

/// 与 OPENSSH_ED25519_PLAIN 同钥的公钥单行形态(误粘场景)。
const ED25519_PUBLIC_ONELINER: &str = "\
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOSFEszbDGZjzbr8u+RB4xE1/2mm2WvIUxJ12GZOaFKv \
matrix-ed25519";

fn decode_error_of(text: &str, passphrase: Option<&str>) -> russh::keys::Error {
    decode_secret_key(text, passphrase).expect_err("sample must fail to decode")
}

fn assert_actionable(text: &str, error: &russh::keys::Error, provided: bool, guidance: &str) {
    let message = private_key_decode_failure(text, error, provided);
    assert!(
        message.starts_with("Failed to decode SSH private key: "),
        "{message}"
    );
    // 底层错误永远原样保留,不吞细节。
    assert!(
        message.contains(&format!("(decoder error: {error})")),
        "{message}"
    );
    assert!(message.contains(guidance), "{message}");
}

fn temp_transfer_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dbx-ssh-resume-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spool_fixture(dir: &Path, task_id: &str, remote_path: &str, size: u64, received: u64) {
    std::fs::write(
        dir.join(format!("upload-{task_id}.part")),
        vec![b'x'; received as usize],
    )
    .unwrap();
    write_upload_meta(
        &dir.join(format!("upload-{task_id}.json")),
        &json!({ "remotePath": remote_path, "size": size }),
    )
    .unwrap();
}

fn structured_error_emitter() -> PluginEmitter {
    let output: Arc<Mutex<Box<dyn std::io::Write + Send>>> =
        Arc::new(Mutex::new(Box::new(Vec::<u8>::new())));
    PluginEmitter::for_tests(output, dbx_plugin_sdk::PluginTransport::JsonLines)
}

struct TransferEventSink(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TransferEventSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn transfer_event_emitter() -> (PluginEmitter, Arc<Mutex<Vec<u8>>>) {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let emitter = PluginEmitter::for_tests(
        Arc::new(Mutex::new(Box::new(TransferEventSink(sink.clone())))),
        dbx_plugin_sdk::PluginTransport::JsonLines,
    );
    (emitter, sink)
}

fn emitted_events(sink: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
    String::from_utf8(sink.lock().unwrap().clone())
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// One framed transport message (the transfer tests need both JSON events and
/// binary download frames from the same emitter).
enum FramedMessage {
    Json(Value),
    Binary { channel: String, data: Vec<u8> },
}

fn framed_messages(sink: &Arc<Mutex<Vec<u8>>>) -> Vec<FramedMessage> {
    let buffer = sink.lock().unwrap().clone();
    let mut messages = Vec::new();
    let mut cursor = 0;
    while cursor + 5 <= buffer.len() {
        let kind = buffer[cursor];
        let length = u32::from_be_bytes(
            buffer[cursor + 1..cursor + 5]
                .try_into()
                .expect("frame length"),
        ) as usize;
        let start = cursor + 5;
        let end = start + length;
        if end > buffer.len() {
            break;
        }
        let payload = &buffer[start..end];
        match kind {
            0 => {
                if let Ok(value) = serde_json::from_slice(payload) {
                    messages.push(FramedMessage::Json(value));
                }
            }
            1 if payload.len() >= 2 => {
                let channel_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
                if payload.len() >= 2 + channel_len {
                    messages.push(FramedMessage::Binary {
                        channel: String::from_utf8_lossy(&payload[2..2 + channel_len]).into_owned(),
                        data: payload[2 + channel_len..].to_vec(),
                    });
                }
            }
            _ => {}
        }
        cursor = end;
    }
    messages
}

fn framed_transfer_emitter() -> (PluginEmitter, Arc<Mutex<Vec<u8>>>) {
    let sink = Arc::new(Mutex::new(Vec::new()));
    let emitter = PluginEmitter::for_tests(
        Arc::new(Mutex::new(Box::new(TransferEventSink(sink.clone())))),
        dbx_plugin_sdk::PluginTransport::Framed,
    );
    (emitter, sink)
}

/// Builds a registered tree task for the chunk-pump tests. `local_source_root`
/// makes the high-level lane open local files instead of an SFTP session.
fn tree_chunk_state(
    root: PathBuf,
    files: Vec<sftp_tree::TreeFile>,
    compression: CompressionMode,
    local_source_root: Option<PathBuf>,
) -> DownloadState {
    let size = files.iter().map(|file| file.size).sum();
    let mut tree = TreeDownloadState::new(
        root,
        files.len() as u64,
        0,
        false,
        TreeDownloadProgress {
            files: VecDeque::from(files),
            current: None,
            current_offset: 0,
            sink: None,
            files_done: 0,
            failures: Vec::new(),
            read_handle: None,
            handle_path: None,
        },
    );
    tree.local_source_root = local_source_root;
    DownloadState {
        session_id: "s-tree".to_string(),
        remote_path: "/remote".to_string(),
        file_name: "tree-root".to_string(),
        size,
        next_offset: 0,
        sink: None,
        emit_chunks: true,
        tree: Some(Arc::new(tree)),
        sudo_tmp: None,
        latin1: false,
        throttle: Throttle::new(0),
        progress_throttle: crate::features::transfer::progress::ProgressThrottle::default(),
        compression,
        ready: true,
        serving_plain: false,
        staged_plain: None,
        gz_staging: None,
        remote_temp: None,
        final_dir: None,
        overwrite: false,
        read_handle: Arc::new(AsyncMutex::new(None)),
        raw_client: Arc::new(AsyncMutex::new(None)),
    }
}

/// Drains a tree download with the framed emitter and returns every binary
/// frame's payload (offset stripped).
async fn drain_tree_frames(
    runtime: &SshRuntime,
    task_id: &str,
    emitter: &PluginEmitter,
    sink: &Arc<Mutex<Vec<u8>>>,
) -> Vec<u8> {
    let mut offset = runtime
        .downloads
        .lock()
        .unwrap()
        .get(task_id)
        .map(|state| state.next_offset)
        .unwrap_or(0);
    for _ in 0..32 {
        let response = runtime
            .download_chunk(task_id, offset, emitter)
            .await
            .expect("tree chunk");
        let length = response["length"].as_u64().expect("length");
        assert_eq!(response["offset"].as_u64(), Some(offset));
        offset += length;
        if response["eof"].as_bool().expect("eof") {
            break;
        }
    }
    let mut streamed = Vec::new();
    for message in framed_messages(sink) {
        if let FramedMessage::Binary { channel, data } = message {
            if channel == format!("sftp/download/{task_id}") {
                let frame_offset = u64::from_be_bytes(data[..8].try_into().expect("frame offset"));
                assert_eq!(frame_offset, streamed.len() as u64, "stale frame offset");
                streamed.extend_from_slice(&data[8..]);
            }
        }
    }
    streamed
}

/// A reader that parks its first `poll_read` on a oneshot gate, so a test can
/// hold a chunk inside the read and release it deterministically.
struct GatedTreeRead {
    started: Arc<tokio::sync::Notify>,
    release: Option<tokio::sync::oneshot::Receiver<()>>,
    announced: bool,
    data: Vec<u8>,
    pos: usize,
}

impl tokio::io::AsyncRead for GatedTreeRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.announced {
            self.announced = true;
            self.started.notify_one();
        }
        if let Some(release) = self.release.as_mut() {
            if std::future::Future::poll(std::pin::Pin::new(release), cx).is_pending() {
                return std::task::Poll::Pending;
            }
        }
        self.release = None;
        let remaining = self.data.len().saturating_sub(self.pos);
        let length = remaining.min(buf.remaining());
        buf.put_slice(&self.data[self.pos..self.pos + length]);
        self.pos += length;
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncSeek for GatedTreeRead {
    fn start_seek(
        mut self: std::pin::Pin<&mut Self>,
        position: std::io::SeekFrom,
    ) -> std::io::Result<()> {
        let target = match position {
            std::io::SeekFrom::Start(offset) => offset as usize,
            std::io::SeekFrom::Current(delta) => (self.pos as i64 + delta).max(0) as usize,
            std::io::SeekFrom::End(_) => self.data.len(),
        };
        self.pos = target.min(self.data.len());
        Ok(())
    }

    fn poll_complete(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<u64>> {
        std::task::Poll::Ready(Ok(self.pos as u64))
    }
}

/// A reader that blocks inside `poll_read` until its sleep elapses, announcing
/// when the read started. Used to prove cancel does not wait behind I/O.
struct SlowTreeRead {
    started: Arc<tokio::sync::Notify>,
    announced: bool,
    delay: std::pin::Pin<Box<tokio::time::Sleep>>,
    data: Vec<u8>,
    pos: usize,
}

impl tokio::io::AsyncRead for SlowTreeRead {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if !self.announced {
            // First poll: announce once, then park in the sleep.
            self.announced = true;
            self.started.notify_one();
        }
        match std::future::Future::poll(self.delay.as_mut(), cx) {
            std::task::Poll::Pending => return std::task::Poll::Pending,
            std::task::Poll::Ready(()) => {}
        }
        let remaining = self.data.len().saturating_sub(self.pos);
        let length = remaining.min(buf.remaining());
        buf.put_slice(&self.data[self.pos..self.pos + length]);
        self.pos += length;
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncSeek for SlowTreeRead {
    fn start_seek(
        mut self: std::pin::Pin<&mut Self>,
        position: std::io::SeekFrom,
    ) -> std::io::Result<()> {
        let target = match position {
            std::io::SeekFrom::Start(offset) => offset as usize,
            std::io::SeekFrom::Current(delta) => (self.pos as i64 + delta).max(0) as usize,
            std::io::SeekFrom::End(_) => self.data.len(),
        };
        self.pos = target.min(self.data.len());
        Ok(())
    }

    fn poll_complete(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<u64>> {
        std::task::Poll::Ready(Ok(self.pos as u64))
    }
}
