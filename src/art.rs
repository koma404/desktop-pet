//! 团子和伪装文件夹的造型。所有坐标都是 dp（96 DPI 下的像素）。
//!
//! 和 canvas.rs 一样不依赖系统 API，build.rs 画 exe 图标时也会用到。

#![allow(dead_code)]

use crate::canvas::{ease_in, ease_out_back, rgb, Canvas, Paint, Pen, Rgba, BLACK, WHITE};
use std::f32::consts::PI;

/// 整个桌宠窗口的大小（dp），和 Win7 中等桌面图标的格子差不多。
pub const CANVAS_W: f32 = 84.0;
pub const CANVAS_H: f32 = 98.0;
/// 团子/文件夹的「落脚点」（底边中心）。
pub const PIVOT_X: f32 = 42.0;
pub const PIVOT_Y: f32 = 57.0;
/// 图标下方文字区域。
pub const LABEL_TOP: f32 = 59.0;
pub const LABEL_W: f32 = 80.0;

/// 团子身体的椭圆半径。
const RX: f32 = 24.0;
const RY: f32 = 21.0;

fn eye_color() -> Rgba {
    rgb(47, 59, 76)
}

fn mouth_color() -> Rgba {
    rgb(122, 53, 66)
}

fn blush_color() -> Rgba {
    rgb(255, 135, 160)
}

/// 表情。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// 平常：圆眼睛 + 小微笑
    Normal,
    /// 紧张：八字眉 + 小撇嘴
    Worried,
    /// 告急：> < 眼 + 张嘴喘气
    Panic,
    /// 开心：^ ^ 眼 + 张嘴笑
    Happy,
    /// 被戳：> < 眼 + 小圆嘴
    Squeeze,
    /// 正在清理：眯眼鼓腮使劲
    Busy,
}

/// 某一帧团子的样子。
#[derive(Clone, Copy, Debug)]
pub struct Dango {
    /// 时间（秒），驱动呼吸、汗滴、星星
    pub t: f32,
    /// 心情颜色：0 = 平静蓝，1 = 紧张黄，2 = 告急红，可取中间值
    pub mood: f32,
    pub face: Face,
    /// 额外压扁量（被戳时），正值变扁变宽
    pub squash: f32,
    /// 跳起高度（dp）
    pub hop: f32,
    /// 水平抖动（dp）
    pub shake: f32,
    /// 呼吸幅度（0 = 不动）
    pub breathe: f32,
    /// 呼吸周期（秒）
    pub breathe_period: f32,
    pub blink: bool,
    /// 汗滴数量（0~2）
    pub sweat: u8,
    pub sparkles: bool,
}

impl Dango {
    pub fn calm() -> Dango {
        Dango {
            t: 0.0,
            mood: 0.0,
            face: Face::Normal,
            squash: 0.0,
            hop: 0.0,
            shake: 0.0,
            breathe: 0.0,
            breathe_period: 3.0,
            blink: false,
            sweat: 0,
            sparkles: false,
        }
    }
}

/// 根据心情值给出 (身体色, 描边色)。
pub fn mood_colors(mood: f32) -> (Rgba, Rgba) {
    let blue = (rgb(168, 216, 247), rgb(98, 160, 214));
    let yellow = (rgb(249, 220, 120), rgb(206, 160, 52));
    let red = (rgb(246, 150, 140), rgb(206, 88, 80));
    let m = mood.clamp(0.0, 2.0);
    if m <= 1.0 {
        (blue.0.mix(yellow.0, m), blue.1.mix(yellow.1, m))
    } else {
        (yellow.0.mix(red.0, m - 1.0), yellow.1.mix(red.1, m - 1.0))
    }
}

