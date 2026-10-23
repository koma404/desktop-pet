//! 在任意系统上预览团子的造型：`cargo run --example preview -- out.ppm [缩放]`
//! 会生成一张包含各种状态的拼图（PPM 格式，可用 ImageMagick 转 PNG）。
//! `cargo run --release --example preview -- bench` 则测量每帧的绘制耗时。

#[path = "../src/art.rs"]
mod art;
#[path = "../src/canvas.rs"]
mod canvas;

use art::{draw_icon, draw_scene, Dango, Face, CANVAS_H, CANVAS_W};
use canvas::{Canvas, Pen};

fn bench() {
    for scale in [1.0f32, 1.5] {
        let cw = (CANVAS_W * scale).ceil() as i32;
        let ch = (CANVAS_H * scale).ceil() as i32;
        let mut buf = vec![0u8; (cw * ch * 4) as usize];
        let n = 2000;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            let d =
                Dango { t: i as f32 * 0.05, mood: 2.0, face: Face::Panic, sweat: 2, breathe: 1.3, breathe_period: 0.9, ..Dango::calm() };
            let mut cv = Canvas::new(cw, ch, &mut buf);
            cv.clear([0, 0, 0, 1]);
            let mut pen = Pen::new(&mut cv, scale);
            draw_scene(&mut pen, 1.0, &d);
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("缩放 {:.1}：每帧 {:.3} ms，20 帧/秒时约占单核 {:.2}%", scale, per, per * 20.0 / 10.0);
    }
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("bench") {
        return bench();
    }
    let out = std::env::args().nth(1).unwrap_or_else(|| "preview.ppm".into());
    let scale = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1.0f32);
    let cw = (CANVAS_W * scale).ceil() as i32;
    let ch = (CANVAS_H * scale).ceil() as i32;

    let mut scenes: Vec<(f32, Dango)> = Vec::new();
    let base = Dango { breathe: 1.0, ..Dango::calm() };
    scenes.push((0.0, base));
    scenes.push((0.3, base));
    scenes.push((0.75, base));
    scenes.push((1.0, base));
    scenes.push((1.0, Dango { blink: true, ..base }));
    scenes.push((0.0, Dango { mood: 0.7, ..base }));
    scenes.push((1.0, Dango { mood: 1.0, face: Face::Worried, sweat: 1, t: 0.3, ..base }));
    scenes.push((1.0, Dango { mood: 2.0, face: Face::Panic, sweat: 2, t: 0.2, shake: 0.8, ..base }));
    scenes.push((1.0, Dango { face: Face::Happy, sparkles: true, t: 0.1, ..base }));
    scenes.push((1.0, Dango { face: Face::Squeeze, squash: 0.2, ..base }));
    scenes.push((1.0, Dango { face: Face::Busy, mood: 1.6, ..base }));
    scenes.push((1.0, Dango { hop: 4.0, ..base }));

    let cols = scenes.len() as i32;
    let rows = 2;
    let (iw, ih) = (cw * cols, ch * rows + 70);
    let mut img = vec![0u8; (iw * ih * 3) as usize];

    for row in 0..rows {
        for (i, (morph, d)) in scenes.iter().enumerate() {
            let mut buf = vec![0u8; (cw * ch * 4) as usize];
            {
                let mut cv = Canvas::new(cw, ch, &mut buf);
                let mut pen = Pen::new(&mut cv, scale);
                draw_scene(&mut pen, *morph, d);
            }
            composite(&mut img, iw, &buf, cw, ch, i as i32 * cw, row * ch, row);
        }
    }
    // 图标
    let mut x = 4;
    for size in [16, 20, 24, 32, 48, 64] {
        let mut buf = vec![0u8; (size * size * 4) as usize];
        {
            let mut cv = Canvas::new(size, size, &mut buf);
            draw_icon(&mut cv, size);
        }
        composite(&mut img, iw, &buf, size, size, x, ch * rows + 3, 2);
        x += size + 8;
    }

    let mut f = format!("P6\n{} {}\n255\n", iw, ih).into_bytes();
    f.extend_from_slice(&img);
    std::fs::write(&out, f).unwrap();
    println!("wrote {} ({}x{})", out, iw, ih);
}

/// 把预乘 BGRA 叠加到背景上：row 0 = Win7 风格蓝色壁纸，1 = 浅色壁纸，2 = 深色任务栏。
fn composite(img: &mut [u8], iw: i32, buf: &[u8], w: i32, h: i32, ox: i32, oy: i32, bg: i32) {
    for y in 0..h {
        for x in 0..w {
            let (gx, gy) = (ox + x, oy + y);
            let bgc: [f32; 3] = match bg {
                0 => {
                    let t = gy as f32 / 200.0;
                    [20.0 + 40.0 * t, 90.0 + 60.0 * t, 160.0 + 50.0 * t]
                }
                1 => [236.0, 238.0, 240.0],
                _ => [40.0, 52.0, 66.0],
            };
            let i = ((y * w + x) * 4) as usize;
            let a = buf[i + 3] as f32 / 255.0;
            let o = ((gy * iw + gx) * 3) as usize;
            img[o] = (buf[i + 2] as f32 + bgc[0] * (1.0 - a)).min(255.0) as u8;
            img[o + 1] = (buf[i + 1] as f32 + bgc[1] * (1.0 - a)).min(255.0) as u8;
            img[o + 2] = (buf[i] as f32 + bgc[2] * (1.0 - a)).min(255.0) as u8;
        }
    }
}
