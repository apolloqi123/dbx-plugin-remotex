//! SFTP 压缩传输（gzip 混合方案）：决策、扩展名黑名单、比率守卫、gzip
//! 流式编解码与远端命令构造。
//!
//! 分工：压缩端用程序内置 flate2（纯 Rust），对端必须落回原样文件——解压/
//! 压缩动作由远端 `gzip`/`gunzip`/`tar` 工具承担（任务开始时 `command -v`
//! 探测）。任何一环不满足（偏好关闭、低于阈值、不可压缩类型、远端缺工具、
//! 只读连接、latin-1 裸包车道、压缩率不划算、弱 CPU）都静默回退普通传输
//! ——功能可降但不可死。latin-1 车道整条不参与：用户路径进 shell 有编码雷
//! 区。CPU 综合判断（M33 补充）：auto 策略下本机并行度 ≤2 直接跳过（决策
//! 层），上传压缩预压另有运行时吞吐守卫（实测压缩速率过低即中止回退，比
//! 核数更真实，对所有策略生效）。

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod local;
mod policy;
mod remote;

#[cfg(test)]
mod tests;

pub use local::*;
pub use policy::*;
pub use remote::*;
