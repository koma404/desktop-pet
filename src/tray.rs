//! 托盘图标：右下角的小团子，鼠标停上去显示内存，左键找团子，右键菜单。

use crate::art;
use crate::canvas::Canvas;
use crate::win::copy_wide;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW};
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON};

pub struct Tray {
    hwnd: HWND,
    msg: u32,
    icon: HICON,
    pub added: bool,
    tip: String,
}

/// 用造型代码画一个带透明度的图标。
pub fn make_icon(size: i32) -> HICON {
    unsafe {
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let screen = GetDC(std::ptr::null_mut());
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(screen, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        ReleaseDC(std::ptr::null_mut(), screen);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let px = std::slice::from_raw_parts_mut(bits as *mut u8, (size * size * 4) as usize);
        px.fill(0);
        {
            let mut cv = Canvas::new(size, size, px);
            art::draw_icon(&mut cv, size);
        }
        // 图标要的是「非预乘」的 Alpha
        for p in px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a > 0 && a < 255 {
                for c in p[..3].iter_mut() {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        let mask_bits = vec![0u8; (((size + 15) / 16) * 2 * size) as usize];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const _);
        let ii = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}

impl Tray {
    pub fn new(hwnd: HWND, callback_msg: u32) -> Tray {
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        Tray { hwnd, msg: callback_msg, icon: make_icon(size), added: false, tip: "团子".into() }
    }

    fn data(&self, flags: u32) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = 1;
        nid.uFlags = flags;
        nid.uCallbackMessage = self.msg;
        nid.hIcon = self.icon;
        copy_wide(&mut nid.szTip, &self.tip);
        nid
    }

    /// 添加图标（资源管理器还没准备好时会失败，稍后会重试）。
    pub fn add(&mut self) -> bool {
        let nid = self.data(NIF_ICON | NIF_MESSAGE | NIF_TIP);
        self.added = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) } != 0;
        self.added
    }

    /// 资源管理器重启后需要重新添加。
    pub fn readd(&mut self) {
        self.added = false;
        self.add();
    }

    pub fn set_tip(&mut self, tip: &str) {
        if tip == self.tip && self.added {
            return;
        }
        self.tip = tip.to_string();
        if self.added {
            let nid = self.data(NIF_TIP);
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
        } else {
            self.add();
        }
    }

    /// 收起托盘图标（退出时调用）。
    pub fn remove(&mut self) {
        if self.added {
            let nid = self.data(0);
            unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        if !self.icon.is_null() {
            unsafe { DestroyIcon(self.icon) };
        }
    }
}
