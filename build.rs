//! 构建脚本：给 Windows 版 exe 嵌入图标、清单（以管理员身份运行、声明兼容 Win7、
//! 使用新版控件样式）和版本信息。图标直接用 src/art.rs 里的造型画出来，不需要图片文件。

#[path = "src/art.rs"]
mod art;
#[path = "src/canvas.rs"]
mod canvas;

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const ICON_SIZES: [i32; 8] = [16, 20, 24, 32, 40, 48, 64, 128];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/art.rs");
    println!("cargo:rerun-if-changed=src/canvas.rs");
    println!("cargo:rerun-if-changed=res/tuanzi.manifest");
    println!("cargo:rerun-if-env-changed=WINDRES");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    std::fs::write(out.join("tuanzi.ico"), make_ico()).unwrap();
    std::fs::copy("res/tuanzi.manifest", out.join("tuanzi.manifest")).unwrap();
    std::fs::write(out.join("tuanzi.rc"), rc_script()).unwrap();

    let ok = match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => compile_msvc(&out),
        _ => compile_gnu(&out),
    };
    if !ok {
        println!("cargo:warning=没有找到资源编译器，exe 将不带图标和清单（程序会在运行时自己申请管理员权限）。");
    }
}

fn compile_gnu(out: &Path) -> bool {
    let windres = env::var("WINDRES").unwrap_or_else(|_| {
        if cfg!(windows) {
            "windres".into()
        } else {
            let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
                Ok("x86") => "i686".to_string(),
                Ok(a) => a.to_string(),
                Err(_) => "x86_64".into(),
            };
            format!("{}-w64-mingw32-windres", arch)
        }
    });
    let status =
        Command::new(&windres).current_dir(out).args(["--codepage=65001", "-O", "coff", "-i", "tuanzi.rc", "-o", "tuanzi_res.o"]).status();
    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", out.join("tuanzi_res.o").display());
            true
        }
        Ok(s) => panic!("{} 编译资源失败：{}", windres, s),
        Err(_) => false,
    }
}

fn compile_msvc(out: &Path) -> bool {
    let status = Command::new("rc.exe").current_dir(out).args(["/nologo", "/c65001", "/fo", "tuanzi.res", "tuanzi.rc"]).status();
    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-link-arg-bins={}", out.join("tuanzi.res").display());
            println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
            true
        }
        _ => false,
    }
}

fn rc_script() -> String {
    let v = |k: &str| env::var(k).unwrap_or_else(|_| "0".into());
    let (major, minor, patch) = (v("CARGO_PKG_VERSION_MAJOR"), v("CARGO_PKG_VERSION_MINOR"), v("CARGO_PKG_VERSION_PATCH"));
    let ver = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    format!(
        r#"#pragma code_page(65001)
1 ICON "tuanzi.ico"
1 24 "tuanzi.manifest"
1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "080404B0"
    BEGIN
      VALUE "FileDescription", "团子桌宠"
      VALUE "FileVersion", "{ver}"
      VALUE "InternalName", "tuanzi"
      VALUE "OriginalFilename", "tuanzi.exe"
      VALUE "ProductName", "团子桌宠"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0804, 1200
  END
END
"#
    )
}

/// 生成多尺寸 .ico（32 位 BMP 格式，Win7 完全支持）。
fn make_ico() -> Vec<u8> {
    let mut images = Vec::new();
    for &size in &ICON_SIZES {
        let mut px = vec![0u8; (size * size * 4) as usize];
        {
            let mut cv = canvas::Canvas::new(size, size, &mut px);
            art::draw_icon(&mut cv, size);
        }
        let mut img = Vec::new();
        // BITMAPINFOHEADER（高度是两倍：颜色 + 掩码）
        for v in [40u32, size as u32, (size * 2) as u32] {
            img.extend_from_slice(&v.to_le_bytes());
        }
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        img.extend_from_slice(&[0u8; 24]);
        // 颜色：自下而上，非预乘 BGRA
        for y in (0..size).rev() {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                let a = px[i + 3] as u32;
                for c in 0..3 {
                    let v = if a == 0 { 0 } else { (px[i + c] as u32 * 255 + a / 2) / a };
                    img.push(v.min(255) as u8);
                }
                img.push(a as u8);
            }
        }
        // AND 掩码：1 位/像素，每行补齐到 4 字节，透明处为 1
        let row = ((size + 31) / 32 * 4) as usize;
        for y in (0..size).rev() {
            let mut bits = vec![0u8; row];
            for x in 0..size {
                if px[((y * size + x) * 4 + 3) as usize] == 0 {
                    bits[(x / 8) as usize] |= 0x80 >> (x % 8);
                }
            }
            img.extend_from_slice(&bits);
        }
        images.push((size, img));
    }

    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, img) in &images {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(img.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for (_, img) in &images {
        ico.extend_from_slice(img);
    }
    ico
}
