//! 团子桌宠：住在桌面上的轻量内存小管家（Windows 7 及以上）。

#![windows_subsystem = "windows"]
#![cfg_attr(not(windows), allow(dead_code))]

mod art;
mod canvas;
mod config;
mod level;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod dialog;
#[cfg(windows)]
mod memory;
#[cfg(windows)]
mod system;
#[cfg(windows)]
mod text;
#[cfg(windows)]
mod tray;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("团子桌宠只能在 Windows 上运行（Windows 7 及以上）。");
}
