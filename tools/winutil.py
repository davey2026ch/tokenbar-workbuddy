"""按进程名找 Tokenbar 自己的窗口（WorkBuddy 主窗口标题也叫 Tokenbar）"""
import ctypes, ctypes.wintypes as wt

u = ctypes.windll.user32
k = ctypes.windll.kernel32

_EP = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def _pid_of(hwnd):
    pid = wt.DWORD()
    u.GetWindowThreadProcessId(wt.HWND(hwnd), ctypes.byref(pid))
    return pid.value


def _exe_of(pid):
    h = k.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        if not k.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            return ""
        return buf.value.replace("\\", "/").rsplit("/", 1)[-1].lower()
    finally:
        k.CloseHandle(h)


def find_bar(include_hidden=False):
    """返回 tokenbar.exe 的顶层窗口句柄，找不到返回 0"""
    found = []

    def cb(hwnd, _):
        if include_hidden:
            if _exe_of(_pid_of(hwnd)) != "tokenbar.exe":
                return True
            n = u.GetWindowTextLengthW(hwnd)
            if n == 0:
                return True
            buf = ctypes.create_unicode_buffer(n + 1)
            u.GetWindowTextW(hwnd, buf, n + 1)
            if buf.value == "Tokenbar":
                found.append(hwnd)
                return False
            return True
        if not u.IsWindowVisible(hwnd):
            return True
        n = u.GetWindowTextLengthW(hwnd)
        if n == 0:
            return True
        buf = ctypes.create_unicode_buffer(n + 1)
        u.GetWindowTextW(hwnd, buf, n + 1)
        if buf.value == "Tokenbar" and _exe_of(_pid_of(hwnd)) == "tokenbar.exe":
            found.append(hwnd)
            return False
        return True

    u.EnumWindows(_EP(cb), 0)
    return found[0] if found else 0
