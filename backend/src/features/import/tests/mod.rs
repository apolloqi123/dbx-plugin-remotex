use super::*;

mod apply;
mod ssh_config;
mod stream;

/// Builds an in-memory ZIP archive with the given entries.
fn xshell_zip(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    buf.into_inner()
}

const WEB_XSH: &str = concat!(
    "[CONNECTION]\n",
    "Protocol=SSH\n",
    "Host=192.168.1.20\n",
    "Port=2222\n",
    "\n",
    "[CONNECTION:AUTHENTICATION]\n",
    "UserName=deploy\n",
    "UserKey=my-key\n",
    "\n",
    "[SCRIPT]\n",
    "something=else\n",
);

const FINGERPRINT_HEX: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// Test-only helper: encrypts like WindTerm does (AES-256-CBC/PKCS7,
/// key/IV derived via PBKDF2-HMAC-SHA3-512), so the parser round-trips
/// against an independent encryption path.
fn windterm_encrypt(master: &str, fingerprint_hex: &str, plaintext: &str) -> String {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeEncrypt, KeyIvInit};
    type Aes256CbcEncryptor = cbc::Encryptor<aes::Aes256>;
    let salt = fingerprint_bytes(fingerprint_hex);
    let (key, iv) = derive_windterm_key(master, &salt);
    let ciphertext = Aes256CbcEncryptor::new_from_slices(&key, &iv)
        .unwrap()
        .encrypt_padded_vec::<Pkcs7>(plaintext.as_bytes());
    BASE64.encode(ciphertext)
}

fn user_config_json(master_password: bool) -> String {
    json!({
        "application": {
            "fingerprint": FINGERPRINT_HEX,
            "masterPassword": master_password,
        }
    })
    .to_string()
}

const SECURECRT_EXPORT: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<VanDyke version=\"9.0\">\n",
    "\t<key name=\"Default\">\n",
    "\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t<dword name=\"[SSH2] Port\">00000016</dword>\n",
    "\t</key>\n",
    "\t<key name=\"Sessions\">\n",
    "\t\t<key name=\"Prod\">\n",
    "\t\t\t<key name=\"web01\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t\t<string name=\"Hostname\">192.168.1.10</string>\n",
    "\t\t\t\t<dword name=\"[SSH2] Port\">000008ae</dword>\n",
    "\t\t\t\t<string name=\"Username\">deploy</string>\n",
    "\t\t\t\t<string name=\"Password V2\">uH9kJ2vQ EncryptedBlob</string>\n",
    "\t\t\t</key>\n",
    "\t\t\t<key name=\"legacy\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t\t<string name=\"Hostname\">10.0.0.5</string>\n",
    "\t\t\t\t<dword name=\"Port\">34</dword>\n",
    "\t\t\t</key>\n",
    "\t\t\t<key name=\"telnet\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">Telnet</string>\n",
    "\t\t\t\t<string name=\"Hostname\">10.0.0.6</string>\n",
    "\t\t\t</key>\n",
    "\t\t\t<key name=\"nohost\">\n",
    "\t\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t</key>\n",
    "\t\t</key>\n",
    "\t\t<key name=\"root-level\">\n",
    "\t\t\t<string name=\"Protocol Name\">SSH2</string>\n",
    "\t\t\t<string name=\"Hostname\">10.0.0.9</string>\n",
    "\t\t</key>\n",
    "\t</key>\n",
    "</VanDyke>\n",
);

fn finalshell_config_json(name: &str, host: &str, extra: &str) -> String {
    format!(
        "{{\"name\":\"{name}\",\"host\":\"{host}\",\"port\":2222,\"user_name\":\"deploy\",\
         \"parent_id\":\"f2\",\"conection_type\":100,\"description\":\"primary\"{extra}}}"
    )
}

const ELECTERM_BOOKMARK: &str = concat!(
    "{",
    "\"id\":\"bm1\",\"title\":\"web\",\"host\":\"10.2.0.1\",\"port\":2222,",
    "\"username\":\"deploy\",\"auth_type\":\"password\",\"type\":\"ssh\"",
    "}"
);

const TERMIUS_EXPORT: &str = concat!(
    "{",
    "\"data\":{",
    "\"hosts\":[",
    "{\"id\":\"h1\",\"label\":\"web\",\"address\":\"10.3.0.1\",",
    "\"group\":{\"id\":\"g2\"},\"ssh_config\":{\"id\":\"c1\"}},",
    "{\"id\":\"h2\",\"address\":\"10.3.0.2\"}",
    "],",
    "\"groups\":[",
    "{\"id\":\"g1\",\"label\":\"Prod\",\"parent\":null},",
    "{\"id\":\"g2\",\"label\":\"Web\",\"parent\":{\"id\":\"g1\"}}",
    "],",
    "\"ssh_configs\":[{\"id\":\"c1\",\"port\":2222,\"identity\":{\"id\":\"i1\"}}],",
    "\"identities\":[",
    "{\"id\":\"i1\",\"username\":\"deploy\",\"password\":\"plain-pw\"},",
    "{\"id\":\"i2\",\"username\":\"ops\",\"password\":\"BAAAAAAA0123456789012345678901234567890123456789\"}",
    "],",
    "\"keys\":[",
    "{\"id\":\"k1\",\"label\":\"key1\",\"privateKey\":\"-----BEGIN OPENSSH PRIVATE KEY-----\\nabc\\n-----END OPENSSH PRIVATE KEY-----\",\"passphrase\":\"key-phr\"}",
    "]",
    "}",
    "}",
);

fn sample_sessions() -> Vec<ImportedSession> {
    vec![
        ImportedSession {
            name: "web".to_string(),
            host: "10.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            group_path: vec!["Prod".to_string()],
            description: String::new(),
            auth: ImportedAuth::Password { has_secret: true },
            secret_note: String::new(),
        },
        ImportedSession {
            name: "keyed".to_string(),
            host: "10.0.0.2".to_string(),
            port: 2222,
            username: "ops".to_string(),
            group_path: Vec::new(),
            description: "d".to_string(),
            auth: ImportedAuth::PrivateKey {
                path: Some("/home/u/key".to_string()),
                has_secret: true,
            },
            secret_note: String::new(),
        },
    ]
}
