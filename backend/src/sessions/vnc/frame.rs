//! VNC session support.

use super::*;

/// Framebuffer guard — deliberately tighter than upstream's 8192px/8.3MP
/// decoder limits (patch payloads stay < 64 MiB by construction).
pub(crate) const MAX_FRAMEBUFFER_WIDTH: u16 = 3840;

pub(crate) const MAX_FRAMEBUFFER_HEIGHT: u16 = 2160;

pub(crate) const BYTES_PER_PIXEL: usize = 4;

// —— 44-byte patch frame protocol (aligned with NyaTerm encode_frame_patch;
// documented in docs/architecture/PROTOCOL.zh-CN.md § VNC 帧补丁) ——

/// Wire format: `44-byte header | payload`, every field little-endian.
/// Documented in docs/architecture/PROTOCOL.zh-CN.md § VNC 帧补丁; the cross-side golden
/// vector is pinned in `tests::patch_frame_golden_vector_matches_frontend`
/// and mirrored in frontend/src/features/remote-desktop/lib/vncFrame.spec.ts (same hex).
pub(crate) const FRAME_HEADER_BYTES: usize = 44;

/// Wire values shared with the frontend decoder and NyaTerm's renderer.
pub(crate) const PIXEL_FORMAT_RGBA8888: u32 = 2;

/// One framebuffer update ready for the `vnc/frame/{sessionId}` binary
/// channel. Validation mirrors NyaTerm's `encode_frame_patch` exactly.
pub(crate) struct FramePatch<'a> {
    pub sequence: u64,
    pub desktop_width: u32,
    pub desktop_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub payload: &'a [u8],
}

/// Encodes `44-byte header | payload` with every network-supplied field
/// re-validated (the framebuffer itself is bounded, but each rect arrives
/// from the wire and must be checked again). Pure — unit-tested.
pub(crate) fn encode_frame_patch(patch: &FramePatch<'_>) -> Result<Vec<u8>, String> {
    if patch.desktop_width == 0 || patch.desktop_height == 0 {
        return Err("VNC framebuffer dimensions must be non-zero".to_string());
    }
    if patch.width == 0 || patch.height == 0 {
        return Err("VNC frame rectangle must be non-zero".to_string());
    }
    let right = patch
        .x
        .checked_add(patch.width)
        .ok_or_else(|| "VNC frame horizontal bounds overflow".to_string())?;
    let bottom = patch
        .y
        .checked_add(patch.height)
        .ok_or_else(|| "VNC frame vertical bounds overflow".to_string())?;
    if right > patch.desktop_width || bottom > patch.desktop_height {
        return Err("VNC frame rectangle exceeds framebuffer bounds".to_string());
    }
    let row_bytes = usize::try_from(patch.width)
        .ok()
        .and_then(|width| width.checked_mul(BYTES_PER_PIXEL))
        .ok_or_else(|| "VNC frame row size overflows".to_string())?;
    let stride =
        usize::try_from(patch.stride).map_err(|_| "VNC frame stride is invalid".to_string())?;
    if stride < row_bytes {
        return Err("VNC frame stride is too small".to_string());
    }
    let required_payload = usize::try_from(patch.height)
        .ok()
        .and_then(|height| stride.checked_mul(height))
        .ok_or_else(|| "VNC frame payload size overflows".to_string())?;
    if patch.payload.len() < required_payload {
        return Err("VNC frame payload is too small".to_string());
    }
    let payload_len = u32::try_from(patch.payload.len())
        .map_err(|_| "VNC frame payload exceeds the wire limit".to_string())?;

    let mut frame = Vec::with_capacity(FRAME_HEADER_BYTES + patch.payload.len());
    frame.extend_from_slice(&patch.sequence.to_le_bytes());
    frame.extend_from_slice(&patch.desktop_width.to_le_bytes());
    frame.extend_from_slice(&patch.desktop_height.to_le_bytes());
    frame.extend_from_slice(&patch.x.to_le_bytes());
    frame.extend_from_slice(&patch.y.to_le_bytes());
    frame.extend_from_slice(&patch.width.to_le_bytes());
    frame.extend_from_slice(&patch.height.to_le_bytes());
    frame.extend_from_slice(&patch.stride.to_le_bytes());
    frame.extend_from_slice(&PIXEL_FORMAT_RGBA8888.to_le_bytes());
    frame.extend_from_slice(&payload_len.to_le_bytes());
    frame.extend_from_slice(patch.payload);
    Ok(frame)
}

/// Server-side copy of the desktop, used to (a) composite incoming rects and
/// (b) repaint reconnecting viewers with one full-frame patch.
pub(crate) struct VncFramebuffer {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}

impl VncFramebuffer {
    pub(crate) fn matches_size(&self, width: u16, height: u16) -> bool {
        self.width == width && self.height == height
    }

