use super::*;
use crate::core::time::unix_now_secs;
use crate::core::vault::KeyfileProvider;

const SECRET_PLAIN: &str = "GEZDGNBVGY3TQOJQ";

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dbx-otp-store-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn keyfile_vault(dir: &Path) -> Vault {
    Vault::new(&KeyfileProvider::new(vault::keyfile_path(dir)))
}

fn save(store: &mut OtpStore, params: Value, vault: &Vault) -> OtpEntry {
    save_entry(store, &params, vault).unwrap().0
}

fn totp_params(extra: Value) -> Value {
    let mut params = json!({
        "otpType": "totp",
        "issuer": "ACME",
        "username": "alice",
        "secret": SECRET_PLAIN,
    });
    if let (Some(base), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    params
}

// —— 存储 / 0600 / 损坏视为空 ————————————————————

#[test]
fn store_roundtrip_preserves_entries_and_bindings() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let entry = save(&mut store, totp_params(json!({})), &vault);
    bind(&mut store, "conn-1", &entry.id).unwrap();
    save_store(&dir, &store).unwrap();

    let loaded = load_store(&dir);
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].otp_type, "totp");
    assert_eq!(loaded.entries[0].algorithm, OtpAlgorithm::Sha1);
    assert_eq!(loaded.entries[0].digits, 6);
    assert_eq!(loaded.entries[0].period, 30);
    assert_eq!(
        loaded.bindings.get("conn-1").map(String::as_str),
        Some(entry.id.as_str())
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn store_file_is_0600_and_secret_never_plaintext_on_disk() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    save(&mut store, totp_params(json!({})), &vault);
    save_store(&dir, &store).unwrap();
    let text = std::fs::read_to_string(store_path(&dir)).unwrap();
    assert!(
        !text.contains(SECRET_PLAIN),
        "plaintext secret must not touch disk"
    );
    assert!(text.contains("secretEnc"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store_path(&dir))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn corrupted_file_falls_back_to_empty_store() {
    let dir = temp_dir();
    std::fs::write(store_path(&dir), "{not json").unwrap();
    assert!(load_store(&dir).entries.is_empty());
    assert!(load_store(&dir).bindings.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

// —— save（加密 / 空保留旧值 / 校验）/ list 脱敏 ————————————

#[test]
fn save_encrypts_new_secret_and_keeps_old_on_empty_update() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let entry = save(&mut store, totp_params(json!({})), &vault);
    assert!(!entry.secret_encrypted.is_empty());

    // 空 secret 更新：保留旧密文。
    let (updated, created) = save_entry(
        &mut store,
        &json!({ "id": entry.id, "username": "bob", "secret": "" }),
        &vault,
    )
    .unwrap();
    assert!(!created);
    assert_eq!(updated.username, "bob");
    assert_eq!(updated.secret_encrypted, entry.secret_encrypted);
    // 明文能解回原密钥。
    assert_eq!(
        get_decrypted_secret(&vault, &updated).unwrap(),
        otp::decode_base32(SECRET_PLAIN).unwrap()
    );

    // 新 secret 覆盖旧值（解密结果 = 新输入密钥的字节，且密文已更换）。
    let (rotated, _) = save_entry(
        &mut store,
        &json!({ "id": entry.id, "secret": "MFRGGZDFMZTWQ2LK" }),
        &vault,
    )
    .unwrap();
    assert_ne!(rotated.secret_encrypted, entry.secret_encrypted);
    assert_eq!(
        get_decrypted_secret(&vault, &rotated).unwrap(),
        otp::decode_base32("MFRGGZDFMZTWQ2LK").unwrap()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn degraded_vault_refuses_new_secret_without_mutating_store() {
    let degraded = Vault::new(&crate::core::vault::BrokenKeyProvider);
    let mut store = OtpStore::default();
    let err = save_entry(&mut store, &totp_params(json!({})), &degraded);
    let message = err.expect_err("degraded vault save must fail");
    assert!(message.contains("unavailable"), "error was: {message}");
    assert!(
        store.entries.is_empty(),
        "a refused save must not leave a half-saved entry"
    );
}

#[test]
fn save_rejects_bad_input_shapes() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    // 新建缺 secret。
    assert!(save_entry(&mut store, &json!({ "issuer": "x" }), &vault).is_err());
    // 非法 base32。
    assert!(save_entry(&mut store, &json!({ "secret": "!!" }), &vault).is_err());
    // otpType / digits / period / counter 校验。
    assert!(save_entry(
        &mut store,
        &json!({ "secret": SECRET_PLAIN, "otpType": "yub" }),
        &vault
    )
    .is_err());
    assert!(save_entry(
        &mut store,
        &json!({ "secret": SECRET_PLAIN, "digits": 5 }),
        &vault
    )
    .is_err());
    assert!(save_entry(
        &mut store,
        &json!({ "secret": SECRET_PLAIN, "period": 0 }),
        &vault
    )
    .is_err());
    assert!(save_entry(
        &mut store,
        &json!({ "secret": SECRET_PLAIN, "otpType": "hotp" }),
        &vault
    )
    .expect_err("hotp requires counter")
    .contains("counter"));
    // issuer 超长。
    assert!(save_entry(
        &mut store,
        &json!({ "secret": SECRET_PLAIN, "issuer": "i".repeat(MAX_ISSUER_LEN + 1) }),
        &vault
    )
    .is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_redact_secret_but_report_has_secret() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let entry = save(&mut store, totp_params(json!({})), &vault);
    let views = list_views(&store);
    assert_eq!(views.len(), 1);
    let view = &views[0];
    assert_eq!(view["hasSecret"], true);
    assert!(view.get("secret").is_none());
    assert!(view.get("secretEnc").is_none());
    assert_eq!(view["issuer"], "ACME");
    let serialized = serde_json::to_string(&views).unwrap();
    assert!(!serialized.contains(SECRET_PLAIN));
    assert_eq!(binding_views(&store), json!({}));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = entry;
}

#[test]
fn hotp_entry_roundtrips_counter() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let entry = save(
        &mut store,
        totp_params(
            json!({ "otpType": "hotp", "counter": 5, "algorithm": "SHA512", "digits": 8, "period": 60 }),
        ),
        &vault,
    );
    assert_eq!(entry.otp_type, "hotp");
    assert_eq!(entry.counter, Some(5));
    assert_eq!(entry.algorithm, OtpAlgorithm::Sha512);
    assert_eq!(entry.digits, 8);
    save_store(&dir, &store).unwrap();
    let loaded = load_store(&dir);
    assert_eq!(loaded.entries[0].counter, Some(5));
    // totp 条目带 counter 输入也会被丢弃。
    let totp = save(&mut store, totp_params(json!({ "counter": 9 })), &vault);
    assert_eq!(totp.counter, None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn entry_cap_blocks_new_entries_only() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    for index in 0..MAX_ENTRIES {
        save(
            &mut store,
            totp_params(json!({ "issuer": format!("p{index}") })),
            &vault,
        );
    }
    assert!(save_entry(
        &mut store,
        &totp_params(json!({ "issuer": "overflow" })),
        &vault
    )
    .is_err());
    let id = store.entries[0].id.clone();
    assert!(save_entry(
        &mut store,
        &json!({ "id": id, "issuer": "edit-ok" }),
        &vault
    )
    .is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

// —— bind / unbind / delete ————————————————————

#[test]
fn bind_unbind_and_delete_cascade() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let entry = save(&mut store, totp_params(json!({})), &vault);
    assert!(bind(&mut store, "conn-1", "missing").is_err());
    bind(&mut store, "conn-1", &entry.id).unwrap();
    assert_eq!(
        binding_views(&store).get("conn-1").and_then(Value::as_str),
        Some(entry.id.as_str())
    );
    // 改绑。
    let second = save(&mut store, totp_params(json!({ "issuer": "B" })), &vault);
    bind(&mut store, "conn-1", &second.id).unwrap();
    assert_eq!(store.bindings["conn-1"], second.id);
    // 解绑。
    assert!(unbind(&mut store, "conn-1"));
    assert!(!unbind(&mut store, "conn-1"));
    // 删除条目级联清绑定。
    bind(&mut store, "conn-2", &entry.id).unwrap();
    assert!(delete_entry(&mut store, &entry.id));
    assert!(!delete_entry(&mut store, &entry.id));
    assert!(!store.bindings.values().any(|id| id == &entry.id));
    let _ = std::fs::remove_dir_all(&dir);
}

// —— 绑定解析 + 共享防重放缓存 ————————————————————

#[test]
fn bound_totp_resolves_binding_and_skips_hotp_or_missing() {
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let totp = save(&mut store, totp_params(json!({})), &vault);
    let hotp = save(
        &mut store,
        totp_params(json!({ "otpType": "hotp", "counter": 1 })),
        &vault,
    );

    bind(&mut store, "conn-a", &totp.id).unwrap();
    save_store(&dir, &store).unwrap();
    let bound = bound_totp_in(&dir, "conn-a").unwrap();
    assert_eq!(bound.entry_id, totp.id);
    assert_eq!(bound.key, otp::decode_base32(SECRET_PLAIN).unwrap());

    // hotp 绑定不参与 TOTP 自动应答。
    bind(&mut store, "conn-b", &hotp.id).unwrap();
    save_store(&dir, &store).unwrap();
    assert!(bound_totp_in(&dir, "conn-b").is_none());
    assert!(bound_totp_in(&dir, "conn-ghost").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn take_window_code_blocks_replay_within_step_and_rotates_next_step() {
    // 不做跨用例 reset:WINDOW_CODES 是进程级共享缓存,cargo test
    // 并行跑用例时,reset 会把并发用例刚标记的窗口一并清掉,令其
    // 防重放断言偶发失败。本用例的 entry/key 为本用例独有,进程内无别的
    // 用例会写同键,清缓存没有必要。
    let key = b"12345678901234567890";
    let first = take_window_code("e1", key, OtpAlgorithm::Sha1, 6, 30, 100).unwrap();
    assert_eq!(first.code.len(), 6);
    assert_eq!(first.remaining_secs, 30 - 100 % 30);
    // 同窗口第二次取码被拒，并给出等待秒数。
    let used = take_window_code("e1", key, OtpAlgorithm::Sha1, 6, 30, 100).unwrap_err();
    assert_eq!(used.remaining_secs, first.remaining_secs);
    // 同窗口稍后仍拒。
    assert!(take_window_code("e1", key, OtpAlgorithm::Sha1, 6, 30, 110).is_err());
    // 下一窗口放行，且码刷新（remaining = period - now % period = 20）。
    let second = take_window_code("e1", key, OtpAlgorithm::Sha1, 6, 30, 130).unwrap();
    assert_eq!(second.remaining_secs, 20);
    // 不同条目互不影响。
    assert!(take_window_code("e2", key, OtpAlgorithm::Sha1, 6, 30, 100).is_ok());
}

#[test]
fn take_connection_totp_key_marks_window_via_shared_cache() {
    // 同上:不做跨用例 reset。entry.id 每次运行新生成,进程内
    // 键必然独占;reset 反而可能清掉并发用例的标记。
    let dir = temp_dir();
    let vault = keyfile_vault(&dir);
    let mut store = OtpStore::default();
    let entry = save(&mut store, totp_params(json!({})), &vault);
    bind(&mut store, "conn-x", &entry.id).unwrap();
    save_store(&dir, &store).unwrap();

    let bound = take_connection_totp_key(&dir, "conn-x").expect("first take succeeds");
    assert_eq!(bound.entry_id, entry.id);
    // 自动应答取过之后，同窗口 otp/generate 路径必须被挡下。
    assert!(take_window_code(
        &entry.id,
        &bound.key,
        bound.algorithm,
        bound.digits,
        bound.period,
        unix_now_secs()
    )
    .is_err());
    // 未绑定连接拿不到。
    assert!(take_connection_totp_key(&dir, "conn-y").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn global_data_dir_roundtrip() {
    // 注入/读取（初始化函数幂等，测试里允许反复覆盖）。
    let dir = temp_dir();
    init_data_dir(&dir);
    assert_eq!(data_dir().as_deref(), Some(dir.as_path()));
    let _ = std::fs::remove_dir_all(&dir);
}

// —— QR 导入 ————————————————————

/// 离线生成的测试向量（Python segno，version 6、EC-M、scale 4、border 2，
/// 197x197 PNG）。内容：
/// `otpauth://totp/ACME:alice%40acme.com?secret=GEZDGNBVGY3TQOJQ&issuer=ACME&digits=8&period=60`
const QR_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAALQAAAC0AQAAAAAVtjufAAABp0lEQVR42tVYy41CQQyLtgH336U7yNrOA2klLiv5wgjBYw75OI4TmP14OF9zPzPQ5yxBgIQv+DOfz3/vZ7F6yQV0lvrii6b9gUMmnI/ffFG2L4PKwfA8F+34GQeOXU7K9mP1yut6t/E3Xf6cMn9SVj4cFWNT5yr+g3SCuDlMQthq/GZNcJEfEZXt/pJpNdaIN1QOpItd5afbVg/mj1wogyo+soiX5blCd/FR9G5eIxQv6OqPC0rH71IcRlvtL0Z41r5gF9wu/m5YY2Pz/r5d/BWz3USDDFAZ/zdIoiiffKr8ZwTz2jbRT7W+FgaPZxGHR9ByfU86M4AZhbOTpr4xEzgZnHxW9RNROHMyuTAjuGg/6hMBskDYX1efD5MNczbcmbb+BB2r3F4+Zf2x2ddukhyq/Jzw0sPrPcFQxkcAwQ97EoHu/hBWesCcQLT1+ZBJF5/EbZk/z0Y7NyDZ7t/QEq/Nzd089f2T4A0a4uhU3p+R7fCxXub/8/siDPKAvJlctn8qxBvwW7efAXbT3frT7i8v0FGKjAJMWd/mejaLtMWiz//v/X/gF/rESY0Iu1K8AAAAAElFTkSuQmCC";

#[test]
fn decode_otpauth_qr_parses_generated_code() {
    let params = decode_otpauth_qr(QR_PNG_BASE64).unwrap();
    assert_eq!(params.otp_type, "totp");
    assert_eq!(params.issuer, "ACME");
    assert_eq!(params.secret_base32, "GEZDGNBVGY3TQOJQ");
    assert_eq!(params.digits, 8);
    assert_eq!(params.period, 60);
    assert_eq!(params.counter, None);
}

#[test]
fn decode_otpauth_qr_accepts_data_url_prefix() {
    let data_url = format!("data:image/png;base64,{QR_PNG_BASE64}");
    let params = decode_otpauth_qr(&data_url).unwrap();
    assert_eq!(params.issuer, "ACME");
}

#[test]
fn decode_otpauth_qr_rejects_garbage_and_non_qr_images() {
    assert!(decode_otpauth_qr("not base64!!").is_err());
    assert!(decode_otpauth_qr("").is_err());
    // 合法 PNG 字节但没有二维码（1x1 透明像素）。
    let tiny_png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    assert!(decode_otpauth_qr(tiny_png).is_err());
}

/// 解压炸弹防线：压缩后极小的 PNG 也可以声明 8193px 宽，超过
/// [`MAX_QR_PIXEL_EDGE`] 时必须在解码前被 limits 拒绝。
#[test]
fn decode_otpauth_qr_rejects_oversized_images() {
    let oversized = image::DynamicImage::new_luma8(MAX_QR_PIXEL_EDGE + 1, 1);
    let mut buf = std::io::Cursor::new(Vec::new());
    oversized
        .write_to(&mut buf, image::ImageFormat::Png)
        .expect("encode oversized png");
    let encoded = BASE64_STANDARD.encode(buf.get_ref());
    let error = decode_otpauth_qr(&encoded).unwrap_err();
    assert!(
        error.starts_with("image could not be decoded"),
        "oversized image must be refused by the decode limits, got: {error}"
    );
}
