//! 用 GDI 把文字画成 8 位覆盖率蒙版，再由我们自己叠加白字和阴影，
//! 这样在透明窗口上也能得到和 Win7 桌面图标一样的「白字 + 柔和黑影」。

use crate::win::wide;
use windows_sys::Win32::Foundation::SIZE;
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW, DeleteDC, DeleteObject, GdiFlush, GetTextExtentPoint32W, GetTextMetricsW,
    SelectObject, SetBkMode, SetTextColor, TextOutW, ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC, HFONT,
    LOGFONTW, TEXTMETRICW, TRANSPARENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_GETICONTITLELOGFONT};

/// 渲染好的一段文字。
pub struct Label {
    pub w: i32,
    pub h: i32,
    /// 文字本身的覆盖率
    pub cov: Vec<u8>,
    /// 模糊加粗后的阴影
    pub shadow: Vec<u8>,
}

pub struct TextRenderer {
    font: HFONT,
    line_h: i32,
}

impl TextRenderer {
    /// 使用「桌面图标标题」的系统字体（中文 Win7 上一般是微软雅黑 9 号）。
    pub fn new(scale: f32) -> TextRenderer {
        unsafe {
            let mut lf: LOGFONTW = std::mem::zeroed();
            let got = SystemParametersInfoW(
                SPI_GETICONTITLELOGFONT,
                std::mem::size_of::<LOGFONTW>() as u32,
                &mut lf as *mut LOGFONTW as *mut _,
                0,
            ) != 0;
            if !got || lf.lfHeight == 0 {
                lf = std::mem::zeroed();
                lf.lfHeight = -(12.0 * scale).round() as i32;
                lf.lfWeight = 400;
                let face = wide("Microsoft YaHei");
                lf.lfFaceName[..face.len()].copy_from_slice(&face);
            }
            // 灰度抗锯齿（ClearType 的彩边在透明背景上会很难看）
            lf.lfQuality = ANTIALIASED_QUALITY;
            let font = CreateFontIndirectW(&lf);

            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let old = SelectObject(dc, font);
            let mut tm: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(dc, &mut tm);
            SelectObject(dc, old);
            DeleteDC(dc);
            let line_h = if tm.tmHeight > 0 { tm.tmHeight } else { (16.0 * scale) as i32 };
            TextRenderer { font, line_h }
        }
    }

    fn measure(dc: HDC, s: &[u16]) -> i32 {
        let mut sz = SIZE { cx: 0, cy: 0 };
        unsafe { GetTextExtentPoint32W(dc, s.as_ptr(), s.len() as i32, &mut sz) };
        sz.cx
    }

    /// 按宽度折行（逐字折行，中文没有空格也能断），最多 `max_lines` 行，超出部分用「…」。
    fn wrap(dc: HDC, text: &str, max_w: i32, max_lines: usize) -> Vec<Vec<u16>> {
        let mut lines: Vec<Vec<u16>> = Vec::new();
        let mut truncated = false;
        'outer: for para in text.split('\n') {
            let chars: Vec<u16> = para.encode_utf16().collect();
            let mut start = 0;
            loop {
                if lines.len() == max_lines {
                    truncated = true;
                    break 'outer;
                }
                let mut end = chars.len();
                while end > start + 1 && Self::measure(dc, &chars[start..end]) > max_w {
                    end -= 1;
                }
                lines.push(chars[start..end].to_vec());
                start = end;
                if start >= chars.len() {
                    break;
                }
            }
        }
        if truncated {
            if let Some(last) = lines.last_mut() {
                let ell = '…' as u16;
                last.push(ell);
                while last.len() > 1 && Self::measure(dc, last) > max_w {
                    last.remove(last.len() - 2);
                }
            }
        }
        lines
    }

    pub fn render(&self, text: &str, max_w: i32, max_lines: usize) -> Label {
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            let old_font = SelectObject(dc, self.font);
            let lines = Self::wrap(dc, text, max_w, max_lines);
            let pad = 2;
            let w = max_w + pad * 2;
            let h = (lines.len() as i32 * self.line_h + pad * 2).max(1);

            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let bmp = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let mut cov = vec![0u8; (w * h) as usize];
            if !bmp.is_null() && !bits.is_null() {
                let px = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
                px.fill(0);
                let old_bmp = SelectObject(dc, bmp);
                SetTextColor(dc, 0x00FF_FFFF);
                SetBkMode(dc, TRANSPARENT as i32);
                for (i, line) in lines.iter().enumerate() {
                    let lw = Self::measure(dc, line);
                    TextOutW(dc, (w - lw) / 2, pad + i as i32 * self.line_h, line.as_ptr(), line.len() as i32);
                }
                GdiFlush();
                for (c, p) in cov.iter_mut().zip(px.chunks_exact(4)) {
                    *c = p[0].max(p[1]).max(p[2]);
                }
                SelectObject(dc, old_bmp);
                DeleteObject(bmp);
            }
            SelectObject(dc, old_font);
            DeleteDC(dc);

            let shadow = make_shadow(&cov, w, h);
            Label { w, h, cov, shadow }
        }
    }
}

impl Drop for TextRenderer {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.font) };
    }
}

/// 3×3 模糊后加浓，模仿 Win7 桌面图标文字的柔和阴影。
fn make_shadow(cov: &[u8], w: i32, h: i32) -> Vec<u8> {
    let mut out = vec![0u8; cov.len()];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0u32;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (sx, sy) = (x + dx, y + dy);
                    if sx >= 0 && sy >= 0 && sx < w && sy < h {
                        sum += cov[(sy * w + sx) as usize] as u32;
                    }
                }
            }
            out[(y * w + x) as usize] = (sum * 2 / 9).min(255) as u8;
        }
    }
    out
}
