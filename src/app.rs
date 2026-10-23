//! 团子的大脑：状态、动画、鼠标交互、菜单，以及两个窗口的消息处理。
//!
//! 窗口分工：
//! - hub：隐藏的顶层窗口，负责定时器、托盘、菜单和各种通知；
//! - pet：桌面上看得见的分层窗口（UpdateLayeredWindow，逐像素透明）。
//!
//! 所有状态放在线程局部的 `APP` 里，通过 [`with_app`] 访问。窗口过程可能被重入
//! （比如 SetWindowPos 会同步发送 WM_WINDOWPOSCHANGING），所以用 `try_borrow_mut`，
//! 拿不到时直接交给默认处理，并且绝不在借用期间弹出菜单或对话框。

use crate::art::{self, Dango, Face};
use crate::canvas::{smoothstep, Canvas, Pen, BLACK, WHITE};
use crate::config::{self, Config};
use crate::level::{next_level, Level};
use crate::memory::{self, CleanKind, MemStatus, ProcGroup};
use crate::text::{Label, TextRenderer};
use crate::tray::{self, Tray};
use crate::win::{class_name, cursor_pos, fmt_bytes, is_above, loword, message_box, point_on_screen, primary_work_area, wide};
use crate::{dialog, system};
use std::cell::RefCell;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, GetDeviceCaps, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, LOGPIXELSX,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows_sys::Win32::UI::Controls::{InitCommonControlsEx, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, WM_MOUSELEAVE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub const HUB_CLASS: &str = "TuanziHub";
const PET_CLASS: &str = "TuanziPet";

const WM_TRAY: u32 = WM_APP + 1;
const WM_APP_MENU: u32 = WM_APP + 2;
const WM_APP_CLEANED: u32 = WM_APP + 3;
const WM_APP_SHOW: u32 = WM_APP + 4;
const WM_APP_AUTOSTART: u32 = WM_APP + 5;

const TIMER_SAMPLE: usize = 1;
const TIMER_ANIM: usize = 2;
/// 动画帧间隔（20 帧/秒，只在团子现身时运行）
const FRAME_MS: u32 = 50;

/// 鼠标停留多久才现形（免得鼠标路过时一闪一闪）
const HOVER_DELAY: Duration = Duration::from_millis(150);
/// 鼠标离开后多久变回文件夹
const LEAVE_DELAY: Duration = Duration::from_millis(450);
/// 变身动画时长（秒）
const MORPH_SECS: f32 = 0.32;
/// 被戳后的弹性动画时长（秒）
const POKE_SECS: f32 = 1.1;
/// 「安静模式」时长
const SNOOZE: Duration = Duration::from_secs(30 * 60);

const PRESETS: [(u32, u32); 3] = [(70, 80), (75, 85), (80, 90)];

const CMD_CLEAN_LIGHT: u32 = 10;
const CMD_CLEAN_DEEP: u32 = 11;
const CMD_PRESET: u32 = 20;
const CMD_AUTO_CLEAN: u32 = 30;
const CMD_POPUP: u32 = 31;
const CMD_AUTOSTART: u32 = 32;
const CMD_RENAME: u32 = 33;
const CMD_OPEN_CFG: u32 = 34;
const CMD_RESET_POS: u32 = 35;
const CMD_SNOOZE: u32 = 40;
const CMD_TOGGLE_VIS: u32 = 41;
const CMD_EXIT: u32 = 50;
const CMD_PROC: u32 = 100;

/// 团子窗口当前是否应该置顶（WM_WINDOWPOSCHANGING 里要读，不能等借用）。
static TOP_MODE: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static HUB: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

thread_local! {
    static APP: RefCell<Option<App>> = RefCell::new(None);
}

/// 访问全局状态；如果正被占用（窗口过程重入），返回 None。
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

struct Press {
    cursor: POINT,
    win: (i32, i32),
    dragging: bool,
}

fn active(t: Option<Instant>, now: Instant) -> bool {
    t.map_or(false, |t| now < t)
}

/// 桌面窗口（Progman，或「显示桌面」时被抬起来的 WorkerW）。
fn is_desktop(hwnd: HWND) -> bool {
    !hwnd.is_null() && matches!(class_name(hwnd).as_str(), "WorkerW" | "Progman")
}

fn gb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

pub struct App {
    pub hub: HWND,
    pet: HWND,
    pub cfg: Config,
    cfg_mtime: Option<SystemTime>,
    is_admin: bool,
    pub scale: f32,
    w: i32,
    h: i32,
    pos: (i32, i32),

    screen_dc: HDC,
    mem_dc: HDC,
    dib: HBITMAP,
    old_bmp: HGDIOBJ,
    bits: *mut u8,
    text: TextRenderer,
    label: Option<(String, Label)>,
    pub tray: Tray,

    mem: MemStatus,
    level: Level,
    start: Instant,
    last_frame: Instant,
    anim_on: bool,
    /// 0 = 文件夹，1 = 团子
    morph: f32,
    visible: bool,
    menu_open: bool,

    hover: bool,
    hover_since: Instant,
    hover_left: Option<Instant>,
    press: Option<Press>,
    poke_at: Option<Instant>,

    cleaning: Option<CleanKind>,
    clean_auto: bool,
    happy_until: Option<Instant>,
    message: Option<(String, Instant)>,
    last_auto_clean: Option<Instant>,
    snooze_until: Option<Instant>,
    summon_until: Option<Instant>,
    /// 「显示桌面」时桌面层被抬到了最上面，团子也要跟着浮上来
    desktop_raised: bool,
    autostart_quiet: bool,
    rng: u32,
}

impl App {
    fn new(hub: HWND, pet: HWND, cfg: Config, is_admin: bool) -> App {
        unsafe {
            let screen_dc = GetDC(std::ptr::null_mut());
            let dpi = GetDeviceCaps(screen_dc, LOGPIXELSX as i32);
            let scale = if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 };
            let w = (art::CANVAS_W * scale).round() as i32;
            let h = (art::CANVAS_H * scale).round() as i32;

            let mem_dc = CreateCompatibleDC(screen_dc);
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
            let dib = CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
            let old_bmp = SelectObject(mem_dc, dib);
            let now = Instant::now();
            let seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7) | 1;

            let mut app = App {
                hub,
                pet,
                cfg_mtime: config::mtime(),
                cfg,
                is_admin,
                scale,
                w,
                h,
                pos: (0, 0),
                screen_dc,
                mem_dc,
                dib,
                old_bmp,
                bits: bits as *mut u8,
                text: TextRenderer::new(scale),
                label: None,
                tray: Tray::new(hub, WM_TRAY),
                mem: MemStatus::default(),
                level: Level::Normal,
                start: now,
                last_frame: now,
                anim_on: false,
                morph: 0.0,
                visible: true,
                menu_open: false,
                hover: false,
                hover_since: now,
                hover_left: None,
                press: None,
                poke_at: None,
                cleaning: None,
                clean_auto: false,
                happy_until: None,
                message: None,
                last_auto_clean: None,
                snooze_until: None,
                summon_until: None,
                desktop_raised: false,
                autostart_quiet: true,
                rng: seed,
            };
            app.pos = app.valid_pos(app.cfg.pos);
            app
        }
    }

    /// 启动：托盘、第一次采样、显示窗口、定时器、同步开机自启。
    /// `by_user` 为 true 表示是用户手动双击启动的（不是开机自启）。
    fn start(&mut self, first_run: bool, by_user: bool) {
        self.tray.add();
        self.mem = memory::sample();
        self.level = next_level(Level::Normal, self.mem.percent(), self.cfg.warn, self.cfg.alert);
        unsafe {
            SetWindowPos(self.pet, HWND_BOTTOM, self.pos.0, self.pos.1, self.w, self.h, SWP_NOACTIVATE);
        }
        self.render();
        unsafe {
            ShowWindow(self.pet, SW_SHOWNOACTIVATE);
            SetTimer(self.hub, TIMER_SAMPLE, self.cfg.interval_sec * 1000, None);
        }
        self.tray.set_tip(&self.tip_text());
        if first_run {
            self.save_cfg();
        }
        if by_user {
            // 手动启动时先在最前面露个脸，免得被资源管理器窗口挡住、以为没启动
            self.summon_until = Some(Instant::now() + Duration::from_secs(5));
            self.set_message(if first_run { "你好呀~\n我是团子" } else { "我来啦~" }, 4.0);
        }
        if self.cfg.autostart {
            // 每次启动都重新登记一次，exe 换了位置也能跟上
            self.apply_autostart(true, true);
        }
        self.after_state_change();
    }

    /// 退出前：收起托盘图标、停掉定时器。
    fn shutdown(&mut self) {
        unsafe {
            KillTimer(self.hub, TIMER_SAMPLE);
            KillTimer(self.hub, TIMER_ANIM);
        }
        self.anim_on = false;
        self.tray.remove();
    }

    // ------------------------------------------------------------------
    // 周期采样
    // ------------------------------------------------------------------

    fn sample(&mut self) {
        let now = Instant::now();
        self.mem = memory::sample();
        self.level = next_level(self.level, self.mem.percent(), self.cfg.warn, self.cfg.alert);

        if self.level == Level::Alert && self.cfg.auto_clean && self.cleaning.is_none() {
            let cooldown = Duration::from_secs(self.cfg.cooldown_min as u64 * 60);
            if self.last_auto_clean.map_or(true, |t| now.duration_since(t) >= cooldown) {
                self.last_auto_clean = Some(now);
                self.start_clean(CleanKind::Light, true);
            }
        }
        if self.snooze_until.map_or(false, |t| now >= t) {
            self.snooze_until = None;
        }
        if !self.tray.added {
            self.tray.add();
        }
        self.tray.set_tip(&self.tip_text());
        self.check_config_file();
        // 兜底：万一漏掉了前台切换事件，也能发现「显示桌面」
        let fg = unsafe { GetForegroundWindow() };
        self.on_foreground(fg);
        self.after_state_change();
    }

    /// 状态变化后：更新置顶，需要动就开动画，否则重画一帧静态画面。
    fn after_state_change(&mut self) {
        self.update_z();
        if self.needs_anim(Instant::now()) {
            self.ensure_anim();
        } else {
            self.render();
        }
    }

    fn tip_text(&self) -> String {
        let quiet = if self.snoozed(Instant::now()) { "\n（安静模式中）" } else { "" };
        format!("团子 · 内存 {:.0}%\n已用 {:.1} GB，共 {:.1} GB{}", self.mem.percent(), gb(self.mem.used()), gb(self.mem.total), quiet)
    }

    fn snoozed(&self, now: Instant) -> bool {
        active(self.snooze_until, now)
    }

    /// 心情值：0 平静蓝 → 1 紧张黄 → 2 告急红，随占用平滑过渡。
    fn mood(&self) -> f32 {
        let p = self.mem.percent();
        let (w, a) = (self.cfg.warn as f32, self.cfg.alert as f32);
        smoothstep(w - 10.0, w, p) + smoothstep(a - 6.0, a, p)
    }

    // ------------------------------------------------------------------
    // 现形 / 动画
    // ------------------------------------------------------------------

    fn hover_shown(&self, now: Instant) -> bool {
        self.hover && now.duration_since(self.hover_since) >= HOVER_DELAY
    }

    fn lingering(&self, now: Instant) -> bool {
        !self.hover && self.morph > 0.0 && self.hover_left.map_or(false, |t| now.duration_since(t) < LEAVE_DELAY)
    }

    fn poke_age(&self, now: Instant) -> Option<f32> {
        self.poke_at.map(|t| now.duration_since(t).as_secs_f32()).filter(|&u| u < POKE_SECS)
    }

    /// 现在应该是团子形态（而不是文件夹）吗？
    fn want_dango(&self, now: Instant) -> bool {
        self.hover_shown(now)
            || self.lingering(now)
            || self.menu_open
            || self.level != Level::Normal
            || self.cleaning.is_some()
            || self.message.as_ref().map_or(false, |m| now < m.1)
            || active(self.happy_until, now)
            || active(self.summon_until, now)
            || self.poke_age(now).is_some()
    }

    fn needs_anim(&self, now: Instant) -> bool {
        self.visible && (self.want_dango(now) || self.morph > 0.0 || self.hover)
    }

    fn ensure_anim(&mut self) {
        if !self.anim_on && self.visible {
            unsafe { SetTimer(self.hub, TIMER_ANIM, FRAME_MS, None) };
            self.anim_on = true;
            self.last_frame = Instant::now();
        }
    }

    fn stop_anim(&mut self) {
        if self.anim_on {
            unsafe { KillTimer(self.hub, TIMER_ANIM) };
            self.anim_on = false;
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        if self.message.as_ref().map_or(false, |m| now >= m.1) {
            self.message = None;
        }
        if self.summon_until.map_or(false, |t| now >= t) {
            self.summon_until = None;
        }
        let target = if self.want_dango(now) { 1.0 } else { 0.0 };
        let step = dt / MORPH_SECS;
        self.morph = if self.morph < target { (self.morph + step).min(target) } else { (self.morph - step).max(target) };
        self.update_z();
        self.render();
        if !self.needs_anim(now) {
            self.stop_anim();
        }
    }

    /// 根据当前状态决定团子这一帧的样子。
    fn dango_params(&self, now: Instant) -> Dango {
        let t = now.duration_since(self.start).as_secs_f32();
        let mut d = Dango { t, mood: self.mood(), breathe: 1.0, breathe_period: 2.8, ..Dango::calm() };
        match self.level {
            Level::Normal => {}
            Level::Warn => {
                d.face = Face::Worried;
                d.sweat = 1;
                d.breathe_period = 1.7;
            }
            Level::Alert => {
                d.face = Face::Panic;
                d.sweat = 2;
                d.breathe = 1.3;
                d.breathe_period = 0.9;
                d.shake = 0.7 * (t * 40.0).sin();
            }
        }
        if active(self.happy_until, now) {
            d.face = Face::Happy;
            d.sparkles = true;
            d.sweat = 0;
            d.shake = 0.0;
        }
        if self.cleaning.is_some() {
            d.face = Face::Busy;
            d.sweat = 0;
            d.shake = 0.0;
            d.sparkles = false;
            d.breathe = 1.6;
            d.breathe_period = 0.55;
        }
        let poke = self.poke_age(now);
        if let Some(u) = poke {
            d.squash = 0.24 * (-5.0 * u).exp() * (15.0 * u).cos();
            d.shake = 0.0;
            if u < 0.4 {
                d.face = Face::Squeeze;
            } else if self.level == Level::Normal && self.cleaning.is_none() {
                d.face = Face::Happy;
            }
        }
        if matches!(d.face, Face::Normal | Face::Worried) {
            let c = t % 3.7;
            let n = (t / 3.7) as u32;
            d.blink = c < 0.12 || (n % 3 == 0 && c > 0.3 && c < 0.42);
        }
        // 心情好的时候，每隔几秒轻轻弹一下
        if d.face == Face::Normal && poke.is_none() {
            let u = t % 5.3;
            if u < 0.42 {
                let k = (PI * u / 0.42).sin();
                d.hop = 3.5 * k;
                d.squash = -0.05 * k;
            } else if u < 0.62 {
                d.squash = 0.07 * (PI * (u - 0.42) / 0.2).sin();
            }
        }
        d
    }

    fn numbers_text(&self) -> String {
        format!("内存 {:.0}%\n{:.1} / {:.1} GB", self.mem.percent(), gb(self.mem.used()), gb(self.mem.total))
    }

    fn label_text(&self, now: Instant) -> String {
        if let Some((m, until)) = &self.message {
            if now < *until {
                return m.clone();
            }
        }
        if self.hover_shown(now) || self.lingering(now) || self.menu_open {
            return self.numbers_text();
        }
        if !self.want_dango(now) {
            return self.cfg.folder_name.clone();
        }
        String::new()
    }

    fn render(&mut self) {
        if !self.visible || self.bits.is_null() {
            return;
        }
        let now = Instant::now();
        let d = self.dango_params(now);
        let text = self.label_text(now);
        if !text.is_empty() && self.label.as_ref().map_or(true, |(t, _)| *t != text) {
            let lab = self.text.render(&text, (art::LABEL_W * self.scale) as i32, 2);
            self.label = Some((text.clone(), lab));
        }
        let (w, h, s) = (self.w, self.h, self.scale);
        let px = unsafe { std::slice::from_raw_parts_mut(self.bits, (w * h * 4) as usize) };
        let mut cv = Canvas::new(w, h, px);
        // 几乎透明（Alpha = 1）的底色：看不见，但整个格子都能接收鼠标，和真的桌面图标一样
        cv.clear([0, 0, 0, 1]);
        {
            let mut pen = Pen::new(&mut cv, s);
            art::draw_scene(&mut pen, self.morph, &d);
        }
        if !text.is_empty() {
            if let Some((_, lab)) = &self.label {
                let ox = (w - lab.w) / 2;
                let oy = (art::LABEL_TOP * s).round() as i32;
                cv.blend_mask(ox + 1, oy + 1, lab.w, lab.h, &lab.shadow, BLACK.alpha(0.8));
                cv.blend_mask(ox, oy, lab.w, lab.h, &lab.cov, WHITE);
            }
        }
        unsafe {
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let dst = POINT { x: self.pos.0, y: self.pos.1 };
            let blend =
                BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            UpdateLayeredWindow(self.pet, self.screen_dc, &dst, &size, self.mem_dc, &src, 0, &blend, ULW_ALPHA);
        }
    }

    fn set_message(&mut self, text: impl Into<String>, secs: f32) {
        self.message = Some((text.into(), Instant::now() + Duration::from_secs_f32(secs)));
        self.ensure_anim();
    }

    // ------------------------------------------------------------------
    // 窗口层级与位置
    // ------------------------------------------------------------------

    /// 平时待在最底层（像桌面图标一样），告急 / 显示桌面 / 被召唤时浮到最上面。
    fn update_z(&mut self) {
        let now = Instant::now();
        let alert_pop = self.level == Level::Alert && self.cfg.popup_on_alert && !self.snoozed(now);
        let want = self.visible && (alert_pop || self.desktop_raised || active(self.summon_until, now));
        if want == TOP_MODE.load(Ordering::Relaxed) {
            return;
        }
        self.set_top(want);
        if !want {
            // 刚沉到底层时，如果正处于「显示桌面」（桌面层在我们上面），就跟着浮上来
            let fg = unsafe { GetForegroundWindow() };
            if self.visible && is_desktop(fg) && is_above(fg, self.pet) {
                self.desktop_raised = true;
                self.set_top(true);
            }
        }
    }

    fn set_top(&mut self, top: bool) {
        TOP_MODE.store(top, Ordering::Relaxed);
        let after = if top { HWND_TOPMOST } else { HWND_BOTTOM };
        unsafe { SetWindowPos(self.pet, after, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    }

    /// 前台窗口变了：判断用户是不是按了「显示桌面」（Win7 会把桌面层抬到最上面）。
    fn on_foreground(&mut self, fg: HWND) {
        if fg.is_null() || fg == self.pet || fg == self.hub || fg == dialog::hwnd() {
            return;
        }
        match class_name(fg).as_str() {
            "WorkerW" | "Progman" => {
                if !self.desktop_raised && !TOP_MODE.load(Ordering::Relaxed) {
                    self.desktop_raised = is_above(fg, self.pet);
                }
            }
            // 任务栏、开始菜单、菜单、缩略图等：不改变判断
            "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "#32768"
            | "DV2ControlHost"
            | "Button"
            | "TaskListThumbnailWnd"
            | "tooltips_class32"
            | "TaskSwitcherWnd" => {}
            _ => self.desktop_raised = false,
        }
        self.update_z();
    }

    fn default_pos(&self) -> (i32, i32) {
        let wa = primary_work_area();
        let m = (12.0 * self.scale) as i32;
        (wa.right - self.w - m, wa.top + m)
    }

    fn valid_pos(&self, p: Option<(i32, i32)>) -> (i32, i32) {
        match p {
            Some((x, y)) if point_on_screen(x + self.w / 2, y + self.h / 2) => (x, y),
            _ => self.default_pos(),
        }
    }

    fn move_to(&mut self, p: (i32, i32)) {
        self.pos = p;
        unsafe { SetWindowPos(self.pet, std::ptr::null_mut(), p.0, p.1, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    /// 分辨率或显示器变化后，确保团子还在屏幕上。
    fn ensure_on_screen(&mut self) {
        if !point_on_screen(self.pos.0 + self.w / 2, self.pos.1 + self.h / 2) {
            let p = self.default_pos();
            self.move_to(p);
        }
    }

    fn set_visible(&mut self, v: bool) {
        self.visible = v;
        unsafe { ShowWindow(self.pet, if v { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
        if v {
            self.after_state_change();
        } else {
            self.stop_anim();
            self.update_z();
        }
    }

    /// 托盘左键 / 再次运行 exe：把团子叫出来，在最前面待几秒。
    fn summon(&mut self) {
        if !self.visible {
            self.set_visible(true);
        }
        self.summon_until = Some(Instant::now() + Duration::from_secs(4));
        let text = format!("我在这儿呢~\n内存 {:.0}%", self.mem.percent());
        self.set_message(text, 3.5);
        self.after_state_change();
    }

    // ------------------------------------------------------------------
    // 鼠标
    // ------------------------------------------------------------------

    fn on_mouse_move(&mut self) {
        let now = Instant::now();
        if !self.hover {
            self.hover = true;
            self.hover_left = None;
            // 团子还没完全变回去时又回来了，就不用再等
            self.hover_since = if self.morph > 0.5 { now.checked_sub(HOVER_DELAY).unwrap_or(now) } else { now };
            let mut tme = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.pet,
                dwHoverTime: 0,
            };
            unsafe { TrackMouseEvent(&mut tme) };
            self.ensure_anim();
        }
        let mut new_pos = None;
        if let Some(p) = &mut self.press {
            let c = cursor_pos();
            let (dx, dy) = (c.x - p.cursor.x, c.y - p.cursor.y);
            let (tx, ty) = unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
            if !p.dragging && (dx.abs() > tx || dy.abs() > ty) {
                p.dragging = true;
            }
            if p.dragging {
                new_pos = Some((p.win.0 + dx, p.win.1 + dy));
            }
        }
        if let Some(np) = new_pos {
            self.move_to(np);
        }
    }

    fn on_mouse_leave(&mut self) {
        self.hover = false;
        self.hover_left = Some(Instant::now());
        self.ensure_anim();
    }

    fn on_lbutton_down(&mut self) {
        self.press = Some(Press { cursor: cursor_pos(), win: self.pos, dragging: false });
        unsafe { SetCapture(self.pet) };
    }

    fn on_lbutton_up(&mut self) {
        let Some(p) = self.press.take() else { return };
        unsafe { ReleaseCapture() };
        if p.dragging {
            self.cfg.pos = Some(self.pos);
            self.save_cfg();
        } else {
            self.poke();
        }
    }

    fn on_dblclk(&mut self) {
        self.press = None;
        self.start_clean(CleanKind::Light, false);
    }

    fn rand(&mut self, n: usize) -> usize {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x as usize) % n.max(1)
    }

    fn poke(&mut self) {
        self.poke_at = Some(Instant::now());
        let words: &[&str] = match self.level {
            Level::Normal => &["嘿嘿", "好痒呀~", "别戳啦~", "干嘛呀", "我在呢~", "噗叽"],
            Level::Warn => &["有点挤…", "呼…有点热", "内存有点多了"],
            Level::Alert => &["好撑…", "快喘不过气啦", "双击我清理吧"],
        };
        let w = words[self.rand(words.len())];
        // 正在清理时只做动作，不盖掉「清理中…」的提示
        if self.cleaning.is_none() {
            self.set_message(w, 1.3);
        } else {
            self.ensure_anim();
        }
    }

    // ------------------------------------------------------------------
    // 清理
    // ------------------------------------------------------------------

    pub fn start_clean(&mut self, kind: CleanKind, auto: bool) {
        if self.cleaning.is_some() {
            // 已经在清理了，保持「清理中…」的提示就好
            return;
        }
        if kind == CleanKind::Deep && !self.is_admin {
            self.set_message("深度清理需要\n管理员权限哦", 3.0);
            return;
        }
        // 轻度清理时跳过正在使用的程序，免得它卡一下
        let skip = if kind == CleanKind::Light {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
            pid
        } else {
            0
        };
        let hub = self.hub as isize;
        let before = memory::sample().avail;
        let spawned = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let ok = match kind {
                    CleanKind::Light => {
                        memory::clean_light(skip);
                        true
                    }
                    CleanKind::Deep => memory::clean_deep(),
                };
                std::thread::sleep(Duration::from_millis(700));
                let freed_mb = memory::sample().avail.saturating_sub(before) / (1024 * 1024);
                let wp = kind as usize | ((ok as usize) << 8);
                unsafe { PostMessageW(hub as HWND, WM_APP_CLEANED, wp, freed_mb as isize) };
            })
            .is_ok();
        if spawned {
            self.cleaning = Some(kind);
            self.clean_auto = auto;
            self.happy_until = None;
            let text = match (kind, auto) {
                (CleanKind::Deep, _) => "深度清理中…",
                (_, true) => "内存告急！\n自动清理中…",
                _ => "清理中…",
            };
            self.set_message(text, 60.0);
            self.after_state_change();
        }
    }

    fn on_cleaned(&mut self, wp: WPARAM, lp: LPARAM) {
        let kind = if wp & 0xFF == CleanKind::Deep as usize { CleanKind::Deep } else { CleanKind::Light };
        let ok = (wp >> 8) & 1 == 1;
        let freed = lp.max(0) as u64 * 1024 * 1024;
        self.cleaning = None;
        let now = Instant::now();
        if !ok {
            self.set_message("这次没清理成功…", 3.0);
        } else if freed >= 10 * 1024 * 1024 {
            let head = match (kind, self.clean_auto) {
                (CleanKind::Deep, _) => "深度清理完成",
                (_, true) => "自动清理完成",
                _ => "清理完成",
            };
            self.set_message(format!("{}\n腾出了 {}", head, fmt_bytes(freed)), 3.5);
            self.happy_until = Some(now + Duration::from_millis(3500));
        } else {
            self.set_message("已经很清爽啦~", 2.5);
            self.happy_until = Some(now + Duration::from_millis(2500));
        }
        self.mem = memory::sample();
        self.tray.set_tip(&self.tip_text());
        self.after_state_change();
    }

    fn after_kill(&mut self, name: &str, ok: u32, fail: u32) {
        if fail == 0 && ok > 0 {
            self.set_message(format!("结束了\n{}", name), 3.0);
            self.happy_until = Some(Instant::now() + Duration::from_secs(3));
        } else if ok == 0 && fail == 0 {
            self.set_message("它已经不在啦", 2.0);
        } else {
            self.set_message(format!("有 {} 个\n没能结束", fail), 3.0);
        }
        self.after_state_change();
    }

    // ------------------------------------------------------------------
    // 设置
    // ------------------------------------------------------------------

    fn save_cfg(&mut self) {
        match self.cfg.save() {
            Some(m) => self.cfg_mtime = Some(m),
            None => self.set_message("设置没能\n保存下来…", 3.0),
        }
    }

    pub fn rename(&mut self, name: &str) {
        self.cfg.folder_name = name.to_string();
        self.cfg.sanitize();
        self.save_cfg();
        self.label = None;
        self.after_state_change();
    }

    fn apply_autostart(&mut self, enable: bool, quiet: bool) {
        self.autostart_quiet = quiet;
        let hub = self.hub as isize;
        let _ = std::thread::Builder::new().stack_size(256 * 1024).spawn(move || {
            let ok = system::set_autostart(enable);
            unsafe { PostMessageW(hub as HWND, WM_APP_AUTOSTART, enable as usize, ok as isize) };
        });
    }

    fn on_autostart_result(&mut self, enable: bool, ok: bool) {
        if !ok {
            self.set_message(if enable { "开机自启没设置成功…" } else { "取消开机自启失败了…" }, 3.0);
        } else if !self.autostart_quiet {
            self.set_message(if enable { "开机自启已开启" } else { "开机自启已关闭" }, 2.0);
        }
    }

    /// 用户在记事本里改了设置文件，就重新读取。
    fn check_config_file(&mut self) {
        let m = config::mtime();
        if m.is_none() || m == self.cfg_mtime {
            return;
        }
        self.cfg_mtime = m;
        let (new, _) = Config::load();
        if new == self.cfg {
            return;
        }
        let old = std::mem::replace(&mut self.cfg, new);
        if self.cfg.autostart != old.autostart {
            self.apply_autostart(self.cfg.autostart, false);
        }
        if self.cfg.interval_sec != old.interval_sec {
            unsafe { SetTimer(self.hub, TIMER_SAMPLE, self.cfg.interval_sec * 1000, None) };
        }
        if self.cfg.pos != old.pos {
            let p = self.valid_pos(self.cfg.pos);
            self.move_to(p);
        }
        self.level = next_level(self.level, self.mem.percent(), self.cfg.warn, self.cfg.alert);
        self.label = None;
        self.set_message("设置已更新~", 2.0);
    }

    fn on_command(&mut self, cmd: u32) {
        let now = Instant::now();
        match cmd {
            c if (CMD_PRESET..CMD_PRESET + PRESETS.len() as u32).contains(&c) => {
                let (w, a) = PRESETS[(c - CMD_PRESET) as usize];
                self.cfg.warn = w;
                self.cfg.alert = a;
                self.save_cfg();
                self.level = next_level(Level::Normal, self.mem.percent(), w, a);
                self.set_message(format!("提醒 {}%\n告急 {}%", w, a), 2.5);
            }
            CMD_AUTO_CLEAN => {
                self.cfg.auto_clean = !self.cfg.auto_clean;
                self.save_cfg();
                self.set_message(if self.cfg.auto_clean { "告急时我会\n自动清理" } else { "不再自动清理啦" }, 2.5);
            }
            CMD_POPUP => {
                self.cfg.popup_on_alert = !self.cfg.popup_on_alert;
                self.save_cfg();
                self.set_message(if self.cfg.popup_on_alert { "告急时我会\n跳出来提醒你" } else { "告急时我就\n原地着急" }, 2.5);
            }
            CMD_AUTOSTART => {
                self.cfg.autostart = !self.cfg.autostart;
                self.save_cfg();
                self.apply_autostart(self.cfg.autostart, false);
            }
            CMD_OPEN_CFG => {
                self.save_cfg();
                system::open_in_notepad(&config::path());
            }
            CMD_RESET_POS => {
                self.cfg.pos = None;
                self.save_cfg();
                let p = self.default_pos();
                self.move_to(p);
                self.set_message("我回来啦~", 2.0);
            }
            CMD_SNOOZE => {
                if self.snoozed(now) {
                    self.snooze_until = None;
                    self.set_message("安静模式结束啦", 2.0);
                } else {
                    self.snooze_until = Some(now + SNOOZE);
                    self.set_message("好的~ 30 分钟\n不打扰你", 2.5);
                }
                self.tray.set_tip(&self.tip_text());
            }
            CMD_TOGGLE_VIS => {
                let v = !self.visible;
                self.set_visible(v);
            }
            _ => {}
        }
        self.after_state_change();
    }

    fn build_menu(&mut self) -> (HMENU, Vec<ProcGroup>) {
        self.menu_open = true;
        self.mem = memory::sample();
        let groups = memory::top_groups(8);
        let now = Instant::now();
        let check = |b: bool| if b { MF_CHECKED } else { 0 };
        unsafe {
            let add = |m: HMENU, flags: u32, id: u32, text: &str| {
                let t = wide(text);
                AppendMenuW(m, flags, id as usize, t.as_ptr());
            };
            let sep = |m: HMENU| {
                AppendMenuW(m, MF_SEPARATOR, 0, std::ptr::null());
            };
            let menu = CreatePopupMenu();
            let info = format!("内存 {:.0}% · 已用 {:.1} / {:.1} GB", self.mem.percent(), gb(self.mem.used()), gb(self.mem.total));
            add(menu, MF_STRING | MF_GRAYED, 0, &info);
            sep(menu);
            add(menu, MF_STRING, CMD_CLEAN_LIGHT, "轻度清理（双击团子也可以）");
            add(menu, MF_STRING | if self.is_admin { 0 } else { MF_GRAYED }, CMD_CLEAN_DEEP, "深度清理");

            let procs = CreatePopupMenu();
            add(procs, MF_STRING | MF_GRAYED, 0, "点一下可以结束它（会先问你）");
            sep(procs);
            if groups.is_empty() {
                add(procs, MF_STRING | MF_GRAYED, 0, "（暂时读不到）");
            }
            for (i, g) in groups.iter().enumerate() {
                let count = if g.count > 1 { format!(" ×{}", g.count) } else { String::new() };
                let tag = if g.protected { "（系统）" } else { "" };
                let text = format!("{}{}{}\t{}", g.name, count, tag, fmt_bytes(g.bytes));
                add(procs, MF_STRING | if g.protected { MF_GRAYED } else { 0 }, CMD_PROC + i as u32, &text);
            }
            let t = wide("吃内存大户");
            AppendMenuW(menu, MF_POPUP, procs as usize, t.as_ptr());
            sep(menu);

            let set = CreatePopupMenu();
            let mut matched = false;
            for (i, &(w, a)) in PRESETS.iter().enumerate() {
                let on = self.cfg.warn == w && self.cfg.alert == a;
                matched |= on;
                add(set, MF_STRING | check(on), CMD_PRESET + i as u32, &format!("{}% 提醒 / {}% 告急", w, a));
            }
            if !matched {
                add(set, MF_STRING | MF_CHECKED | MF_GRAYED, 0, &format!("自定义：{}% 提醒 / {}% 告急", self.cfg.warn, self.cfg.alert));
            }
            sep(set);
            add(set, MF_STRING | check(self.cfg.auto_clean), CMD_AUTO_CLEAN, "告急时自动轻度清理");
            add(set, MF_STRING | check(self.cfg.popup_on_alert), CMD_POPUP, "告急时跳到最前面");
            add(set, MF_STRING | check(self.cfg.autostart), CMD_AUTOSTART, "开机自动启动");
            sep(set);
            add(set, MF_STRING, CMD_RENAME, "给伪装文件夹改名…");
            add(set, MF_STRING, CMD_OPEN_CFG, "打开设置文件…");
            add(set, MF_STRING, CMD_RESET_POS, "团子回到默认位置");
            let t = wide("设置");
            AppendMenuW(menu, MF_POPUP, set as usize, t.as_ptr());

            add(menu, MF_STRING, CMD_SNOOZE, if self.snoozed(now) { "结束安静模式" } else { "安静 30 分钟（告急时不跳出来）" });
            add(menu, MF_STRING, CMD_TOGGLE_VIS, if self.visible { "隐藏团子" } else { "显示团子" });
            sep(menu);
            add(menu, MF_STRING, CMD_EXIT, "退出");
            self.ensure_anim();
            (menu, groups)
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old_bmp);
            DeleteObject(self.dib);
            DeleteDC(self.mem_dc);
            ReleaseDC(std::ptr::null_mut(), self.screen_dc);
        }
    }
}

// ----------------------------------------------------------------------
// 菜单与命令（这些会弹出模态界面，必须在借用之外执行）
// ----------------------------------------------------------------------

fn show_menu() {
    let Some((menu, groups, hub)) = with_app(|a| {
        let (m, g) = a.build_menu();
        (m, g, a.hub)
    }) else {
        return;
    };
    let cmd = unsafe {
        SetForegroundWindow(hub);
        let pt = cursor_pos();
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hub, std::ptr::null());
        PostMessageW(hub, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd.max(0) as u32
    };
    with_app(|a| a.menu_open = false);
    handle_command(cmd, &groups);
}

fn handle_command(cmd: u32, groups: &[ProcGroup]) {
    match cmd {
        0 => {}
        CMD_CLEAN_LIGHT => {
            with_app(|a| a.start_clean(CleanKind::Light, false));
        }
        CMD_CLEAN_DEEP => {
            with_app(|a| a.start_clean(CleanKind::Deep, false));
        }
        CMD_RENAME => {
            if let Some((hub, name, s)) = with_app(|a| (a.hub, a.cfg.folder_name.clone(), a.scale)) {
                dialog::open(hub, &name, s);
            }
        }
        CMD_EXIT => {
            if let Some(hub) = with_app(|a| {
                a.shutdown();
                a.hub
            }) {
                unsafe { DestroyWindow(hub) };
            }
        }
        c if c >= CMD_PROC && ((c - CMD_PROC) as usize) < groups.len() => confirm_kill(&groups[(c - CMD_PROC) as usize]),
        c => {
            with_app(|a| a.on_command(c));
        }
    }
}

fn confirm_kill(g: &ProcGroup) {
    let hub = HUB.load(Ordering::Relaxed);
    let count = if g.count > 1 { format!("一共 {} 个进程", g.count) } else { "1 个进程".to_string() };
    let text =
        format!("要结束「{}」吗？\n\n{}，大约占用 {} 内存。\n如果里面有还没保存的内容，会丢失哦。", g.name, count, fmt_bytes(g.bytes));
    let r = message_box(hub, &text, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND);
    if r == IDYES {
        let (ok, fail) = memory::kill_group(&g.name);
        with_app(|a| a.after_kill(&g.name, ok, fail));
    }
}

// ----------------------------------------------------------------------
// 窗口过程
// ----------------------------------------------------------------------

unsafe extern "system" fn pet_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_WINDOWPOSCHANGING => {
            // 牢牢守住自己的层级：平时在最底层，需要时在最顶层
            let pos = &mut *(lp as *mut WINDOWPOS);
            if pos.flags & SWP_NOZORDER == 0 {
                pos.hwndInsertAfter = if TOP_MODE.load(Ordering::Relaxed) { HWND_TOPMOST } else { HWND_BOTTOM };
            }
        }
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_MOUSEMOVE => {
            with_app(|a| a.on_mouse_move());
            return 0;
        }
        WM_MOUSELEAVE => {
            with_app(|a| a.on_mouse_leave());
            return 0;
        }
        WM_LBUTTONDOWN => {
            with_app(|a| a.on_lbutton_down());
            return 0;
        }
        WM_LBUTTONUP => {
            with_app(|a| a.on_lbutton_up());
            return 0;
        }
        WM_LBUTTONDBLCLK => {
            with_app(|a| a.on_dblclk());
            return 0;
        }
        WM_RBUTTONUP => {
            PostMessageW(HUB.load(Ordering::Relaxed), WM_APP_MENU, 0, 0);
            return 0;
        }
        WM_CAPTURECHANGED => {
            if lp as HWND != hwnd {
                with_app(|a| a.press = None);
            }
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe extern "system" fn hub_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            match wp {
                TIMER_SAMPLE => {
                    with_app(|a| a.sample());
                }
                TIMER_ANIM => {
                    with_app(|a| a.tick());
                }
                _ => {}
            }
            return 0;
        }
        WM_TRAY => {
            match loword(lp as usize) {
                WM_LBUTTONUP => {
                    with_app(|a| a.summon());
                }
                WM_RBUTTONUP => show_menu(),
                _ => {}
            }
            return 0;
        }
        WM_APP_MENU => {
            show_menu();
            return 0;
        }
        WM_APP_CLEANED | WM_APP_AUTOSTART => {
            let done = with_app(|a| if msg == WM_APP_CLEANED { a.on_cleaned(wp, lp) } else { a.on_autostart_result(wp != 0, lp != 0) });
            if done.is_none() {
                // 极少数情况下正忙，稍后再处理，免得「清理中」状态卡住
                PostMessageW(hwnd, msg, wp, lp);
            }
            return 0;
        }
        WM_APP_SHOW => {
            with_app(|a| a.summon());
            return 0;
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            with_app(|a| a.ensure_on_screen());
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            return 0;
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            return 0;
        }
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            // 资源管理器重启了，托盘图标要重新放上去
            with_app(|a| a.tray.readd());
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe extern "system" fn win_event_proc(_hook: HWINEVENTHOOK, event: u32, hwnd: HWND, _obj: i32, _child: i32, _thread: u32, _time: u32) {
    if event == EVENT_SYSTEM_FOREGROUND {
        with_app(|a| a.on_foreground(hwnd));
    }
}

// ----------------------------------------------------------------------
// 入口
// ----------------------------------------------------------------------

unsafe fn register_class(name: &[u16], style: u32, proc_: WNDPROC, icon: HICON, icon_sm: HICON) {
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        lpfnWndProc: proc_,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: GetModuleHandleW(std::ptr::null()),
        hIcon: icon,
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: name.as_ptr(),
        hIconSm: icon_sm,
    };
    RegisterClassExW(&wc);
}

pub fn run() {
    unsafe {
        SetProcessDPIAware();
        let args: Vec<String> = std::env::args().collect();

        // 只允许运行一个团子；再次运行时把已有的团子叫出来
        let mutex_name = wide("Local\\TuanziDesktopPet");
        let mut mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class = wide(HUB_CLASS);
            let h = FindWindowW(class.as_ptr(), std::ptr::null());
            if !h.is_null() {
                PostMessageW(h, WM_APP_SHOW, 0, 0);
            }
            return;
        }

        // exe 自带的清单会让系统以管理员身份启动；万一没有生效，这里补请求一次
        let mut admin = system::is_admin();
        if !admin && !args.iter().any(|a| a == "--no-elevate") {
            CloseHandle(mutex);
            if system::relaunch_elevated("--no-elevate") {
                return;
            }
            mutex = CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr());
            admin = system::is_admin();
        }
        if admin {
            for p in ["SeDebugPrivilege", "SeProfileSingleProcessPrivilege", "SeIncreaseQuotaPrivilege"] {
                system::enable_privilege(p);
            }
        }

        let icc = INITCOMMONCONTROLSEX { dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32, dwICC: ICC_STANDARD_CLASSES };
        InitCommonControlsEx(&icc);

        let (cfg, existed) = Config::load();
        let icon = tray::make_icon(GetSystemMetrics(SM_CXICON).max(32));
        let icon_sm = tray::make_icon(GetSystemMetrics(SM_CXSMICON).max(16));
        let (hub_class, pet_class) = (wide(HUB_CLASS), wide(PET_CLASS));
        register_class(&hub_class, 0, Some(hub_proc), icon, icon_sm);
        register_class(&pet_class, CS_DBLCLKS, Some(pet_proc), icon, icon_sm);
        dialog::register(icon, icon_sm);

        let hinst = GetModuleHandleW(std::ptr::null());
        let title = wide("团子");
        let hub = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            hub_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        let pet = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            pet_class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            1,
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hub.is_null() || pet.is_null() {
            message_box(std::ptr::null_mut(), "团子没能启动（创建窗口失败）。", MB_OK | MB_ICONWARNING);
            return;
        }
        HUB.store(hub, Ordering::Relaxed);

        // 我们以管理员身份运行，要显式允许资源管理器（普通权限）发来的托盘消息
        let tb_name = wide("TaskbarCreated");
        let taskbar_created = RegisterWindowMessageW(tb_name.as_ptr());
        TASKBAR_CREATED.store(taskbar_created, Ordering::Relaxed);
        for m in [taskbar_created, WM_TRAY, WM_APP_SHOW] {
            if m != 0 {
                ChangeWindowMessageFilterEx(hub, m, MSGFLT_ALLOW, std::ptr::null_mut());
            }
        }

        let app = App::new(hub, pet, cfg, admin);
        APP.with(|c| *c.borrow_mut() = Some(app));
        let by_user = !args.iter().any(|a| a == "--autostart");
        with_app(|a| a.start(!existed, by_user));

        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(win_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            let dlg = dialog::hwnd();
            if !dlg.is_null() && IsDialogMessageW(dlg, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if !hook.is_null() {
            UnhookWinEvent(hook);
        }
        let app = APP.with(|c| c.borrow_mut().take());
        drop(app);
        DestroyWindow(pet);
        DestroyIcon(icon);
        DestroyIcon(icon_sm);
        CloseHandle(mutex);
    }
}
