//! Serial upload engine layer.

use super::*;

// —— ZMODEM 编解码原语 ——————————————————————————————————————————

/// 帧编码字节（`ZDLE` 前导之后的那个字符）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ZEncoding {
    Zbin,
    Zhex,
    Zbin32,
}

impl ZEncoding {
    pub(crate) fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            b'A' => Some(Self::Zbin),
            b'B' => Some(Self::Zhex),
            b'C' => Some(Self::Zbin32),
            _ => None,
        }
    }

    /// 头部负载（帧类型 + 4 标志）之后的校验字节数。
    pub(crate) fn crc_len(self) -> usize {
        match self {
            Self::Zbin32 => 4,
            _ => 2,
        }
    }
}

/// 对端帧头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ZHeader {
    pub(crate) frame: u8,
    pub(crate) flags: [u8; 4],
}

impl ZHeader {
    pub(crate) fn count(&self) -> u32 {
        u32::from_le_bytes(self.flags)
    }
}

/// 帧头逐字节扫描器的状态。
#[derive(Debug)]
pub(crate) enum HeaderScan {
    /// 扫描 ZPAD…ZDLE 前导（`**\x18` 或 `*\x18`）。
    SeekingZpad { pads: usize },
    /// 已见 ZPAD+ZDLE，等编码字节。
    WantEncoding,
    /// 读帧负载：帧类型 + 4 标志 + CRC（逐字节反转义）。
    WantData {
        encoding: ZEncoding,
        payload: Vec<u8>,
        crc: Vec<u8>,
        escape_pending: bool,
    },
    /// hex 帧已完整报出，吃掉一个收尾字节（`\r`）。
    HexCr,
}

