use super::*;

use std::path::{Path, PathBuf};

/// Serialises the tests that redirect HOME so they cannot race each
/// other or the read-only HOME accesses elsewhere in the crate.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restores the home environment variable on drop, even on panic.
struct HomeEnvGuard {
    previous: Option<std::ffi::OsString>,
}

impl HomeEnvGuard {
    fn set(home: &Path) -> Self {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let previous = std::env::var_os(var);
        std::env::set_var(var, home);
        Self { previous }
    }
}

impl Drop for HomeEnvGuard {
    fn drop(&mut self) {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        match self.previous.take() {
            Some(value) => std::env::set_var(var, value),
            None => std::env::remove_var(var),
        }
    }
}

#[test]
fn read_private_key_file_validates_before_returning_content() {
    let dir = tempfile::tempdir().unwrap();
    // 不依赖 ssh-keygen 的拒绝路径：纯文本、公钥、超大文件、目录。
    let text_file = dir.path().join("notes.txt");
    std::fs::write(&text_file, "hello").unwrap();
    assert!(read_private_key_file(&text_file).is_err());
    let pub_file = dir.path().join("id_ed25519.pub");
    std::fs::write(
        &pub_file,
        "-----BEGIN PUBLIC KEY-----\nxx\n-----END PUBLIC KEY-----",
    )
    .unwrap();
    assert!(read_private_key_file(&pub_file).is_err());
    let big = dir.path().join("big.pem");
    std::fs::write(&big, vec![b'x'; (IMPORT_MAX_BYTES + 1) as usize]).unwrap();
    assert!(read_private_key_file(&big).is_err());
    assert!(read_private_key_file(dir.path()).is_err());
    // 真实密钥（ssh-keygen 可用时）：未加密与加密的都应读回原文。
    let plain = dir.path().join("id_ed25519");
    if generate_key(&plain, "") {
        let content = read_private_key_file(&plain).unwrap();
        assert!(content.contains("BEGIN OPENSSH PRIVATE KEY"));
        let locked = dir.path().join("locked.pem");
        assert!(generate_key(&locked, "phrase"));
        assert!(read_private_key_file(&locked).is_ok());
    }
}

/// Generates an ed25519 key with ssh-keygen, returning false when the
/// tool is unavailable so callers can fall back to reduced assertions.
fn generate_key(path: &Path, passphrase: &str) -> bool {
    std::process::Command::new("ssh-keygen")
        .arg("-t")
        .arg("ed25519")
        .arg("-N")
        .arg(passphrase)
        .arg("-C")
        .arg("keys-rs-test")
        .arg("-f")
        .arg(path)
        .arg("-q")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[test]
fn discover_reports_generated_keys() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let ssh_dir = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();

    if !generate_key(&ssh_dir.join("id_ed25519"), "") {
        // No ssh-keygen on this machine: verify the tolerant paths only.
        assert!(discover_in(&ssh_dir).is_empty());
        std::fs::write(ssh_dir.join("junk.pem"), "not a key at all\n").unwrap();
        std::fs::write(
            ssh_dir.join("id_junk"),
            "-----BEGIN CERTIFICATE-----\nnope\n-----END CERTIFICATE-----\n",
        )
        .unwrap();
        assert!(discover_in(&ssh_dir).is_empty());
        return;
    }

    assert!(generate_key(&ssh_dir.join("encrypted.pem"), "secret123"));
    assert!(generate_key(&ssh_dir.join("relative_key"), ""));
    assert!(generate_key(&ssh_dir.join("dotslash_key"), ""));
    let absolute = home.path().join("outside_key.pem");
    assert!(generate_key(&absolute, ""));
    std::fs::write(ssh_dir.join("junk.pem"), "not a key at all\n").unwrap();
    std::fs::write(
        ssh_dir.join("id_junk"),
        "-----BEGIN CERTIFICATE-----\nnope\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    std::fs::write(
        ssh_dir.join("config"),
        format!(
            "Host example\n  HostName example.test\n  IdentityFile relative_key\n\
             identityfile = {}\n  IdentityFile \"dotslash_key\"\n\
             # IdentityFile commented_out\n  User someone\n",
            absolute.display()
        ),
    )
    .unwrap();

    let _guard_env = HomeEnvGuard::set(home.path());
    let keys = discover().unwrap();

    let paths: Vec<&str> = keys.iter().map(|key| key.path.as_str()).collect();
    let expected = [
        ssh_dir.join("id_ed25519"),
        ssh_dir.join("encrypted.pem"),
        ssh_dir.join("relative_key"),
        ssh_dir.join("dotslash_key"),
        absolute.clone(),
    ];
    for path in &expected {
        assert!(
            paths.contains(&path.display().to_string().as_str()),
            "discovery missed {}",
            path.display()
        );
    }
    assert!(!paths.iter().any(|path| path.ends_with("junk.pem")));
    assert!(!paths.iter().any(|path| path.ends_with("id_junk")));
    assert_eq!(paths.len(), expected.len());

    let plain = keys
        .iter()
        .find(|key| key.path.ends_with("id_ed25519"))
        .unwrap();
    assert_eq!(plain.algorithm, "ssh-ed25519");
    assert!(plain.fingerprint.starts_with("SHA256:"));
    assert!(!plain.has_passphrase);

    let encrypted = keys
        .iter()
        .find(|key| key.path.ends_with("encrypted.pem"))
        .unwrap();
    assert!(encrypted.has_passphrase);
    // The OpenSSH format keeps the public half readable, so metadata is
    // still recoverable for passphrase-protected keys.
    assert_eq!(encrypted.algorithm, "ssh-ed25519");
    assert!(encrypted.fingerprint.starts_with("SHA256:"));

    // The private key material must never leave the files.
    let serialized = serde_json::to_string(&keys).unwrap();
    assert!(!serialized.contains("PRIVATE KEY"));
    let secret_body = std::fs::read_to_string(ssh_dir.join("id_ed25519"))
        .unwrap()
        .lines()
        .nth(1)
        .unwrap()
        .to_string();
    assert!(!serialized.contains(&secret_body));
}

