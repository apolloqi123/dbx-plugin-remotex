//! 原生拖拽指针探针：只读取光标屏幕坐标与宿主窗口几何，不读窗口内容。
//!
//! DBX 把 wry 的 macOS 拖拽位置（已是逻辑点）又除了一次
//! `devicePixelRatio`，再减掉 iframe 在窗口内的偏移，于是插件收到的坐标
//! 同时被缩小并左移；单靠这个坐标无法反解。把同一时刻的真实光标与宿主
//! 转发的点配对，就能解出 iframe 偏移，之后按宿主点线性换算即可。
//!
//! 只在 macOS 生效；其它平台返回空样本，前端按原坐标处理（Windows/Linux
//! 的宿主换算本来就是物理像素，无需反解）。

// 非 macOS 构建不会调用窗口选择逻辑（sample 直接返回空样本），测试仍在跑。
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.x <= self.x + self.width
            && point.y >= self.y
            && point.y <= self.y + self.height
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowInfo {
    pub pid: i64,
    pub layer: i64,
    pub bounds: Rect,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PointerSample {
    pub platform: &'static str,
    pub cursor: Option<Point>,
    pub window: Option<Rect>,
}

/// 光标下最前面的普通窗口（layer 0）。窗口列表按前到后排列：若最前面的
/// 命中窗口不属于宿主进程，说明拖拽其实悬在别的应用上，返回 None 而不是
/// 拿宿主窗口做一次错误校准。过小的浮层（阴影/提示条）直接跳过。
pub(crate) fn pick_window(cursor: Point, windows: &[WindowInfo], owner_pid: i64) -> Option<Rect> {
    let hit = windows.iter().find(|window| {
        window.layer == 0
            && window.bounds.width >= 300.0
            && window.bounds.height >= 200.0
            && window.bounds.contains(cursor)
    })?;
    (hit.pid == owner_pid).then_some(hit.bounds)
}

pub(crate) fn sample() -> PointerSample {
    let platform = std::env::consts::OS;
    #[cfg(target_os = "macos")]
    {
        let cursor = macos::cursor();
        let window = cursor.and_then(|point| {
            let windows = macos::windows();
            pick_window(point, &windows, macos::owner_pid())
        });
        PointerSample {
            platform,
            cursor,
            window,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        PointerSample {
            platform,
            cursor: None,
            window: None,
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{Point, Rect, WindowInfo};
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *mut c_void;
        static kCGWindowOwnerPID: *const c_void;
        static kCGWindowLayer: *const c_void;
        static kCGWindowBounds: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(value: *const c_void);
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
        fn CFNumberGetValue(number: *const c_void, number_type: i64, value: *mut c_void) -> bool;
        fn CGRectMakeWithDictionaryRepresentation(
            dictionary: *const c_void,
            rect: *mut CGRect,
        ) -> bool;
    }

    const LIST_ON_SCREEN_ONLY: u32 = 1;
    const LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const CF_NUMBER_DOUBLE: i64 = 13;

    pub(super) fn cursor() -> Option<Point> {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let point = CGEventGetLocation(event);
            CFRelease(event.cast());
            (point.x.is_finite() && point.y.is_finite()).then_some(Point {
                x: point.x,
                y: point.y,
            })
        }
    }

    fn number(dictionary: *const c_void, key: *const c_void) -> Option<f64> {
        let value = unsafe { CFDictionaryGetValue(dictionary, key) };
        if value.is_null() {
            return None;
        }
        let mut out = 0.0_f64;
        let ok = unsafe {
            CFNumberGetValue(
                value.cast(),
                CF_NUMBER_DOUBLE,
                (&mut out as *mut f64).cast::<c_void>(),
            )
        };
        ok.then_some(out)
    }

    fn bounds(dictionary: *const c_void) -> Option<Rect> {
        let value = unsafe { CFDictionaryGetValue(dictionary, kCGWindowBounds) };
        if value.is_null() {
            return None;
        }
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        if !unsafe { CGRectMakeWithDictionaryRepresentation(value.cast(), &mut rect) } {
            return None;
        }
        if !rect.size.width.is_finite()
            || !rect.size.height.is_finite()
            || !rect.origin.x.is_finite()
            || !rect.origin.y.is_finite()
        {
            return None;
        }
        Some(Rect {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }

    pub(super) fn windows() -> Vec<WindowInfo> {
        unsafe {
            let array =
                CGWindowListCopyWindowInfo(LIST_ON_SCREEN_ONLY | LIST_EXCLUDE_DESKTOP_ELEMENTS, 0);
            if array.is_null() {
                return Vec::new();
            }
            let count = CFArrayGetCount(array);
            let mut windows = Vec::new();
            for index in 0..count {
                let dictionary = CFArrayGetValueAtIndex(array, index);
                if dictionary.is_null() {
                    continue;
                }
                let Some(bounds) = bounds(dictionary) else {
                    continue;
                };
                windows.push(WindowInfo {
                    pid: number(dictionary, kCGWindowOwnerPID).unwrap_or(-1.0) as i64,
                    layer: number(dictionary, kCGWindowLayer).unwrap_or(-1.0) as i64,
                    bounds,
                });
            }
            CFRelease(array);
            windows
        }
    }

    /// sidecar 由 DBX 直接拉起：父进程拥有宿主窗口（主窗口或浮动插件窗口）。
    pub(super) fn owner_pid() -> i64 {
        std::os::unix::process::parent_id() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(pid: i64, layer: i64, x: f64, y: f64, width: f64, height: f64) -> WindowInfo {
        WindowInfo {
            pid,
            layer,
            bounds: Rect {
                x,
                y,
                width,
                height,
            },
        }
    }

    #[test]
    fn picks_the_frontmost_host_window_under_the_cursor() {
        // 另一个应用的全屏窗口压在宿主上面：最前面的命中窗口不是宿主，
        // 此时不能拿宿主的窗口做校准。
        let windows = [
            window(7, 0, 0.0, 0.0, 1600.0, 1000.0),
            window(42, 0, 100.0, 40.0, 1400.0, 900.0),
        ];
        assert_eq!(
            pick_window(Point { x: 700.0, y: 300.0 }, &windows, 42),
            None
        );
    }

    #[test]
    fn returns_the_host_window_when_it_is_frontmost() {
        let windows = [
            window(42, 0, 100.0, 40.0, 1400.0, 900.0),
            window(7, 0, 0.0, 0.0, 1600.0, 1000.0),
        ];
        assert_eq!(
            pick_window(Point { x: 700.0, y: 300.0 }, &windows, 42),
            Some(Rect {
                x: 100.0,
                y: 40.0,
                width: 1400.0,
                height: 900.0,
            })
        );
    }

    #[test]
    fn skips_drag_images_panels_and_windows_outside_the_cursor() {
        let windows = [
            // 拖拽图标等高层级窗口不是落点窗口。
            window(88, 500, 600.0, 200.0, 64.0, 64.0),
            // 太小的浮层不能代表宿主窗口。
            window(42, 0, 600.0, 200.0, 120.0, 40.0),
            // 宿主窗口：光标在窗口外时不命中。
            window(42, 0, 100.0, 140.0, 1400.0, 900.0),
        ];
        assert_eq!(
            pick_window(Point { x: 700.0, y: 100.0 }, &windows, 42),
            None
        );
        assert_eq!(
            pick_window(Point { x: 700.0, y: 300.0 }, &windows, 42),
            Some(Rect {
                x: 100.0,
                y: 140.0,
                width: 1400.0,
                height: 900.0,
            })
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires a live macOS session (window server cursor)"]
    fn reads_the_live_cursor() {
        let sample = sample();
        let cursor = sample.cursor.expect("CGEventCreate(NULL) must expose the cursor");
        assert!(cursor.x.is_finite() && cursor.y.is_finite());
        assert_eq!(sample.platform, "macos");
    }
}