/// 画团子。(gx, gy) 是落脚点，`scale` 是整体缩放（变身动画用），
/// `small` 为小图标模式（五官放大、省略细节）。
pub fn draw_dango(p: &mut Pen, gx: f32, gy: f32, scale: f32, d: &Dango, small: bool) {
    if scale <= 0.001 {
        return;
    }
    let (body, edge) = mood_colors(d.mood);
    let b = if d.breathe > 0.0 { (2.0 * PI * d.t / d.breathe_period).sin() * d.breathe } else { 0.0 };
    let ky = (1.0 + 0.03 * b) * (1.0 - d.squash) * scale;
    let kx = (1.0 - 0.022 * b) * (1.0 + d.squash * 0.85) * scale;
    let ox = gx + d.shake;

    // 地上的影子：跳起来时变小变淡
    if !small {
        let lift = (d.hop / 6.0).clamp(0.0, 1.0);
        p.set(ox, gy, kx, scale);
        p.ellipse(0.0, 0.3, 19.0 * (1.0 - 0.2 * lift), 3.0, BLACK.alpha(0.2 * (1.0 - 0.5 * lift)));
    }

    // 身体：描边 → 渐变本体 → 高光
    p.set(ox, gy - d.hop * scale, kx, ky);
    p.mochi(0.0, 0.0, RX, RY, if small { 1.6 } else { 1.3 }, edge);
    p.mochi(0.0, 0.0, RX, RY, 0.0, Paint::VGrad { top: body.mix(WHITE, 0.55), bottom: body, y0: -40.0, y1: -8.0 });
    if !small {
        p.ellipse(-10.5, -31.0, 5.6, 3.3, WHITE.alpha(0.6));
        p.circle(-3.8, -34.0, 1.4, WHITE.alpha(0.55));
    }

    let eye = eye_color();
    let es = if small { 1.55 } else { 1.0 }; // 小图标里五官放大
    let ex = 8.5;
    let ey = -20.0;

    // 腮红
    let (bl_rx, bl_a) = if d.face == Face::Busy { (4.4, 0.7) } else { (3.6, 0.5) };
    if !small || d.face != Face::Normal {
        p.ellipse(-14.5, -13.8, bl_rx, 2.3, blush_color().alpha(bl_a));
        p.ellipse(14.5, -13.8, bl_rx, 2.3, blush_color().alpha(bl_a));
    } else {
        p.ellipse(-15.0, -12.5, 4.0, 2.8, blush_color().alpha(0.55));
        p.ellipse(15.0, -12.5, 4.0, 2.8, blush_color().alpha(0.55));
    }

    match d.face {
        Face::Normal | Face::Worried => {
            for sx in [-1.0f32, 1.0] {
                let x = ex * sx;
                if d.blink {
                    p.line(x - 2.5 * es, ey, x + 2.5 * es, ey, 1.6 * es, eye);
                } else {
                    p.ellipse(x, ey - 0.5, 2.5 * es, 3.2 * es, eye);
                    if !small {
                        p.circle(x - 0.8, ey - 1.7, 0.95, WHITE);
                    }
                }
            }
            if d.face == Face::Normal {
                if !small {
                    p.arc(0.0, -16.2, 2.8, 0.18 * PI, 0.82 * PI, 1.5, eye);
                }
            } else {
                // 八字眉：靠里的一端抬高
                p.line(-12.5, -26.2, -6.0, -28.6, 1.4, eye);
                p.line(12.5, -26.2, 6.0, -28.6, 1.4, eye);
                p.arc(0.0, -11.6, 2.7, 1.22 * PI, 1.78 * PI, 1.4, eye);
            }
        }
        Face::Panic | Face::Squeeze => {
            let (w, h) = if d.face == Face::Panic { (4.5, 3.2) } else { (4.0, 2.8) };
            for sx in [-1.0f32, 1.0] {
                let outer = (ex + 2.3) * sx;
                let inner = outer - w * sx;
                p.polyline(&[(outer, ey - h), (inner, ey), (outer, ey + h)], 1.7 * es, eye);
            }
            if d.face == Face::Panic {
                p.ellipse(0.0, -13.2, 2.7, 3.3, mouth_color());
                p.ellipse(0.0, -11.3, 1.6, 1.1, blush_color());
            } else {
                p.ellipse(0.0, -14.2, 1.7, 2.0, mouth_color());
            }
        }
        Face::Happy => {
            for sx in [-1.0f32, 1.0] {
                p.arc(ex * sx, ey + 1.6, 3.1 * es, 1.12 * PI, 1.88 * PI, 1.7 * es, eye);
            }
            p.ellipse_lower(0.0, -16.0, 3.8, 3.6, -16.0, mouth_color());
            p.ellipse(0.0, -13.4, 1.9, 1.0, blush_color());
        }
        Face::Busy => {
            for sx in [-1.0f32, 1.0] {
                p.arc(ex * sx, ey - 2.4, 3.0 * es, 0.2 * PI, 0.8 * PI, 1.6 * es, eye);
            }
            p.ellipse(0.0, -14.0, 1.9, 2.1, mouth_color());
        }
    }

    // 汗滴和星星不跟着身体压扁
    p.set(ox, gy - d.hop * scale, scale, scale);
    let drop_fill = rgb(150, 210, 250);
    let drop_edge = rgb(70, 140, 205);
    let drops: &[(f32, f32, f32)] = match d.sweat {
        0 => &[],
        1 => &[(20.5, -37.0, 0.0)],
        _ => &[(20.5, -37.0, 0.0), (-21.5, -32.0, 0.45)],
    };
    for &(x, y, phase) in drops {
        let s = (d.t * 0.8 + phase).fract();
        let fade = 1.0 - ((s - 0.7) / 0.3).clamp(0.0, 1.0);
        let yy = y + s * 5.0;
        p.opacity = fade;
        p.drop(x, yy, 3.0, drop_edge);
        p.drop(x, yy, 2.3, drop_fill);
        p.circle(x - 0.7, yy - 0.4, 0.6, WHITE.alpha(0.9));
        p.opacity = 1.0;
    }

    if d.sparkles {
        let stars = [(-26.0, -40.0, 4.2, 0.0), (26.5, -33.0, 3.4, 2.1), (27.5, -9.0, 2.6, 4.2), (-27.0, -14.0, 2.4, 1.3)];
        for &(x, y, r, ph) in &stars {
            let k = 0.55 + 0.45 * (d.t * 7.0 + ph).sin();
            p.sparkle(x, y, r * k + 0.8, rgb(232, 168, 40));
            p.sparkle(x, y, r * k, rgb(255, 228, 120));
        }
    }
}

