import ctypes, ctypes.wintypes as wt, sys, time, json
from pathlib import Path

user32 = ctypes.windll.user32
dwmapi = ctypes.windll.dwmapi
kernel32 = ctypes.windll.kernel32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    try:
        user32.SetProcessDPIAware()
    except Exception:
        pass


def _pid_of(hwnd):
    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(wt.HWND(hwnd), ctypes.byref(pid))
    return pid.value


def _exe_of(pid):
    h = kernel32.OpenProcess(0x1000, False, pid)
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wt.DWORD(1024)
        if not kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            return ""
        return buf.value.replace("\\", "/").rsplit("/", 1)[-1].lower()
    finally:
        kernel32.CloseHandle(h)


def find_windows(title_sub):
    out = []
    EnumWindowsProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

    def cb(hwnd, lparam):
        n = user32.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        if buf.value and title_sub.lower() in buf.value.lower():
            if _exe_of(_pid_of(hwnd)) == "tokenbar.exe":
                out.append((hwnd, buf.value))
        return True

    user32.EnumWindows(EnumWindowsProc(cb), 0)
    return out


def rect_of(hwnd):
    r = wt.RECT()
    dwmapi.DwmGetWindowAttribute(wt.HWND(hwnd), 9, ctypes.byref(r), ctypes.sizeof(r))
    if r.right == r.left:
        user32.GetWindowRect(wt.HWND(hwnd), ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def shot(title_sub, path):
    from PIL import ImageGrab
    ws = find_windows(title_sub)
    if not ws:
        print(json.dumps({"ok": False, "reason": "window not found"}))
        return None
    hwnd, title = ws[0]
    user32.SetWindowPos(wt.HWND(hwnd), wt.HWND(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0040)
    time.sleep(0.6)
    l, t, r, b = rect_of(hwnd)
    img = ImageGrab.grab(bbox=(l, t, r, b))
    img.save(path)
    print(json.dumps({"ok": True, "hwnd": hwnd, "rect": [l, t, r, b], "size": [r - l, b - t]}))
    return (l, t, r, b)


def click_at(x, y, double=False):
    user32.SetCursorPos(int(x), int(y))
    time.sleep(0.35)
    user32.mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.05)
    user32.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.35)


def move_to(x, y):
    user32.SetCursorPos(int(x), int(y))
    time.sleep(0.4)


def shot_hwnd(hwnd, path):
    from PIL import ImageGrab
    l, t, r, b = rect_of(hwnd)
    img = ImageGrab.grab(bbox=(l, t, r, b))
    img.save(path)
    print(json.dumps({"ok": True, "hwnd": hwnd, "rect": [l, t, r, b], "size": [r - l, b - t]}))
    return (l, t, r, b)


def shot_bar(path):
    sys.path.insert(0, str(Path(__file__).parent))
    from winutil import find_bar
    hwnd = find_bar(include_hidden=True)
    if not hwnd:
        print(json.dumps({"ok": False, "reason": "bar not found"}))
        return None
    user32.SetWindowPos(wt.HWND(hwnd), wt.HWND(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0040)
    time.sleep(0.5)
    return shot_hwnd(hwnd, path)


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "shot":
        shot(sys.argv[2], sys.argv[3])
    elif cmd == "click":
        click_at(float(sys.argv[2]), float(sys.argv[3]))
    elif cmd == "move":
        move_to(float(sys.argv[2]), float(sys.argv[3]))
    elif cmd == "bar":
        shot_bar(sys.argv[2])
    elif cmd == "find":
        print(find_windows(sys.argv[2]))
