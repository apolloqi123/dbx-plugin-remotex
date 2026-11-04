//! Windows file clipboard via CF_HDROP (native Explorer copy/paste).
//!
//! Reading and writing the drop list are plain Win32 clipboard calls; the
//! process does not need a window or a message loop because both directions
//! use rendered (global-memory) data. `GetClipboardSequenceNumber` is the cheap
//! change counter the RDP worker polls.
//!
//! Only absolute paths are accepted in both directions. Unit tests never touch
//! the developer's clipboard (`cfg!(test)` guards), matching the macOS backend.

use std::path::PathBuf;
use std::ptr;
use std::time::Duration;
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Shell::DragQueryFileW;

const CF_HDROP: u32 = 15;
const DROPEFFECT_COPY: u32 = 1;
const DROP_EFFECT_FORMAT: &str = "Preferred DropEffect";

fn open_clipboard() -> bool {
    for attempt in 0..10 {
        // SAFETY: a null owner is valid; user32 only associates the calling
        // thread with the clipboard for the duration of the open.
        if unsafe { OpenClipboard(ptr::null_mut()) } != 0 {
            return true;
        }
        if attempt < 9 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    false
}

pub(crate) fn supported() -> bool {
    true
}

pub(crate) fn change_count() -> i64 {
    // SAFETY: no arguments, no handles retained.
    unsafe { GetClipboardSequenceNumber() as i64 }
}

pub(crate) fn has_file_urls() -> bool {
    if cfg!(test) {
        return false;
    }
    if !open_clipboard() {
        return false;
    }
    // SAFETY: the clipboard is open on this thread until CloseClipboard.
    let present = unsafe { IsClipboardFormatAvailable(CF_HDROP) } != 0;
    unsafe { CloseClipboard() };
    present
}

pub(crate) fn file_urls() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    if !open_clipboard() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    // SAFETY: the handle stays valid while the clipboard is open; the buffer is
    // sized from the character count DragQueryFileW reports.
    unsafe {
        let drop = GetClipboardData(CF_HDROP);
        if !drop.is_null() {
            let count = DragQueryFileW(drop, u32::MAX, ptr::null_mut(), 0);
            for index in 0..count {
                let length = DragQueryFileW(drop, index, ptr::null_mut(), 0);
                if length == 0 {
                    continue;
                }
                let mut buffer = vec![0u16; length as usize + 1];
                let written = DragQueryFileW(drop, index, buffer.as_mut_ptr(), length + 1);
                if written == 0 {
                    continue;
                }
                let path = PathBuf::from(String::from_utf16_lossy(&buffer[..written as usize]));
                if path.is_absolute() {
                    paths.push(path);
                }
            }
        }
        CloseClipboard();
    }
    paths
}

/// Writes a CF_HDROP list plus `Preferred DropEffect = copy`, so Explorer (and
/// Office) paste instead of moving the files.
pub(crate) fn write_file_urls(paths: &[PathBuf]) -> Result<(), String> {
    if cfg!(test) {
        return Ok(());
    }
    let absolute: Vec<PathBuf> = paths
        .iter()
        .filter(|path| path.is_absolute())
        .cloned()
        .collect();
    if absolute.is_empty() {
        return Err("No absolute paths to place on the clipboard".to_string());
    }
    let payload = super::uri_list::build_dropfiles_payload(&absolute);
    if !open_clipboard() {
        return Err("The Windows clipboard is busy".to_string());
    }
    // SAFETY: empty + set are paired on the same open clipboard; after a
    // successful SetClipboardData the system owns the global memory and we must
    // not free it. Every failure path frees or closes what it allocated.
    unsafe {
        if EmptyClipboard() == 0 {
            CloseClipboard();
            return Err("Unable to empty the Windows clipboard".to_string());
        }
        let global = GlobalAlloc(GMEM_MOVEABLE, payload.len());
        if global.is_null() {
            CloseClipboard();
            return Err("Unable to allocate clipboard memory".to_string());
        }
        let pointer = GlobalLock(global) as *mut u8;
        if pointer.is_null() {
            GlobalFree(global);
            CloseClipboard();
            return Err("Unable to lock clipboard memory".to_string());
        }
        ptr::copy_nonoverlapping(payload.as_ptr(), pointer, payload.len());
        GlobalUnlock(global);
        if SetClipboardData(CF_HDROP, global).is_null() {
            GlobalFree(global);
            CloseClipboard();
            return Err("Writing the Windows clipboard was rejected".to_string());
        }
        write_copy_drop_effect();
        CloseClipboard();
    }
    Ok(())
}

/// Best-effort `Preferred DropEffect`; losing it only changes copy to the
/// Explorer default (still pasteable), so failures are ignored.
unsafe fn write_copy_drop_effect() {
    let name: Vec<u16> = DROP_EFFECT_FORMAT
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let format = RegisterClipboardFormatW(name.as_ptr());
    if format == 0 {
        return;
    }
    let memory = GlobalAlloc(GMEM_MOVEABLE, 4);
    if memory.is_null() {
        return;
    }
    let pointer = GlobalLock(memory) as *mut u8;
    if pointer.is_null() {
        GlobalFree(memory);
        return;
    }
    ptr::copy_nonoverlapping(DROPEFFECT_COPY.to_le_bytes().as_ptr(), pointer, 4);
    GlobalUnlock(memory);
    if SetClipboardData(format, memory).is_null() {
        GlobalFree(memory);
    }
}
