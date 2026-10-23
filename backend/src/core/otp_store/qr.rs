//! Metrics support.

use super::*;

/// import-qr 图片字节上限：二维码本身极小，8 MiB 足够任何手机截图。
pub(crate) const MAX_QR_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// QR 解码像素边长/内存上限：压缩输入虽有 8 MiB 帽，但解压炸弹（如
/// 60000×60000 的 PNG）仍会撑爆内存——8192² / 64 MiB 远超任何二维码截图。
pub(crate) const MAX_QR_PIXEL_EDGE: u32 = 8192;

pub(crate) const MAX_QR_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

/// `otp/import-qr`：解码图片中的 `otpauth://` 二维码供表单预填（不入库）。
/// 解码链：base64 → `image` 解码 → luma8 → `rqrr` PreparedImage →
/// detect_grids → decode，取第一条能解出 otpauth URI 的结果。
pub fn decode_otpauth_qr(image_base64: &str) -> Result<OtpUriParams, String> {
    let payload = image_base64.trim();
    // 容忍 data URL 前缀（截图工具常带）。
    let payload = payload
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .map(|(_, encoded)| encoded)
        .unwrap_or(payload);
    let raw = BASE64_STANDARD
        .decode(payload)
        .map_err(|error| format!("imageBase64 is not valid base64: {error}"))?;
    if raw.len() > MAX_QR_IMAGE_BYTES {
        return Err(format!(
            "image exceeds {MAX_QR_IMAGE_BYTES} bytes; crop the QR code and retry"
        ));
    }
    let image = decode_with_limits(&raw)?.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(image);
    for grid in prepared.detect_grids() {
        let Ok((_, text)) = grid.decode() else {
            continue;
        };
        let text = text.trim();
        if text.starts_with("otpauth://") {
            return otp::parse_otpauth_uri(text);
        }
    }
    Err("no otpauth:// QR code found in the image".to_string())
}

/// `image` 解码 with 硬上限（[`MAX_QR_PIXEL_EDGE`] / [`MAX_QR_ALLOC_BYTES`]）：
/// `load_from_memory` 不设限，解压炸弹会直接吃满内存；`ImageReader::limits`
/// 在解码前按声明尺寸与分配预算拒绝（`decode` 先 reserve 再 set_limits）。
pub(crate) fn decode_with_limits(raw: &[u8]) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(raw));
    // `Limits` 是 non_exhaustive，只能在 default 之上逐字段覆盖。
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_QR_PIXEL_EDGE);
    limits.max_image_height = Some(MAX_QR_PIXEL_EDGE);
    limits.max_alloc = Some(MAX_QR_ALLOC_BYTES);
    reader.limits(limits);
    reader
        .with_guessed_format()
        .map_err(|error| format!("image format could not be guessed: {error}"))?
        .decode()
        .map_err(|error| format!("image could not be decoded: {error}"))
}
