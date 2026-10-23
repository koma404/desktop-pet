//! 内存：读取占用、轻度/深度清理、找出吃内存大户。

use crate::system::enable_privilege;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Memory::SetSystemFileCacheSize;
use windows_sys::Win32::System::ProcessStatus::{K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    PROCESS_VM_READ,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MemStatus {
    pub total: u64,
    pub avail: u64,
}

impl MemStatus {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.avail)
    }

    /// 占用百分比（0~100）。
    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            self.used() as f32 * 100.0 / self.total as f32
        }
    }
}

pub fn sample() -> MemStatus {
    unsafe {
        let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut ms) == 0 {
            return MemStatus::default();
        }
        MemStatus { total: ms.ullTotalPhys, avail: ms.ullAvailPhys }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanKind {
    Light = 0,
    Deep = 1,
}

/// 遍历所有进程：(pid, exe 文件名)。
fn for_each_process(mut f: impl FnMut(u32, &[u16])) {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut pe) != 0 {
            loop {
                let n = pe.szExeFile.iter().position(|&c| c == 0).unwrap_or(pe.szExeFile.len());
                f(pe.th32ProcessID, &pe.szExeFile[..n]);
                if Process32NextW(snap, &mut pe) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
}

fn open(pid: u32, access: u32) -> Option<HANDLE> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() {
        None
    } else {
        Some(h)
    }
}

/// 轻度清理：让各个进程把暂时不用的内存还给系统（整理工作集）。
/// 不会结束任何程序；`skip_pid` 一般传前台程序，避免正在用的程序卡一下。
/// 返回成功整理的进程数。
pub fn clean_light(skip_pid: u32) -> u32 {
    let mut n = 0;
    for_each_process(|pid, _| {
        if pid == 0 || pid == 4 || pid == skip_pid {
            return;
        }
        if let Some(h) = open(pid, PROCESS_SET_QUOTA | PROCESS_QUERY_LIMITED_INFORMATION) {
            if unsafe { K32EmptyWorkingSet(h) } != 0 {
                n += 1;
            }
            unsafe { CloseHandle(h) };
        }
    });
    n
}

type NtSetSystemInformation = unsafe extern "system" fn(u32, *mut core::ffi::c_void, u32) -> i32;

const SYSTEM_MEMORY_LIST_INFORMATION: u32 = 80;
const MEMORY_EMPTY_WORKING_SETS: u32 = 2;
const MEMORY_FLUSH_MODIFIED_LIST: u32 = 3;
const MEMORY_PURGE_STANDBY_LIST: u32 = 4;

/// 深度清理（需要管理员）：清空所有工作集 → 清理系统文件缓存 → 写回已修改页 → 清空备用列表。
/// 返回是否至少有一步成功。
pub fn clean_deep() -> bool {
    enable_privilege("SeProfileSingleProcessPrivilege");
    enable_privilege("SeIncreaseQuotaPrivilege");
    let ntdll = crate::win::wide("ntdll.dll");
    let f: Option<NtSetSystemInformation> = unsafe {
        let m = GetModuleHandleW(ntdll.as_ptr());
        if m.is_null() {
            None
        } else {
            GetProcAddress(m, b"NtSetSystemInformation\0".as_ptr()).map(|p| std::mem::transmute(p))
        }
    };
    let command = |cmd: u32| -> bool {
        match f {
            Some(f) => {
                let mut c = cmd;
                unsafe { f(SYSTEM_MEMORY_LIST_INFORMATION, &mut c as *mut u32 as *mut _, 4) >= 0 }
            }
            None => false,
        }
    };
    let mut ok = command(MEMORY_EMPTY_WORKING_SETS);
    if !ok {
        // 系统级命令不可用时，退回到逐个进程整理
        ok = clean_light(0) > 0;
    }
    ok |= unsafe { SetSystemFileCacheSize(usize::MAX, usize::MAX, 0) } != 0;
    ok |= command(MEMORY_FLUSH_MODIFIED_LIST);
    ok |= command(MEMORY_PURGE_STANDBY_LIST);
    ok
}

/// 一组同名进程（比如浏览器的多个进程合在一起）。
#[derive(Clone, Debug)]
pub struct ProcGroup {
    pub name: String,
    pub count: u32,
    pub bytes: u64,
    /// 系统关键进程，不允许结束
    pub protected: bool,
}

/// 结束后会让系统不稳定、或者马上又会重启的进程。
const PROTECTED: &[&str] = &[
    "system",
    "registry",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsm.exe",
    "svchost.exe",
    "dwm.exe",
    "explorer.exe",
    "audiodg.exe",
    "conhost.exe",
    "taskhost.exe",
    "taskhostex.exe",
    "taskhostw.exe",
    "taskeng.exe",
    "spoolsv.exe",
    "ctfmon.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "wudfhost.exe",
    "memory compression",
];

pub fn is_protected(name_lower: &str) -> bool {
    PROTECTED.contains(&name_lower)
}

fn working_set(pid: u32) -> u64 {
    let h = open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ).or_else(|| open(pid, PROCESS_QUERY_LIMITED_INFORMATION));
    let Some(h) = h else { return 0 };
    unsafe {
        let mut pmc: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = K32GetProcessMemoryInfo(h, &mut pmc, pmc.cb) != 0;
        CloseHandle(h);
        if ok {
            pmc.WorkingSetSize as u64
        } else {
            0
        }
    }
}

/// 占用物理内存最多的前 n 组进程（按程序名合并）。
pub fn top_groups(n: usize) -> Vec<ProcGroup> {
    let me = unsafe { GetCurrentProcessId() };
    let mut groups: Vec<ProcGroup> = Vec::new();
    for_each_process(|pid, exe| {
        if pid == 0 || pid == me {
            return;
        }
        let ws = working_set(pid);
        if ws == 0 {
            return;
        }
        let name = String::from_utf16_lossy(exe);
        let key = name.to_lowercase();
        match groups.iter_mut().find(|g| g.name.to_lowercase() == key) {
            Some(g) => {
                g.count += 1;
                g.bytes += ws;
            }
            None => groups.push(ProcGroup { protected: is_protected(&key), name, count: 1, bytes: ws }),
        }
    });
    groups.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    groups.truncate(n);
    groups
}

/// 结束所有叫这个名字的进程，返回 (成功数, 失败数)。
pub fn kill_group(name: &str) -> (u32, u32) {
    let key = name.to_lowercase();
    if is_protected(&key) {
        return (0, 0);
    }
    let me = unsafe { GetCurrentProcessId() };
    let (mut ok, mut fail) = (0, 0);
    for_each_process(|pid, exe| {
        if pid == me || String::from_utf16_lossy(exe).to_lowercase() != key {
            return;
        }
        match open(pid, PROCESS_TERMINATE) {
            Some(h) => {
                if unsafe { TerminateProcess(h, 1) } != 0 {
                    ok += 1;
                } else {
                    fail += 1;
                }
                unsafe { CloseHandle(h) };
            }
            None => fail += 1,
        }
    });
    (ok, fail)
}
