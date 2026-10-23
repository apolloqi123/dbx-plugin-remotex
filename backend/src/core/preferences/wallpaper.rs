//! Plugin preferences layer.

use super::*;

/// 背景图（P2-9）落盘位置：`<plugin_data_dir>/wallpaper`（无扩展名，格式由
/// 魔数判定），与 preferences.json 同层。
pub(crate) const WALLPAPER_FILE_NAME: &str = "wallpaper";

/// 背景图上限 8 MiB。
pub(crate) const WALLPAPER_MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn wallpaper_path(data_dir: &Path) -> PathBuf {
    data_dir.join(WALLPAPER_FILE_NAME)
}

/// png / jpeg / webp 魔数识别；其余一律拒绝（不信任扩展名）。
pub(crate) fn detect_image_format(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("jpeg");
    }
    if bytes.len() >= 12 && bytes[0..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        return Some("webp");
    }
    None
}

/// 读取落盘背景并打包成 data URL；文件缺失或魔数非法（手工替换/损坏）一律
/// 返回空对象，前端按无背景处理。
pub fn load_wallpaper(data_dir: &Path) -> Value {
    let bytes = match std::fs::read(wallpaper_path(data_dir)) {
        Ok(bytes) => bytes,
        Err(_) => return json!({}),
    };
    match detect_image_format(&bytes) {
        Some(format) => json!({
            "dataUrl": format!("data:image/{format};base64,{}", BASE64_STANDARD.encode(bytes)),
        }),
        None => json!({}),
    }
}

/// 保存背景图：校验 ≤8 MiB 且为 png/jpeg/webp 魔数后原子落盘（tmp + rename），
/// 返回与 get 相同的 data URL 载荷。
pub fn save_wallpaper(data_dir: &Path, params: &Value) -> Result<Value, String> {
    let data_base64 = params
        .get("imageBase64")
        .and_then(Value::as_str)
        .ok_or_else(|| "imageBase64 must be a base64 string".to_string())?;
    // 粗判：base64 每 3 字节占 4 字符，超限先拒，避免无谓解码大 payload。
    if data_base64.len() > WALLPAPER_MAX_BYTES / 3 * 4 + 4 {
        return Err(format!(
            "imageBase64 exceeds the wallpaper limit of {} MiB",
            WALLPAPER_MAX_BYTES / 1024 / 1024
        ));
    }
    let bytes = BASE64_STANDARD
        .decode(data_base64.trim())
        .map_err(|error| format!("imageBase64 is not valid base64: {error}"))?;
    if bytes.is_empty() || bytes.len() > WALLPAPER_MAX_BYTES {
        return Err(format!(
            "imageBase64 exceeds the wallpaper limit of {} MiB",
            WALLPAPER_MAX_BYTES / 1024 / 1024
        ));
    }
    detect_image_format(&bytes)
        .ok_or_else(|| "imageBase64 must be a png, jpeg or webp image".to_string())?;
    let path = wallpaper_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("wallpaper.tmp");
    std::fs::write(&tmp, &bytes)
        .map_err(|error| format!("Failed to write {}: {error}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|error| format!("Failed to write {}: {error}", path.display()))?;
    Ok(load_wallpaper(data_dir))
}

/// 清除背景图：文件不存在视为成功（幂等）。
pub fn clear_wallpaper(data_dir: &Path) -> Value {
    let _ = std::fs::remove_file(wallpaper_path(data_dir));
    json!({})
}