#[test]
fn discover_tolerates_missing_home_ssh() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let _guard_env = HomeEnvGuard::set(home.path());
    assert!(discover().unwrap().is_empty());

    let ssh_dir = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    std::fs::write(ssh_dir.join("config"), "IdentityFile nothing_here\n").unwrap();
    assert!(discover().unwrap().is_empty());
}

/// `discover_options` renders one dropdown entry per discovered key whose
/// label is the key path itself (no metadata, so the text stays short);
/// key material never travels in the options.
#[test]
fn discover_options_shape_only_metadata() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let ssh_dir = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();

    if !generate_key(&ssh_dir.join("id_ed25519"), "") {
        // No ssh-keygen on this machine: verify the tolerant empty shape.
        let _guard_env = HomeEnvGuard::set(home.path());
        let options = discover_options().unwrap();
        assert_eq!(options["options"].as_array().unwrap().len(), 0);
        return;
    }
    assert!(generate_key(&ssh_dir.join("locked.pem"), "phrase"));

    let _guard_env = HomeEnvGuard::set(home.path());
    let options = discover_options().unwrap()["options"]
        .as_array()
        .unwrap()
        .clone();
    let plain = options
        .iter()
        .find(|option| option["value"].as_str().unwrap().ends_with("id_ed25519"))
        .expect("plain key listed");
    assert_eq!(
        plain["label"], plain["value"],
        "the option label is the key path itself"
    );
    assert!(!plain["label"].as_str().unwrap().contains("SHA256:"));
    let encrypted = options
        .iter()
        .find(|option| option["value"].as_str().unwrap().ends_with("locked.pem"))
        .expect("encrypted key listed");
    assert_eq!(encrypted["label"], encrypted["value"]);

    let serialized = serde_json::to_string(&options).unwrap();
    assert!(!serialized.contains("PRIVATE KEY"));
}

#[cfg(unix)]
#[test]
fn discover_skips_unreadable_files() {
    let _guard = ENV_LOCK.lock().unwrap();
    let home = tempfile::tempdir().unwrap();
    let ssh_dir = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh_dir).unwrap();
    if !generate_key(&ssh_dir.join("id_ed25519"), "") {
        assert!(discover_in(&ssh_dir).is_empty());
        return;
    }
    let unreadable = ssh_dir.join("locked.pem");
    std::fs::write(&unreadable, "-----BEGIN OPENSSH PRIVATE KEY-----\nzz\n").unwrap();
    std::fs::set_permissions(
        &unreadable,
        std::os::unix::fs::PermissionsExt::from_mode(0o000),
    )
    .unwrap();

    let _guard_env = HomeEnvGuard::set(home.path());
    let keys = discover().unwrap();
    assert_eq!(keys.len(), 1);
    assert!(keys[0].path.ends_with("id_ed25519"));
}

