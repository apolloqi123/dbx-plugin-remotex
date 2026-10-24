use super::*;

/// RFC 4226 附录 D：SHA1、ASCII 密钥 "12345678901234567890"、6 位。
const RFC4226_SECRET: &[u8] = b"12345678901234567890";
const RFC4226_CODES_6: [u32; 10] = [
    755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
];
/// RFC 6238 附录 B：8 位、period 30，`(时刻, SHA1, SHA256, SHA512)`。
const RFC6238_VECTORS: [(u64, &str, &str, &str); 6] = [
    (59, "94287082", "46119246", "90693936"),
    (1111111109, "07081804", "68084774", "25091201"),
    (1111111111, "14050471", "67062674", "99943326"),
    (1234567890, "89005924", "91819424", "93441116"),
    (2000000000, "69279037", "90698825", "38618901"),
    (20000000000, "65353130", "77737706", "47863826"),
];

// —— HOTP（RFC 4226 附录 D 官方向量）———————————————

#[test]
fn hotp_matches_rfc4226_appendix_d() {
    for (counter, expected) in RFC4226_CODES_6.iter().enumerate() {
        assert_eq!(
            hotp(OtpAlgorithm::Sha1, RFC4226_SECRET, counter as u64, 6),
            *expected,
            "counter {counter}"
        );
    }
}

#[test]
fn hotp_dynamic_truncation_is_counter_sensitive_and_digit_scaled() {
    // 相邻 counter 不碰撞（附录 D 十个向量互不相同已覆盖）；这里补
    // digits 位数语义：8 位 = 前 8 位十进制（补零语义在 totp_at 验证）。
    assert_eq!(hotp(OtpAlgorithm::Sha1, RFC4226_SECRET, 0, 8), 84_755_224);
}

// —— TOTP（RFC 6238 SHA1/SHA256/SHA512 官方向量）———————

#[test]
fn totp_at_matches_rfc6238_vectors_for_all_algorithms() {
    let secrets = [
        (OtpAlgorithm::Sha1, &b"12345678901234567890"[..]),
        (
            OtpAlgorithm::Sha256,
            &b"12345678901234567890123456789012"[..],
        ),
        (
            OtpAlgorithm::Sha512,
            &b"1234567890123456789012345678901234567890123456789012345678901234"[..],
        ),
    ];
    for (algorithm, secret) in secrets {
        for (unix_secs, sha1, sha256, sha512) in RFC6238_VECTORS {
            let expected = match algorithm {
                OtpAlgorithm::Sha1 => sha1,
                OtpAlgorithm::Sha256 => sha256,
                OtpAlgorithm::Sha512 => sha512,
            };
            let (code, remaining) = totp_at(algorithm, secret, 30, unix_secs, 8);
            assert_eq!(code, expected, "{algorithm:?} at {unix_secs}");
            assert_eq!(remaining, 30 - unix_secs % 30);
        }
    }
}

#[test]
fn totp_at_pads_left_and_reports_remaining_seconds() {
    // 剩余秒数 = period - unix_secs % period；码左补零到 digits 位。
    let (code, remaining) = totp_at(OtpAlgorithm::Sha1, RFC4226_SECRET, 30, 60, 6);
    assert_eq!(remaining, 30);
    assert_eq!(code.len(), 6);
    // 人为夹一个会前导补零的组合（counter 0、5 位裁切不必真实——只验
    // 形态）：digits=8 时长度恒为 8。
    let (code, _) = totp_at(OtpAlgorithm::Sha1, RFC4226_SECRET, 30, 59, 8);
    assert_eq!(code, "94287082");
}

#[test]
fn totp_at_zero_period_degrades_to_one_second_step() {
    // 解析层拒绝 period 0；这里验证不 panic（防御性 max(1)：步长 1s，
    // 任何时刻剩余都是 1）。
    let (code, remaining) = totp_at(OtpAlgorithm::Sha1, RFC4226_SECRET, 0, 7, 6);
    assert_eq!(remaining, 1);
    assert_eq!(code.len(), 6);
}

// —— otpauth URI 解析 / 生成往返 ————————————————————

#[test]
fn parse_totp_uri_with_all_parameters() {
    let params = parse_otpauth_uri(
        "otpauth://totp/ACME%3Aalice%40acme.com?secret=gezd%20gnbvgy3tqojq&issuer=ACME&algorithm=SHA256&digits=8&period=60",
    )
    .unwrap();
    assert_eq!(params.otp_type, "totp");
    assert_eq!(params.issuer, "ACME");
    assert_eq!(params.label, "ACME:alice@acme.com");
    assert_eq!(params.secret_base32, "GEZDGNBVGY3TQOJQ");
    assert_eq!(params.algorithm, OtpAlgorithm::Sha256);
    assert_eq!(params.digits, 8);
    assert_eq!(params.period, 60);
    assert_eq!(params.counter, None);
}