    pub(crate) fn new(width: u16, height: u16) -> Result<Self, String> {
        if width == 0
            || height == 0
            || width > MAX_FRAMEBUFFER_WIDTH
            || height > MAX_FRAMEBUFFER_HEIGHT
        {
            return Err(format!(
                "VNC framebuffer {width}x{height} is outside the supported range (max {MAX_FRAMEBUFFER_WIDTH}x{MAX_FRAMEBUFFER_HEIGHT})"
            ));
        }
        let len = usize::from(width)
            .checked_mul(usize::from(height))
            .and_then(|pixels| pixels.checked_mul(BYTES_PER_PIXEL))
            .ok_or_else(|| "VNC framebuffer size overflows".to_string())?;
        Ok(Self {
            width,
            height,
            rgba: vec![0; len],
        })
    }

    /// Composites one RGBA rect into the framebuffer. `pixels` must be
    /// exactly `width*4*height` bytes (tight packing, stride == width*4).
    pub(crate) fn apply_rgba(&mut self, rect: Rect, pixels: &[u8]) -> Result<(), String> {
        let right = u32::from(rect.x) + u32::from(rect.width);
        let bottom = u32::from(rect.y) + u32::from(rect.height);
        if rect.width == 0
            || rect.height == 0
            || right > u32::from(self.width)
            || bottom > u32::from(self.height)
        {
            return Err("VNC rectangle exceeds framebuffer bounds".to_string());
        }
        let row_bytes = usize::from(rect.width)
            .checked_mul(BYTES_PER_PIXEL)
            .ok_or_else(|| "VNC rectangle row size overflows".to_string())?;
        let required = row_bytes
            .checked_mul(usize::from(rect.height))
            .ok_or_else(|| "VNC rectangle payload size overflows".to_string())?;
        if pixels.len() != required {
            return Err("VNC rectangle payload length is invalid".to_string());
        }
        let framebuffer_stride = usize::from(self.width) * BYTES_PER_PIXEL;
        for row in 0..usize::from(rect.height) {
            let src_start = row * row_bytes;
            let dst_start = (usize::from(rect.y) + row) * framebuffer_stride
                + usize::from(rect.x) * BYTES_PER_PIXEL;
            self.rgba[dst_start..dst_start + row_bytes]
                .copy_from_slice(&pixels[src_start..src_start + row_bytes]);
        }
        Ok(())
    }

    /// CopyRect: copy the source rect into the destination rect and return the
    /// destination RGBA payload for the incremental patch. Overlap is handled
    /// through a temporary buffer, and both rects are re-validated against the
    /// framebuffer (server-supplied coordinates).
    pub(crate) fn copy_rect(&mut self, dst: Rect, src: Rect) -> Result<Vec<u8>, String> {
        if dst.width != src.width || dst.height != src.height {
            return Err("VNC CopyRect source and destination sizes differ".to_string());
        }
        for rect in [dst, src] {
            let right = u32::from(rect.x) + u32::from(rect.width);
            let bottom = u32::from(rect.y) + u32::from(rect.height);
            if rect.width == 0
                || rect.height == 0
                || right > u32::from(self.width)
                || bottom > u32::from(self.height)
            {
                return Err("VNC CopyRect exceeds framebuffer bounds".to_string());
            }
        }
        let row_bytes = usize::from(dst.width) * BYTES_PER_PIXEL;
        let framebuffer_stride = usize::from(self.width) * BYTES_PER_PIXEL;
        let height = usize::from(dst.height);
        let mut temp = vec![0u8; row_bytes * height];
        for row in 0..height {
            let start = (usize::from(src.y) + row) * framebuffer_stride
                + usize::from(src.x) * BYTES_PER_PIXEL;
            temp[row * row_bytes..(row + 1) * row_bytes]
                .copy_from_slice(&self.rgba[start..start + row_bytes]);
        }
        for row in 0..height {
            let start = (usize::from(dst.y) + row) * framebuffer_stride
                + usize::from(dst.x) * BYTES_PER_PIXEL;
            self.rgba[start..start + row_bytes]
                .copy_from_slice(&temp[row * row_bytes..(row + 1) * row_bytes]);
        }
        Ok(temp)
    }

    pub(crate) fn patch_bytes(
        &self,
        sequence: u64,
        rect: Rect,
        pixels: &[u8],
    ) -> Result<Vec<u8>, String> {
        encode_frame_patch(&FramePatch {
            sequence,
            desktop_width: u32::from(self.width),
            desktop_height: u32::from(self.height),
            x: u32::from(rect.x),
            y: u32::from(rect.y),
            width: u32::from(rect.width),
            height: u32::from(rect.height),
            stride: u32::from(rect.width) * 4,
            payload: pixels,
        })
    }

    pub(crate) fn full_frame_bytes(&self, sequence: u64) -> Result<Vec<u8>, String> {
        self.patch_bytes(
            sequence,
            Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            &self.rgba,
        )
    }
}