#[cfg(unix)]
#[test]
fn identity_file_directives_resolve_paths() {
    let ssh_dir = Path::new("/home/tester/.ssh");
    let config = "\
# global config
IdentityFile relative_key
identityfile = /absolute/from/equals.pem
IdentityFile \"quoted key\"
IdentityFile ~/dotslash_key
HostName example.test
IdentityFile
";
    let paths = identity_files_from_config(config, ssh_dir);
    assert_eq!(
        paths,
        vec![
            ssh_dir.join("relative_key"),
            PathBuf::from("/absolute/from/equals.pem"),
            ssh_dir.join("quoted key"),
            PathBuf::from("/home/tester/dotslash_key"),
        ]
    );
}

#[cfg(windows)]
#[test]
fn identity_file_directives_resolve_windows_and_unicode_paths() {
    let home = PathBuf::from(r"C:\Users\测试");
    let ssh_dir = home.join(".ssh");
    let config = "\
IdentityFile C:\\Keys\\生产\\id_ed25519\r\n\
IdentityFile \\\\server\\share\\运维.key\r\n\
IdentityFile ~\\.ssh\\id_fallback\r\n";
    let paths = identity_files_from_config(config, &ssh_dir);

    assert_eq!(
        paths,
        vec![
            PathBuf::from(r"C:\Keys\生产\id_ed25519"),
            PathBuf::from(r"\\server\share\运维.key"),
            home.join(".ssh").join("id_fallback"),
        ]
    );
}

#[test]
fn known_hosts_list_and_remove_roundtrip() {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path();

    // Missing store behaves like an empty one.
    assert_eq!(remove_known_host(data_dir, "example.test", 22).unwrap(), 0);
    let empty = list_known_hosts(data_dir).unwrap();
    assert_eq!(empty["entries"].as_array().unwrap().len(), 0);

    let key_line = if generate_key(&directory.path().join("probe_key"), "") {
        std::fs::read_to_string(directory.path().join("probe_key.pub"))
            .unwrap()
            .trim()
            .to_string()
    } else {
        // Bogus base64 still lists; the fingerprint just stays empty.
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1234567890abcdefghijklmnopqrstuvwxyz test"
            .to_string()
    };
    std::fs::write(
        data_dir.join("known_hosts"),
        format!(
            "# comment\nexample.test {key_line}\n[example.test]:2222 {key_line}\nother.test {key_line}\n"
        ),
    )
    .unwrap();

    let listed = list_known_hosts(data_dir).unwrap();
    let entries = listed["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["host"], "example.test");
    assert_eq!(entries[0]["port"], 22);
    assert_eq!(entries[1]["host"], "example.test");
    assert_eq!(entries[1]["port"], 2222);
    assert_eq!(entries[2]["host"], "other.test");
    assert_eq!(entries[0]["keyType"], "ssh-ed25519");
    for entry in entries {
        let fingerprint = entry["fingerprint"].as_str().unwrap();
        assert!(fingerprint.is_empty() || fingerprint.starts_with("SHA256:"));
        // Plain entries always carry a null marker so the wire shape is
        // stable for the frontend.
        assert!(entry["marker"].is_null());
    }

    assert_eq!(remove_known_host(data_dir, "example.test", 22).unwrap(), 1);
    assert_eq!(
        remove_known_host(data_dir, "example.test", 2222).unwrap(),
        1
    );
    let remaining = list_known_hosts(data_dir).unwrap();
    let entries = remaining["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["host"], "other.test");
    assert_eq!(remove_known_host(data_dir, "example.test", 22).unwrap(), 0);
}

#[test]
fn host_field_splitting() {
    assert_eq!(
        split_host_field("example.test"),
        ("example.test".to_string(), 22)
    );
    assert_eq!(
        split_host_field("[example.test]:2222"),
        ("example.test".to_string(), 2222)
    );
    assert_eq!(
        split_host_field("[broken]:notaport"),
        ("[broken]:notaport".to_string(), 22)
    );
}
