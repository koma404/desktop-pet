# 团子桌宠 🍡

住在桌面上的轻量内存小管家，专为 **Windows 7 / 4 GB 内存** 的电脑设计。

平时伪装成一个 Win7 风格的文件夹，角落里藏着团子的小脑袋；内存紧张时它会现出原形提醒你，还能帮你清理内存。

![团子的样子：伪装文件夹、平常、紧张、告急、清理完成、被戳、清理中](docs/preview.png)

<sub>从左到右：伪装文件夹 · 平常 · 紧张（75%）· 告急（85%）· 清理完成 · 被戳 · 正在清理</sub>

## 下载

打开本仓库的 **Actions** 页面，点开最新一次成功的「构建 Windows 版」，在页面底部的 **Artifacts** 里下载 `tuanzi-win7-x64`。解压后得到：

- `tuanzi.exe`：团子本体，单个文件，不需要安装任何运行库
- `README.txt`：使用说明（记事本可直接打开）

推送 `v` 开头的标签（比如 `v1.0.0`）时，还会自动发布到 **Releases** 页面。

## 功能

| | |
|---|---|
| **低调伪装** | 平时是一个桌面文件夹（名字可改），和桌面图标一样待在所有窗口下面 |
| **悬停现形** | 鼠标停上去，变回团子并显示「内存 63% · 2.5 / 3.9 GB」 |
| **紧张提醒** | 内存 ≥ 75%：在原地现形，变黄、皱眉、冒汗 |
| **告急提醒** | 内存 ≥ 85%：变红、喘气发抖，跳到所有窗口最前面，并自动轻度清理 |
| **轻度清理** | 双击团子，或在右键菜单里点。整理各程序的闲置内存，不关闭任何程序 |
| **深度清理** | 右键菜单。额外清理系统文件缓存、已修改页和备用内存 |
| **吃内存大户** | 右键菜单。按程序合并列出占用最多的程序，确认后才结束；系统进程受保护 |
| **戳一戳** | 单击团子，它会被戳扁一下并回应你 |
| **托盘图标** | 鼠标停留显示内存；左键把团子叫出来；右键打开菜单 |
| **开机自启** | 通过「任务计划程序」以最高权限启动，开机时不弹 UAC |
| **安静模式** | 30 分钟内告急也不跳到最前面 |

状态切换带回差：告急后要降到 80% 以下才解除，紧张要降到 72% 以下才解除，避免在临界值附近反复跳动。自动清理两次之间至少间隔 5 分钟。

## 轻量是怎么做到的

- **Rust 原生程序**，只依赖 Win7 自带的系统 DLL，没有 .NET、VC 运行库或 UCRT。
- **不用任何图片素材**：团子、文件夹、托盘图标和 exe 图标都是代码实时画的（`src/art.rs`，基于带符号距离场的抗锯齿绘制）。
- **伪装时完全不重绘**：只在团子现身时才以每秒 20 帧播放动画，一帧绘制约 0.14 ms。
- **每 2 秒读一次内存**（`GlobalMemoryStatusEx`），开销可以忽略。
- 清理在后台线程进行，不会卡住界面。

## 设置

设置保存在 exe 旁边的 `tuanzi.ini`（UTF-8，记事本可直接编辑，保存后几秒内自动生效）：

```ini
folder_name=资料              ; 伪装文件夹的名字
warn_percent=75               ; 达到多少开始紧张
alert_percent=85              ; 达到多少进入告急
auto_clean=1                  ; 告急时自动轻度清理
auto_clean_interval_min=5     ; 自动清理的最短间隔（分钟）
popup_on_alert=1              ; 告急时跳到最前面
autostart=1                   ; 开机自启
check_interval_sec=2          ; 检查内存的间隔（秒）
```

## 自己编译

推荐在 Linux（或 WSL）上交叉编译，和 CI 完全一致：

```bash
sudo apt install gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64
rustup toolchain install 1.77.2 --profile minimal --target x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
python3 tools/check_win7.py target/x86_64-pc-windows-gnu/release/tuanzi.exe
```

> Rust **1.77.2** 是最后一个官方支持 Windows 7 的版本，已经在 `rust-toolchain.toml` 里锁定，请不要升级。

在任意系统上预览造型、测量绘制速度：

```bash
cargo run --example preview -- preview.ppm 2.0   # 生成各状态拼图
cargo run --release --example preview -- bench   # 每帧绘制耗时
cargo test                                       # 设置解析、状态判定的单元测试
```

## 代码结构

```
src/
  main.rs     入口
  app.rs      团子的大脑：状态、动画、鼠标、菜单、窗口消息
  art.rs      团子和伪装文件夹的造型（纯 Rust，可跨平台预览）
  canvas.rs   抗锯齿软件绘图（SDF）
  level.rs    平静 / 紧张 / 告急的判定（带回差）
  memory.rs   内存读取、轻度/深度清理、吃内存大户
  config.rs   tuanzi.ini 的读写
  system.rs   管理员权限、特权、开机自启（任务计划）
  text.rs     桌面图标风格的文字（白字 + 柔和阴影）
  tray.rs     托盘图标
  dialog.rs   改名对话框
  win.rs      Win32 小工具
build.rs      生成 exe 图标、嵌入清单和版本信息
tools/check_win7.py   检查 exe 是否只用了 Win7 就有的 API
```
