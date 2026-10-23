//! 和系统打交道的功能：管理员权限、特权、开机自启（任务计划程序）、打开文件。

use crate::win::wide;
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, GetTokenInformation, LookupPrivilegeValueW, TokenElevation, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TASK_NAME: &str = "TuanziDesktopPet";

/// 当前进程是否以管理员身份（已提权）运行。
pub fn is_admin() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut TOKEN_ELEVATION as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// 打开一项特权（比如 SeDebugPrivilege），成功返回 true。
pub fn enable_privilege(name: &str) -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        let n = wide(name);
        let mut ok = LookupPrivilegeValueW(std::ptr::null(), n.as_ptr(), &mut luid) != 0;
        if ok {
            let tp =
                TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }] };
            ok = AdjustTokenPrivileges(token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0 && GetLastError() == 0;
        }
        CloseHandle(token);
        ok
    }
}

/// 以管理员身份重新启动自己（会弹 UAC），成功发起返回 true。
pub fn relaunch_elevated(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let (verb, file, params) = (wide("runas"), wide(&exe.to_string_lossy()), wide(args));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), SW_SHOWNORMAL) };
    r as isize > 32
}

/// 用记事本打开一个文件。
pub fn open_in_notepad(path: &std::path::Path) {
    let (verb, file, params) = (wide("open"), wide("notepad.exe"), wide(&format!("\"{}\"", path.display())));
    unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), params.as_ptr(), std::ptr::null(), SW_SHOWNORMAL) };
}

fn schtasks(args: &[&str]) -> bool {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// 生成任务计划的 XML：登录时以最高权限启动，不限运行时长，用电池也照常运行。
fn task_xml(exe: &str, dir: &str, user: &str) -> String {
    let (exe, dir, user) = (xml_escape(exe), xml_escape(dir), xml_escape(user));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>团子桌宠：登录后自动启动</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT5S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--autostart</Arguments>
      <WorkingDirectory>{dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// 打开或关闭开机自启。会调用 schtasks.exe，耗时几百毫秒，请在后台线程调用。
pub fn set_autostart(enable: bool) -> bool {
    if !enable {
        schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]);
        return !schtasks(&["/Query", "/TN", TASK_NAME]);
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let dir = exe.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) if !d.is_empty() => format!("{}\\{}", d, u),
        (_, Ok(u)) => u,
        _ => return false,
    };
    let xml = task_xml(&exe.to_string_lossy(), &dir, &user);
    // schtasks 读 XML 时最稳妥的是 UTF-16 LE + BOM
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let file = std::env::temp_dir().join(format!("tuanzi_task_{}.xml", std::process::id()));
    if std::fs::write(&file, bytes).is_err() {
        return false;
    }
    let ok = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &file.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&file);
    ok
}
