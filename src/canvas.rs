//! 纯软件绘图：在「预乘 Alpha 的 BGRA」缓冲区上，用带符号距离场（SDF）画抗锯齿图形。
//!
//! 这里不调用任何系统 API，所以 build.rs 生成 exe 图标、examples/preview.rs
//! 在 Linux 上预览造型时都能直接复用。

#![allow(dead_code)]

use std::f32::consts::PI;

/// 颜色（0..1，非预乘 Alpha）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

pub fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}

impl Rgba {
    pub fn alpha(self, a: f32) -> Rgba {
        Rgba { a: self.a * a, ..self }
    }

    pub fn mix(self, o: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        Rgba {
            r: self.r + (o.r - self.r) * t,
            g: self.g + (o.g - self.g) * t,
            b: self.b + (o.b - self.b) * t,
            a: self.a + (o.a - self.a) * t,
        }
    }
}

pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };

/// 填充方式：纯色，或沿局部 y 方向的线性渐变。
#[derive(Clone, Copy, Debug)]
pub enum Paint {
    Solid(Rgba),
    VGrad { top: Rgba, bottom: Rgba, y0: f32, y1: f32 },
}

impl From<Rgba> for Paint {
    fn from(c: Rgba) -> Paint {
        Paint::Solid(c)
    }
}

/// 一块绘图缓冲区：BGRA、预乘 Alpha、从上到下逐行排列（和 32 位 top-down DIB 一致）。
pub struct Canvas<'a> {
    pub w: i32,
    pub h: i32,
    pub px: &'a mut [u8],
}

impl<'a> Canvas<'a> {
    pub fn new(w: i32, h: i32, px: &'a mut [u8]) -> Canvas<'a> {
        assert!(px.len() >= (w * h * 4) as usize);
        Canvas { w, h, px }
    }

    /// 用同一个预乘像素值填满整块画布。
    pub fn clear(&mut self, bgra: [u8; 4]) {
        for p in self.px.chunks_exact_mut(4) {
            p.copy_from_slice(&bgra);
        }
    }

    /// 以覆盖率 `cov`（0..1）把颜色 `c` 叠加到一个像素上（source-over）。
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgba, cov: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let a = (c.a * cov).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let p = &mut self.px[i..i + 4];
        let inv = 1.0 - a;
        p[0] = (c.b * a * 255.0 + p[0] as f32 * inv + 0.5) as u8;
        p[1] = (c.g * a * 255.0 + p[1] as f32 * inv + 0.5) as u8;
        p[2] = (c.r * a * 255.0 + p[2] as f32 * inv + 0.5) as u8;
        p[3] = (a * 255.0 + p[3] as f32 * inv + 0.5) as u8;
    }