impl Default for HeaderScan {
    fn default() -> Self {
        Self::SeekingZpad { pads: 0 }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScanEvent {
    Header(ZHeader),
    /// 编码非法或 CRC 校验失败：丢弃当前帧重新扫描。
    Corrupt,
}

/// 对端帧头逐字节扫描器。跨 feed 的部分帧在扫描器里缓存。
#[derive(Debug, Default)]
pub(crate) struct HeaderScanner {
    scan: HeaderScan,
    /// hex 字节对的高半位。
    hex_hi: Option<u8>,
}

impl HeaderScanner {
    pub(crate) fn feed_byte(&mut self, byte: u8) -> Option<ScanEvent> {
        // 先在局部把状态迁移算完，避免在 &mut self.scan 借用中再借 self。
        let mut corrupt = false;
        let mut event = None;
        match &mut self.scan {
            HeaderScan::SeekingZpad { pads } => match byte {
                ZPAD => *pads += 1,
                ZDLE if *pads > 0 => {
                    *pads = 0;
                    self.scan = HeaderScan::WantEncoding;
                }
                _ => *pads = 0,
            },
            HeaderScan::WantEncoding => match ZEncoding::from_byte(byte) {
                Some(encoding) => {
                    self.scan = HeaderScan::WantData {
                        encoding,
                        payload: Vec::with_capacity(5),
                        crc: Vec::new(),
                        escape_pending: false,
                    };
                }
                None => {
                    self.scan = HeaderScan::default();
                    corrupt = true;
                }
            },
            HeaderScan::WantData {
                encoding,
                payload,
                crc,
                escape_pending,
            } => {
                let unescaped = if *escape_pending {
                    *escape_pending = false;
                    Some(unescape_zdle(byte))
                } else if byte == ZDLE {
                    *escape_pending = true;
                    None
                } else {
                    Some(byte)
                };
                if let Some(value) = unescaped {
                    if encoding == &ZEncoding::Zhex {
                        let Some(hi) = self.hex_hi.take() else {
                            self.hex_hi = Some(value);
                            return None;
                        };
                        let (Some(h), Some(l)) = (decode_hex(hi), decode_hex(value)) else {
                            self.scan = HeaderScan::default();
                            return Some(ScanEvent::Corrupt);
                        };
                        push_header_byte(payload, crc, (h << 4) | l);
                    } else {
                        push_header_byte(payload, crc, value);
                    }
                    let expected_crc = encoding.crc_len();
                    if crc.len() == expected_crc {
                        let (next, result) = complete_header(*encoding, payload, crc);
                        self.scan = next;
                        event = result;
                    }
                }
            }
            HeaderScan::HexCr => self.scan = HeaderScan::default(),
        }
        if corrupt {
            return Some(ScanEvent::Corrupt);
        }
        event
    }
}

/// 向 hex/binary 帧负载缓冲压入一个解码字节（前 5 字节是负载，其后是 CRC）。
pub(crate) fn push_header_byte(payload: &mut Vec<u8>, crc: &mut Vec<u8>, value: u8) {
    if payload.len() + crc.len() < 5 {
        payload.push(value);
    } else {
        crc.push(value);
    }
}

/// 负载齐了以后校验 CRC 并产出帧头；同时给出扫描器的下一个状态。
pub(crate) fn complete_header(
    encoding: ZEncoding,
    payload: &[u8],
    crc: &[u8],
) -> (HeaderScan, Option<ScanEvent>) {
    let next = if encoding == ZEncoding::Zhex {
        HeaderScan::HexCr
    } else {
        HeaderScan::default()
    };
    if payload.len() != 5 || crc.len() != encoding.crc_len() {
        return (HeaderScan::default(), Some(ScanEvent::Corrupt));
    }
    let ok = match encoding {
        ZEncoding::Zbin32 => {
            crc32_iso_hdlc(payload) == u32::from_le_bytes([crc[0], crc[1], crc[2], crc[3]])
        }
        _ => crc16_xmodem(payload) == u16::from_be_bytes([crc[0], crc[1]]),
    };
    if !ok {
        return (HeaderScan::default(), Some(ScanEvent::Corrupt));
    }
    (
        next,
        Some(ScanEvent::Header(ZHeader {
            frame: payload[0],
            flags: [payload[1], payload[2], payload[3], payload[4]],
        })),
    )
}

pub(crate) fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// ZDLE 反转义（ZDLE 之后那个字节）：'l'→DEL、'm'→0xFF，其余按
/// "bit6 置位则异或 0x40" 规则还原。
pub(crate) fn unescape_zdle(byte: u8) -> u8 {
    match byte {
        b'l' => 0x7f,
        b'm' => 0xff,
        b if (b & 0x60) == 0x40 => b ^ 0x40,
        b => b,
    }
}

/// ZDLE 转义单字节（与 zmodem2 的 ZDLE_TABLE 行为一致）：CR/XON/XOFF 相邻
/// 流控字节与其高位孪生按 ^0x40 转义，DEL/0xFF 用 'l'/'m'，其余原样。
pub(crate) fn escaped_zdle(byte: u8) -> Option<u8> {
    match byte {
        0x0d | 0x10 | 0x11 | 0x13 | 0x18 => Some(byte ^ 0x40),
        0x8d => Some(0xcd),
        0x90 => Some(0xd0),
        0x91 => Some(0xd1),
        0x93 => Some(0xd3),
        0x7f => Some(b'l'),
        0xff => Some(b'm'),
        _ => None,
    }
}

pub(crate) fn push_escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for &byte in bytes {
        if let Some(escaped) = escaped_zdle(byte) {
            out.push(ZDLE);
            out.push(escaped);
        } else {
            out.push(byte);
        }
    }
}

/// 十六进制帧头（ZRQINIT/ZFIN/ZNAK 用）：`**\x18B` + hex(帧+标志+CRC16) +
/// `\r\n`（ZFIN/ZACK 之外再补 XON，见 zmodem2 write_header_end_hex）。
pub(crate) fn hex_header(frame: u8, flags: [u8; 4]) -> Vec<u8> {
    let mut payload = [0u8; 5];
    payload[0] = frame;
    payload[1..].copy_from_slice(&flags);
    let crc = crc16_xmodem(&payload).to_be_bytes();
    let mut out = vec![ZPAD, ZPAD, ZDLE, b'B'];
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in payload.into_iter().chain(crc) {
        out.push(HEX[usize::from(byte >> 4)]);
        out.push(HEX[usize::from(byte & 0x0f)]);
    }
    out.push(b'\r');
    out.push(b'\n');
    if frame != FRAME_ZACK && frame != FRAME_ZFIN {
        out.push(XON);
    }
    out
}

/// 二进制 CRC32 帧头：`*\x18C` + 转义(帧+标志+CRC32 LE)。
pub(crate) fn bin32_header(frame: u8, flags: [u8; 4]) -> Vec<u8> {
    let mut payload = [0u8; 5];
    payload[0] = frame;
    payload[1..].copy_from_slice(&flags);
    let crc = crc32_iso_hdlc(&payload).to_le_bytes();
    let mut out = vec![ZPAD, ZDLE, b'C'];
    push_escaped(&mut out, &payload);
    push_escaped(&mut out, &crc);
    out
}

/// ZCRCW 数据子包：转义(数据) + ZDLE + 0x6b + 转义(CRC32(数据+类型) LE)。
pub(crate) fn zcrcw_subpacket(data: &[u8]) -> Vec<u8> {
    let mut crc_input = Vec::with_capacity(data.len() + 1);
    crc_input.extend_from_slice(data);
    crc_input.push(SUBPACKET_ZCRCW);
    let crc = crc32_iso_hdlc(&crc_input).to_le_bytes();
    let mut out = Vec::with_capacity(data.len() * 2 + 12);
    push_escaped(&mut out, data);
    out.push(ZDLE);
    out.push(SUBPACKET_ZCRCW);
    push_escaped(&mut out, &crc);
    out
}

/// ZMODEM 取消序列（NyaTerm cancel_sequence）：ZDLE×5 + 退格×5。
pub(crate) fn zmodem_cancel_sequence() -> Vec<u8> {
    let mut seq = vec![ZDLE; Z_CANCEL_COUNT];
    seq.extend([0x08; Z_CANCEL_COUNT]);
    seq
}
