//! Win32 的零碎小工具。

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONULL};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetCursorPos, GetWindow, MessageBoxW, SystemParametersInfoW, GW_HWNDPREV, MESSAGEBOX_STYLE, SPI_GETWORKAREA,
};

/// 转成以 0 结尾的 UTF-16。
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 复制到定长的 UTF-16 缓冲区（会截断并保证以 0 结尾）。
pub fn copy_wide(dst: &mut [u16], s: &str) {
    let mut n = 0;
    for c in s.encode_utf16() {
        if n + 1 >= dst.len() {
            break;
        }
        dst[n] = c;
        n += 1;
    }
    for d in dst[n..].iter_mut() {
        *d = 0;
    }
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn cursor_pos() -> POINT {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pt) };
    pt
}

/// 窗口 `a` 在 Z 序上是否位于窗口 `b` 之上。
pub fn is_above(a: HWND, b: HWND) -> bool {
    let mut h = b;
    for _ in 0..4096 {
        h = unsafe { GetWindow(h, GW_HWNDPREV) };
        if h.is_null() {
            return false;
        }
        if h == a {
            return true;
        }
    }
    false
}

/// 主显示器的工作区（去掉任务栏）。
pub fn primary_work_area() -> RECT {
    let mut rc = RECT { left: 0, top: 0, right: 1024, bottom: 768 };
    unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut rc as *mut RECT as *mut _, 0) };
    rc
}

/// 这个点是否落在某个显示器上。
pub fn point_on_screen(x: i32, y: i32) -> bool {
    unsafe { !MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL).is_null() }
}

pub fn message_box(owner: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> i32 {
    let (t, c) = (wide(text), wide("团子"));
    unsafe { MessageBoxW(owner, t.as_ptr(), c.as_ptr(), flags) }
}

#[inline]
pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

/// 把字节数写成「812 MB」或「1.2 GB」。
pub fn fmt_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}