    /// 在包围盒 `bbox`（像素坐标）内，按 SDF 填充（`sdf` 返回到边界的像素距离，内部为负）。
    pub fn fill(&mut self, bbox: [f32; 4], sdf: impl Fn(f32, f32) -> f32, paint: impl Fn(f32, f32) -> Rgba) {
        let x0 = (bbox[0].floor() as i32 - 1).max(0);
        let y0 = (bbox[1].floor() as i32 - 1).max(0);
        let x1 = (bbox[2].ceil() as i32 + 1).min(self.w);
        let y1 = (bbox[3].ceil() as i32 + 1).min(self.h);
        for y in y0..y1 {
            let fy = y as f32 + 0.5;
            for x in x0..x1 {
                let fx = x as f32 + 0.5;
                let cov = (0.5 - sdf(fx, fy)).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let c = paint(fx, fy);
                    self.blend(x, y, c, cov);
                }
            }
        }
    }

    /// 把一张 8 位覆盖率蒙版（如文字）以颜色 `c` 叠加到 (ox, oy)。
    pub fn blend_mask(&mut self, ox: i32, oy: i32, mw: i32, mh: i32, mask: &[u8], c: Rgba) {
        for y in 0..mh {
            for x in 0..mw {
                let m = mask[(y * mw + x) as usize];
                if m != 0 {
                    self.blend(ox + x, oy + y, c, m as f32 / 255.0);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 基础 SDF（像素空间）
// ---------------------------------------------------------------------------

#[inline]
fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// 平滑并集。
#[inline]
pub fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}

/// 平滑交集。
#[inline]
pub fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}

/// 椭圆（近似距离，足够画小图标）。
#[inline]
pub fn sd_ellipse(px: f32, py: f32, rx: f32, ry: f32) -> f32 {
    let k0 = len(px / rx, py / ry);
    let k1 = len(px / (rx * rx), py / (ry * ry));
    if k1 < 1e-6 {
        return -rx.min(ry);
    }
    k0 * (k0 - 1.0) / k1
}

/// 圆角矩形，(hw, hh) 为半宽半高。
#[inline]
pub fn sd_rrect(px: f32, py: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let r = r.min(hw).min(hh);
    let qx = px.abs() - hw + r;
    let qy = py.abs() - hh + r;
    len(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

/// 线段（两端圆头），hw 为半线宽。
#[inline]
pub fn sd_segment(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, hw: f32) -> f32 {
    let (pax, pay) = (px - ax, py - ay);
    let (bax, bay) = (bx - ax, by - ay);
    let bb = bax * bax + bay * bay;
    let h = if bb > 1e-9 { ((pax * bax + pay * bay) / bb).clamp(0.0, 1.0) } else { 0.0 };
    len(pax - bax * h, pay - bay * h) - hw
}

/// 圆弧：角度按屏幕坐标（y 向下）用 atan2 计，从 a0 顺时针扫到 a1。
#[inline]
pub fn sd_arc(px: f32, py: f32, r: f32, a0: f32, a1: f32, hw: f32) -> f32 {
    let mut ang = py.atan2(px);
    while ang < a0 {
        ang += 2.0 * PI;
    }
    while ang >= a0 + 2.0 * PI {
        ang -= 2.0 * PI;
    }
    if ang <= a1 {
        ((len(px, py) - r).abs()) - hw
    } else {
        let d0 = len(px - r * a0.cos(), py - r * a0.sin());
        let d1 = len(px - r * a1.cos(), py - r * a1.sin());
        d0.min(d1) - hw
    }
}

/// 菱形，(bx, by) 为半对角线。
#[inline]
pub fn sd_rhombus(px: f32, py: f32, bx: f32, by: f32) -> f32 {
    let (px, py) = (px.abs(), py.abs());
    let ndot = (bx - 2.0 * px) * bx - (by - 2.0 * py) * by;
    let h = (ndot / (bx * bx + by * by)).clamp(-1.0, 1.0);
    let d = len(px - 0.5 * bx * (1.0 - h), py - 0.5 * by * (1.0 + h));
    let s = px * by + py * bx - bx * by;
    if s < 0.0 {
        -d
    } else {
        d
    }
}

/// 任意简单多边形（这里用于四边形：Win7 文件夹的斜前片、竖着的纸张）。
#[inline]
pub fn sd_polygon(px: f32, py: f32, v: &[(f32, f32)]) -> f32 {
    let n = v.len();
    let mut d = (px - v[0].0).powi(2) + (py - v[0].1).powi(2);
    let mut s = 1.0f32;
    let mut j = n - 1;
    for i in 0..n {
        let (ex, ey) = (v[j].0 - v[i].0, v[j].1 - v[i].1);
        let (wx, wy) = (px - v[i].0, py - v[i].1);
        let ee = ex * ex + ey * ey;
        let h = if ee > 1e-9 { ((wx * ex + wy * ey) / ee).clamp(0.0, 1.0) } else { 0.0 };
        let (bx, by) = (wx - ex * h, wy - ey * h);
        d = d.min(bx * bx + by * by);
        let c1 = py >= v[i].1;
        let c2 = py < v[j].1;
        let c3 = ex * wy > ey * wx;
        if (c1 && c2 && c3) || (!c1 && !c2 && !c3) {
            s = -s;
        }
        j = i;
    }
    s * d.sqrt()
}

// ---------------------------------------------------------------------------
// 带变换的画笔：用「设计单位 dp」作画，最终乘以 DPI 缩放落到像素上。
// ---------------------------------------------------------------------------

/// 画笔：局部坐标 (x, y) → 像素坐标 ((tx + x·kx)·s, (ty + y·ky)·s)。
pub struct Pen<'c, 'a> {
    pub cv: &'c mut Canvas<'a>,
    /// dp → 像素
    pub s: f32,
    pub tx: f32,
    pub ty: f32,
    pub kx: f32,
    pub ky: f32,
    /// 整体不透明度
    pub opacity: f32,
}

impl<'c, 'a> Pen<'c, 'a> {
    pub fn new(cv: &'c mut Canvas<'a>, s: f32) -> Pen<'c, 'a> {
        Pen { cv, s, tx: 0.0, ty: 0.0, kx: 1.0, ky: 1.0, opacity: 1.0 }
    }

    /// 设置局部坐标系：原点放在 (tx, ty)（dp），并按 (kx, ky) 缩放。
    pub fn set(&mut self, tx: f32, ty: f32, kx: f32, ky: f32) {
        self.tx = tx;
        self.ty = ty;
        self.kx = kx;
        self.ky = ky;
    }

    #[inline]
    fn mx(&self, x: f32) -> f32 {
        (self.tx + x * self.kx) * self.s
    }

    #[inline]
    fn my(&self, y: f32) -> f32 {
        (self.ty + y * self.ky) * self.s
    }

    #[inline]
    fn mk(&self) -> f32 {
        (self.kx * self.ky).abs().sqrt() * self.s
    }

    fn paint_fn(&self, paint: Paint) -> impl Fn(f32, f32) -> Rgba {
        let op = self.opacity;
        let (sy0, sy1) = match paint {
            Paint::VGrad { y0, y1, .. } => (self.my(y0), self.my(y1)),
            _ => (0.0, 1.0),
        };
        move |_x: f32, y: f32| match paint {
            Paint::Solid(c) => c.alpha(op),
            Paint::VGrad { top, bottom, .. } => {
                let t = if (sy1 - sy0).abs() > 1e-3 { (y - sy0) / (sy1 - sy0) } else { 0.0 };
                top.mix(bottom, t).alpha(op)
            }
        }
    }

    /// 椭圆；`grow` 为外扩的 dp（用来画描边底）。
    pub fn ellipse_g(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, grow: f32, paint: impl Into<Paint>) {
        let (cx, cy) = (self.mx(cx), self.my(cy));
        let rx = (rx * self.kx * self.s).abs().max(0.01);
        let ry = (ry * self.ky * self.s).abs().max(0.01);
        let g = grow * self.mk();
        let f = self.paint_fn(paint.into());
        self.cv.fill([cx - rx - g, cy - ry - g, cx + rx + g, cy + ry + g], |x, y| sd_ellipse(x - cx, y - cy, rx, ry) - g, f);
    }

    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, paint: impl Into<Paint>) {
        self.ellipse_g(cx, cy, rx, ry, 0.0, paint);
    }

    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, paint: impl Into<Paint>) {
        self.ellipse_g(cx, cy, r, r, 0.0, paint);
    }

    /// 只保留椭圆中 y ≥ cut_y 的部分（用来画张开的笑嘴）。
    pub fn ellipse_lower(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, cut_y: f32, paint: impl Into<Paint>) {
        let (cx, cy, cut) = (self.mx(cx), self.my(cy), self.my(cut_y));
        let rx = (rx * self.kx * self.s).abs().max(0.01);
        let ry = (ry * self.ky * self.s).abs().max(0.01);
        let f = self.paint_fn(paint.into());
        self.cv.fill([cx - rx, cut.min(cy + ry), cx + rx, cy + ry], |x, y| sd_ellipse(x - cx, y - cy, rx, ry).max(cut - y), f);
    }

    /// 圆角矩形（左上 x0,y0 到右下 x1,y1）；`grow` 为外扩 dp。
    pub fn rrect_g(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, grow: f32, paint: impl Into<Paint>) {
        let (ax, ay, bx, by) = (self.mx(x0), self.my(y0), self.mx(x1), self.my(y1));
        let (cx, cy) = ((ax + bx) * 0.5, (ay + by) * 0.5);
        let (hw, hh) = (((bx - ax) * 0.5).abs(), ((by - ay) * 0.5).abs());
        let r = r * self.kx.abs().min(self.ky.abs()) * self.s;
        let g = grow * self.mk();
        let f = self.paint_fn(paint.into());
        self.cv.fill([cx - hw - g, cy - hh - g, cx + hw + g, cy + hh + g], |x, y| sd_rrect(x - cx, y - cy, hw, hh, r) - g, f);
    }

    pub fn rrect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, r: f32, paint: impl Into<Paint>) {
        self.rrect_g(x0, y0, x1, y1, r, 0.0, paint);
    }

    /// 四边形（顶点按顺序给出）；`grow` 为外扩 dp，`round` 为圆角 dp（向内收再外扩）。
    pub fn quad(&mut self, pts: [(f32, f32); 4], round: f32, grow: f32, paint: impl Into<Paint>) {
        let v: [(f32, f32); 4] = [
            (self.mx(pts[0].0), self.my(pts[0].1)),
            (self.mx(pts[1].0), self.my(pts[1].1)),
            (self.mx(pts[2].0), self.my(pts[2].1)),
            (self.mx(pts[3].0), self.my(pts[3].1)),
        ];
        let x0 = v.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        let x1 = v.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let y0 = v.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let y1 = v.iter().map(|p| p.1).fold(f32::MIN, f32::max);
        let k = self.mk();
        let (g, r) = (grow * k, round * k);
        // 先把多边形向内收 r，再整体外扩 r，得到圆角
        let cx = (x0 + x1) * 0.5;
        let cy = (y0 + y1) * 0.5;
        let shrink = |p: (f32, f32)| {
            let (dx, dy) = (p.0 - cx, p.1 - cy);
            let l = (dx * dx + dy * dy).sqrt().max(1e-3);
            (p.0 - dx / l * r, p.1 - dy / l * r)
        };
        let vi = [shrink(v[0]), shrink(v[1]), shrink(v[2]), shrink(v[3])];
        let f = self.paint_fn(paint.into());
        self.cv.fill([x0 - g, y0 - g, x1 + g, y1 + g], |x, y| sd_polygon(x, y, &vi) - r - g, f);
    }

    /// 圆头线段，w 为线宽（dp）。
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, paint: impl Into<Paint>) {
        let (ax, ay, bx, by) = (self.mx(x0), self.my(y0), self.mx(x1), self.my(y1));
        let hw = (w * 0.5 * self.mk()).max(0.35);
        let f = self.paint_fn(paint.into());
        self.cv.fill([ax.min(bx) - hw, ay.min(by) - hw, ax.max(bx) + hw, ay.max(by) + hw], |x, y| sd_segment(x, y, ax, ay, bx, by, hw), f);
    }

    /// 折线（如「>」「<」眼睛）。
    pub fn polyline(&mut self, pts: &[(f32, f32)], w: f32, paint: impl Into<Paint> + Copy) {
        for p in pts.windows(2) {
            self.line(p[0].0, p[0].1, p[1].0, p[1].1, w, paint);
        }
    }

    /// 圆弧，角度见 [`sd_arc`]。
    pub fn arc(&mut self, cx: f32, cy: f32, r: f32, a0: f32, a1: f32, w: f32, paint: impl Into<Paint>) {
        let (cx, cy) = (self.mx(cx), self.my(cy));
        let r = r * self.mk();
        let hw = (w * 0.5 * self.mk()).max(0.35);
        let f = self.paint_fn(paint.into());
        self.cv.fill([cx - r - hw, cy - r - hw, cx + r + hw, cy + r + hw], |x, y| sd_arc(x - cx, y - cy, r, a0, a1, hw), f);
    }

    /// 团子的身体：椭圆和地面平滑相交，得到底部微平的「麻薯」形。
    /// (gx, gy) 是底边中心，rx/ry 为椭圆半径；`grow` 为外扩 dp。
    pub fn mochi(&mut self, gx: f32, gy: f32, rx: f32, ry: f32, grow: f32, paint: impl Into<Paint>) {
        let gxp = self.mx(gx);
        let gyp = self.my(gy);
        let rxp = (rx * self.kx * self.s).abs().max(0.01);
        let ryp = (ry * self.ky * self.s).abs().max(0.01);
        let cyp = gyp - ryp * 0.92;
        let k = ryp * 0.2;
        let g = grow * self.mk();
        let f = self.paint_fn(paint.into());
        self.cv.fill(
            [gxp - rxp - g, cyp - ryp - g, gxp + rxp + g, gyp + g],
            |x, y| smax(sd_ellipse(x - gxp, y - cyp, rxp, ryp), y - gyp, k) - g,
            f,
        );
    }

    /// 汗滴：下圆上尖。
    pub fn drop(&mut self, cx: f32, cy: f32, r: f32, paint: impl Into<Paint>) {
        let (cx, cy) = (self.mx(cx), self.my(cy));
        let r = r * self.mk();
        let tip = cy - r * 2.1;
        let f = self.paint_fn(paint.into());
        self.cv.fill(
            [cx - r, tip - 1.0, cx + r, cy + r],
            |x, y| smin(len(x - cx, y - cy) - r, sd_segment(x, y, cx, cy, cx, tip, r * 0.08), r * 1.1),
            f,
        );
    }

    /// 四角星星（闪光）。
    pub fn sparkle(&mut self, cx: f32, cy: f32, r: f32, paint: impl Into<Paint>) {
        let (cx, cy) = (self.mx(cx), self.my(cy));
        let r = r * self.mk();
        let f = self.paint_fn(paint.into());
        self.cv.fill(
            [cx - r, cy - r, cx + r, cy + r],
            |x, y| {
                let (dx, dy) = (x - cx, y - cy);
                sd_rhombus(dx, dy, r, r * 0.26).min(sd_rhombus(dx, dy, r * 0.26, r))
            },
            f,
        );
    }
}

// ---------------------------------------------------------------------------
// 缓动函数
// ---------------------------------------------------------------------------

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t
}

/// 末尾略微回弹的缓出，用于「啵」地冒出来。
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.9;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}
