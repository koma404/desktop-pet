#!/usr/bin/env python3
"""检查编译出来的 exe 能不能在 Windows 7 上运行。

1. 只能依赖 Win7 自带的系统 DLL（不能有 UCRT / VC 运行库 / api-ms-win-* 等）；
2. 不能导入 Win8 及以后才有的函数；
3. 必须嵌入了清单（以管理员身份运行）、图标和版本信息。

用法：python3 tools/check_win7.py target/x86_64-pc-windows-gnu/release/tuanzi.exe
需要 x86_64-w64-mingw32-objdump / windres（Ubuntu 上装 binutils-mingw-w64-x86-64 即可）。
"""

import re
import subprocess
import sys

# Win7 SP1 自带的 DLL
ALLOWED_DLLS = {
    "advapi32.dll", "bcrypt.dll", "comctl32.dll", "gdi32.dll", "kernel32.dll", "msvcrt.dll",
    "ntdll.dll", "shell32.dll", "user32.dll", "ws2_32.dll", "userenv.dll", "ole32.dll",
    "psapi.dll", "shlwapi.dll", "uxtheme.dll", "secur32.dll", "crypt32.dll",
}

# Win8 及以后才有（或 Win7 需要额外补丁）的函数
WIN8_PLUS = {
    "GetSystemTimePreciseAsFileTime", "WaitOnAddress", "WakeByAddressSingle", "WakeByAddressAll",
    "CreateFile2", "SetThreadDescription", "GetThreadDescription", "ProcessPrng",
    "GetDpiForWindow", "GetDpiForSystem", "GetDpiForMonitor", "SetProcessDpiAwareness",
    "SetProcessDpiAwarenessContext", "SetThreadDpiAwarenessContext", "AdjustWindowRectExForDpi",
    "GetSystemMetricsForDpi", "SystemParametersInfoForDpi", "EnableNonClientDpiScaling",
    "PrefetchVirtualMemory", "DiscardVirtualMemory", "OfferVirtualMemory", "ReclaimVirtualMemory",
    "SetDefaultDllDirectories", "AddDllDirectory", "RemoveDllDirectory", "GetCurrentPackageId",
    "GetPackageFamilyName", "CreateFileMappingFromApp", "MapViewOfFileFromApp", "VirtualAllocFromApp",
    "GetOverlappedResultEx", "CompareObjectHandles", "RoInitialize", "SetCoalescableTimer",
    "GetPointerType", "RegisterPointerInputTarget", "GetTickCount64Precise",
}


def main(path: str) -> int:
    problems = []
    dump = subprocess.run(["x86_64-w64-mingw32-objdump", "-p", path], capture_output=True, text=True, check=True).stdout

    dlls = {}
    current = None
    for line in dump.splitlines():
        m = re.match(r"\s*DLL Name: (\S+)", line)
        if m:
            current = m.group(1).lower()
            dlls.setdefault(current, set())
            continue
        m = re.match(r"\s+[0-9a-f]+\s+[0-9a-f]+\s+([A-Za-z_][\w@]*)\s*$", line)
        if current and m:
            dlls[current].add(m.group(1))
        elif not line.strip():
            current = None

    for dll, funcs in sorted(dlls.items()):
        if dll not in ALLOWED_DLLS:
            problems.append(f"依赖了 Win7 上可能没有的 DLL：{dll}")
        bad = sorted(funcs & WIN8_PLUS)
        if bad:
            problems.append(f"{dll} 中导入了 Win8+ 才有的函数：{', '.join(bad)}")

    print("导入的 DLL：")
    for dll, funcs in sorted(dlls.items()):
        print(f"  {dll}（{len(funcs)} 个函数）")

    rc = subprocess.run(["x86_64-w64-mingw32-windres", "-i", path, "-O", "rc"], capture_output=True, text=True).stdout
    for needle, what in [("RT_MANIFEST", "清单"), ("RT_GROUP_ICON", "图标"), ("VERSIONINFO", "版本信息")]:
        if needle not in rc:
            problems.append(f"exe 里没有嵌入{what}")
    data = open(path, "rb").read()
    if b'level="requireAdministrator"' not in data:
        problems.append("清单里没有声明以管理员身份运行")
    if b"35138b9a-5d96-4fbd-8e2d-a2440225f93a" not in data:
        problems.append("清单里没有声明兼容 Windows 7")

    if problems:
        print("\n发现问题：")
        for p in problems:
            print("  ✗ " + p)
        return 1
    print("\n✓ 没有发现 Win7 兼容性问题，资源也都嵌入了。")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1]))