#[test]
fn parse_issuer_falls_back_to_label_prefix() {
    let params =
        parse_otpauth_uri("otpauth://totp/Example:alice@example.com?secret=JBSWY3DPEHPK3PXP")
            .unwrap();
    assert_eq!(params.issuer, "Example");
    assert_eq!(params.algorithm, OtpAlgorithm::Sha1, "缺省 SHA1");
    assert_eq!(params.digits, 6);
    assert_eq!(params.period, 30);
}

#[test]
fn parse_hotp_uri_requires_counter() {
    let error = parse_otpauth_uri("otpauth://hotp/counter?secret=JBSWY3DPEHPK3PXP")
        .expect_err("hotp without counter must be rejected");
    assert!(error.contains("counter"), "{error}");
    let params =
        parse_otpauth_uri("otpauth://hotp/counter?secret=JBSWY3DPEHPK3PXP&counter=42").unwrap();
    assert_eq!(params.counter, Some(42));
    assert_eq!(params.otp_type, "hotp");
}

#[test]
fn parse_uri_rejects_wrong_prefix_or_missing_secret() {
    for bad in [
        "https://example.com",
        "otpauth://move/label?secret=JBSWY3DPEHPK3PXP",
        "otpauth://totp/",
        "otpauth://totp/label",
        "otpauth://totp/label?digits=5",
        "otpauth://totp/label?period=0",
        "otpauth://totp/label?counter=x&",
    ] {
        // digits/period/counter 非法都要报错；前缀/secret 缺失也要报错。
        assert!(parse_otpauth_uri(bad).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn parse_uri_tolerates_base32_spaces_and_case() {
    let params = parse_otpauth_uri("otpauth://totp/x?secret=jbsw y3dp ehpk 3pxp").unwrap();
    assert_eq!(params.secret_base32, "JBSWY3DPEHPK3PXP");
}

#[test]
fn uri_round_trip_preserves_parameters() {
    let params = OtpUriParams {
        otp_type: "totp".to_string(),
        issuer: "ACME Corp".to_string(),
        label: "ACME Corp:alice@example.com".to_string(),
        secret_base32: "JBSWY3DPEHPK3PXP".to_string(),
        algorithm: OtpAlgorithm::Sha512,
        digits: 8,
        period: 45,
        counter: None,
    };
    let rebuilt = parse_otpauth_uri(&to_otpauth_uri(&params)).unwrap();
    assert_eq!(rebuilt, params);

    let hotp = OtpUriParams {
        otp_type: "hotp".to_string(),
        counter: Some(7),
        ..params
    };
    let rebuilt = parse_otpauth_uri(&to_otpauth_uri(&hotp)).unwrap();
    assert_eq!(rebuilt, hotp);
}

#[test]
fn algorithm_parse_defaults_to_sha1() {
    assert_eq!(OtpAlgorithm::parse("sha1"), OtpAlgorithm::Sha1);
    assert_eq!(OtpAlgorithm::parse("Sha256 "), OtpAlgorithm::Sha256);
    assert_eq!(OtpAlgorithm::parse("SHA512"), OtpAlgorithm::Sha512);
    assert_eq!(OtpAlgorithm::parse(""), OtpAlgorithm::Sha1);
    assert_eq!(OtpAlgorithm::parse("md5"), OtpAlgorithm::Sha1);
    assert_eq!(OtpAlgorithm::Sha1.as_str(), "SHA1");
    assert_eq!(OtpAlgorithm::Sha256.as_str(), "SHA256");
    assert_eq!(OtpAlgorithm::Sha512.as_str(), "SHA512");
}

#[test]
fn base32_decode_is_tolerant_and_validating() {
    // "GEZDGNBVGY3TQOJQ" 是 ASCII "1234567890" 的 base32（RFC 6238 测试密钥
    // 前 10 字节），大小写与空格/连字符形态都应解出同一结果。
    assert_eq!(decode_base32("GEZDGNBVGY3TQOJQ").unwrap(), b"1234567890");
    assert_eq!(decode_base32("gezd gnbv-gy3t qojq").unwrap(), b"1234567890");
    assert!(decode_base32("").is_err());
    assert!(decode_base32("!!").is_err());
}

#[test]
fn percent_codec_round_trips_ordinary_text() {
    let original = "ACME Corp:alice@example.com/path+x";
    assert_eq!(percent_decode(&percent_encode(original)), original);
    // 宽松解码：非法序列原样保留。
    assert_eq!(percent_decode("100%"), "100%");
    assert_eq!(percent_decode("a%2Fb"), "a/b");
}
