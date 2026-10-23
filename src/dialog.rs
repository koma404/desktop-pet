//! 「给伪装文件夹改名」的小对话框（不用资源文件，直接创建控件）。

use crate::win::{loword, wide};
use std::cell::Cell;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, COLOR_BTNFACE, HBRUSH, HFONT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{EM_LIMITTEXT, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowTextLengthW, GetWindowTextW, LoadCursorW,
    RegisterClassExW, SendMessageW, SetForegroundWindow, ShowWindow, SystemParametersInfoW, BS_DEFPUSHBUTTON, BS_PUSHBUTTON,
    ES_AUTOHSCROLL, HICON, HMENU, IDCANCEL, IDC_ARROW, IDOK, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_DESTROY, WM_SETFONT, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_TOPMOST, WS_POPUP, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

const CLASS: &str = "TuanziRenameDialog";
const ID_EDIT: i32 = 100;
/// 静态文本不把 & 当作快捷键前缀
const SS_NOPREFIX: u32 = 0x80;

thread_local! {
    static DLG: Cell<HWND> = Cell::new(std::ptr::null_mut());
    static EDIT: Cell<HWND> = Cell::new(std::ptr::null_mut());
    static FONT: Cell<HFONT> = Cell::new(std::ptr::null_mut());
}

/// 当前打开的对话框（没有打开时为空），消息循环里用来处理 Tab / 回车 / Esc。
pub fn hwnd() -> HWND {
    DLG.with(|d| d.get())
}

pub fn register(icon: HICON, icon_sm: HICON) {
    let class = wide(CLASS);
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(dlg_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(std::ptr::null()),
            hIcon: icon,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_BTNFACE + 1) as usize as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: icon_sm,
        };
        RegisterClassExW(&wc);
    }
}

/// 打开改名对话框；已经打开时只是把它带到前面。
pub fn open(owner: HWND, current: &str, scale: f32) {
    unsafe {
        let existing = hwnd();
        if !existing.is_null() {
            SetForegroundWindow(existing);
            return;
        }
        let d = |v: f32| (v * scale).round() as i32;
        let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let ex = WS_EX_DLGMODALFRAME | WS_EX_TOPMOST;
        let mut rc = RECT { left: 0, top: 0, right: d(300.0), bottom: d(112.0) };
        AdjustWindowRectEx(&mut rc, style, 0, ex);
        let (ww, wh) = (rc.right - rc.left, rc.bottom - rc.top);
        let wa = crate::win::primary_work_area();
        let (x, y) = (wa.left + (wa.right - wa.left - ww) / 2, wa.top + (wa.bottom - wa.top - wh) / 2);

        let hinst = GetModuleHandleW(std::ptr::null());
        let (class, title) = (wide(CLASS), wide("给文件夹改个名字"));
        let dlg =
            CreateWindowExW(ex, class.as_ptr(), title.as_ptr(), style, x, y, ww, wh, owner, std::ptr::null_mut(), hinst, std::ptr::null());
        if dlg.is_null() {
            return;
        }
        DLG.with(|c| c.set(dlg));

        // 对话框字体：和系统消息框一致
        let mut ncm: NONCLIENTMETRICSW = std::mem::zeroed();
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        let font = if SystemParametersInfoW(SPI_GETNONCLIENTMETRICS, ncm.cbSize, &mut ncm as *mut NONCLIENTMETRICSW as *mut _, 0) != 0 {
            CreateFontIndirectW(&ncm.lfMessageFont)
        } else {
            std::ptr::null_mut()
        };
        FONT.with(|c| c.set(font));

        let child = |ex: u32, class: &str, text: &str, style: u32, x: f32, y: f32, w: f32, h: f32, id: i32| -> HWND {
            let (c, t) = (wide(class), wide(text));
            let h = CreateWindowExW(
                ex,
                c.as_ptr(),
                t.as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                d(x),
                d(y),
                d(w),
                d(h),
                dlg,
                id as isize as HMENU,
                hinst,
                std::ptr::null(),
            );
            if !font.is_null() {
                SendMessageW(h, WM_SETFONT, font as usize, 1);
            }
            h
        };
        child(0, "STATIC", "团子伪装成的文件夹，想叫什么名字呢？", SS_NOPREFIX, 14.0, 14.0, 272.0, 20.0, -1);
        let edit = child(WS_EX_CLIENTEDGE, "EDIT", current, WS_TABSTOP | ES_AUTOHSCROLL as u32, 14.0, 38.0, 272.0, 24.0, ID_EDIT);
        child(0, "BUTTON", "确定", WS_TABSTOP | BS_DEFPUSHBUTTON as u32, 118.0, 76.0, 80.0, 26.0, IDOK);
        child(0, "BUTTON", "取消", WS_TABSTOP | BS_PUSHBUTTON as u32, 206.0, 76.0, 80.0, 26.0, IDCANCEL);
        EDIT.with(|c| c.set(edit));
        SendMessageW(edit, EM_LIMITTEXT, 40, 0);
        SendMessageW(edit, EM_SETSEL, 0, -1);

        ShowWindow(dlg, SW_SHOW);
        SetForegroundWindow(dlg);
        SetFocus(edit);
    }
}

unsafe extern "system" fn dlg_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let id = loword(wp) as i32;
            if id == IDOK {
                let edit = EDIT.with(|c| c.get());
                let len = GetWindowTextLengthW(edit).max(0) as usize;
                let mut buf = vec![0u16; len + 1];
                let n = GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
                let name = String::from_utf16_lossy(&buf[..n]);
                crate::app::with_app(|a| a.rename(&name));
                DestroyWindow(hwnd);
                return 0;
            }
            if id == IDCANCEL {
                DestroyWindow(hwnd);
                return 0;
            }
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            return 0;
        }
        WM_DESTROY => {
            DLG.with(|c| c.set(std::ptr::null_mut()));
            EDIT.with(|c| c.set(std::ptr::null_mut()));
            let font = FONT.with(|c| c.replace(std::ptr::null_mut()));
            if !font.is_null() {
                DeleteObject(font);
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