/// 画 Win7 风格的文件夹：没有标签页，浅米黄色，前片是带透视的斜平行四边形，
/// 里面竖着几张白纸；团子躲在纸后面探出小脑袋，两只小爪子扒在前片边上。
pub fn draw_folder(p: &mut Pen, gx: f32, gy: f32, scale: f32, peek_mood: f32) {
    if scale <= 0.001 {
        return;
    }
    p.set(gx, gy, scale, scale);

    // 影子
    p.ellipse(1.0, -3.6, 24.0, 2.3, BLACK.alpha(0.14));

    // 后片：竖直的矩形，左侧和上沿露在前片外面
    let back_line = rgb(183, 146, 62);
    let back = Paint::VGrad { top: rgb(240, 214, 140), bottom: rgb(214, 176, 88), y0: -45.0, y1: -5.0 };
    p.rrect_g(-23.0, -45.0, 18.0, -5.0, 1.6, 0.75, back_line);
    p.rrect(-23.0, -45.0, 18.0, -5.0, 1.6, back);
    // 后片内侧的一道亮边
    p.line(-21.6, -43.4, -21.6, -7.0, 1.0, WHITE.alpha(0.35));

    // 竖着的白纸：两张，略微向右倾斜
    let paper_line = rgb(190, 190, 186);
    let paper = Paint::VGrad { top: rgb(255, 255, 255), bottom: rgb(236, 236, 232), y0: -42.0, y1: -12.0 };
    let sheet_back = [(-15.5, -40.5), (7.5, -42.0), (8.5, -12.0), (-14.5, -11.0)];
    let sheet_front = [(-18.5, -37.5), (4.5, -38.5), (5.0, -10.0), (-17.5, -9.5)];
    p.quad(sheet_back, 0.4, 0.6, paper_line);
    p.quad(sheet_back, 0.4, 0.0, paper);
    p.quad(sheet_front, 0.4, 0.6, paper_line);
    p.quad(sheet_front, 0.4, 0.0, paper);

    // 偷偷探头的团子（在纸和前片之间）
    let (body, edge) = mood_colors(peek_mood);
    let (hx, hy) = (12.5, -29.0);
    p.circle(hx, hy, 8.3, edge);
    p.circle(hx, hy, 7.5, Paint::VGrad { top: body.mix(WHITE, 0.5), bottom: body, y0: hy - 8.0, y1: hy + 2.0 });
    p.ellipse(hx - 3.4, hy - 4.5, 2.2, 1.3, WHITE.alpha(0.6));
    p.circle(hx - 2.6, hy - 2.0, 1.15, eye_color());
    p.circle(hx + 2.6, hy - 2.0, 1.15, eye_color());

    // 前片：斜着的平行四边形，上沿向右错开，像微微张开的文件夹
    let front_line = rgb(196, 160, 72);
    let front = Paint::VGrad { top: rgb(255, 246, 206), bottom: rgb(240, 208, 120), y0: -26.0, y1: -4.0 };
    let flap = [(-17.0, -26.0), (25.0, -26.0), (19.0, -4.5), (-23.0, -4.5)];
    p.quad(flap, 1.2, 0.75, front_line);
    p.quad(flap, 1.2, 0.0, front);
    // 前片上沿的高光
    p.quad([(-15.6, -25.0), (23.4, -25.0), (22.6, -22.4), (-16.3, -22.4)], 0.6, 0.0, WHITE.alpha(0.45));

    // 扒在前片上沿的小爪子
    for dx in [-4.6f32, 4.6] {
        p.ellipse(hx + dx, -26.0, 2.5, 1.9, edge);
        p.ellipse(hx + dx, -26.0, 1.8, 1.25, body.mix(WHITE, 0.25));
    }
}

/// 画整个场景（不含文字）：`morph` 0 = 文件夹，1 = 团子，中间为变身动画。
pub fn draw_scene(p: &mut Pen, morph: f32, d: &Dango) {
    let m = morph.clamp(0.0, 1.0);
    // 前半段文件夹缩小消失，后半段团子「啵」地弹出来
    let folder_k = if m < 0.5 { 1.0 - ease_in(m / 0.5) } else { 0.0 };
    let dango_k = if m > 0.5 { ease_out_back((m - 0.5) / 0.5) } else { 0.0 };
    if folder_k > 0.0 {
        p.opacity = folder_k.sqrt();
        draw_folder(p, PIVOT_X, PIVOT_Y, folder_k, d.mood);
        p.opacity = 1.0;
    }
    if dango_k > 0.0 {
        draw_dango(p, PIVOT_X, PIVOT_Y, dango_k, d, false);
    }
}

/// 画一个方形的团子图标（托盘 / exe 图标）。
pub fn draw_icon(cv: &mut Canvas, size: i32) {
    let s = size as f32 / 52.0;
    let mut p = Pen::new(cv, s);
    let d = Dango::calm();
    draw_dango(&mut p, 26.0, 50.0, 1.0, &d, size <= 24);
}
