//! SFTP 文件名编码偏好（M14-B）：偏好串 ↔ 编码枚举的纯映射。
//!
//! 只承载偏好层的白名单语义；显示解码与传输路径的分离仍由
//! `filesystem/sftp/name.rs` 负责（该模块 re-export 本类型，调用点不变）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameEncoding {
    /// UTF-8 优先，无效字节按 latin-1 重试（默认）。
    Auto,
    /// 固定按 latin-1 解码（服务器端文件系统编码确知非 UTF-8 时选用）。
    Latin1,
}

impl NameEncoding {
    /// 偏好串 → 编码（白名单外返回 None，由 preferences 层拒绝/回退）。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "latin-1" => Some(Self::Latin1),
            _ => None,
        }
    }

    /// 序列化回偏好串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Latin1 => "latin-1",
        }
    }
}
