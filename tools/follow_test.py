"""显隐跟随 + 设置面板退出 测试

顺序很重要：先把"退出"在全新实例上测掉（此时窗口有输入焦点、状态干净），
再测隐藏/显示——隐藏后重新出现时，第一次点击可能只用于激活窗口，属系统行为。
"""
import ctypes, ctypes.wintypes as wt, time, subprocess, os, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from winutil import find_bar

u = ctypes.windll.user32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

EXE = Path(os.environ.get("TOKENBAR_EXE", r"F:/Projects/Tokenbar/target/debug/tokenbar.exe"))
# 用独立配置目录，别动用户自己的 ini
CFG = Path(os.environ.get("TOKENBAR_CONFIG_DIR", str(Path(__file__).parent / ".cache" / "follow-cfg")))


def bar():
    return find_bar(include_hidden=True)


def rect(h=None):
    h = h or bar()
    if not h:
        return (0, 0, 0, 0)
    r = wt.RECT()
    u.GetWindowRect(wt.HWND(h), ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def visible(h=None):
    """隐藏 = 窗口被挪到屏幕外（-30000）"""
    r = rect(h)
    return r[0] > -10000


def proc_alive():
    out = subprocess.run(
        ["tasklist", "/FI", "IMAGENAME eq Tokenbar.exe"], capture_output=True
    ).stdout.decode("gbk", errors="ignore")
    return "Tokenbar.exe" in out


def kill_all():
    subprocess.run(["taskkill", "/F", "/IM", "Tokenbar.exe"], capture_output=True)
    for _ in range(30):
        if not proc_alive():
            return
        time.sleep(0.2)


def start():
    CFG.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    env["TOKENBAR_CONFIG_DIR"] = str(CFG)
    subprocess.Popen(
        [str(EXE)],
        cwd=str(EXE.parent),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    for _ in range(40):
        time.sleep(0.25)
        if bar():
            return True
    return False


def click(x, y, right=False):
    u.SetCursorPos(int(x), int(y))
    time.sleep(0.35)
    if right:
        u.mouse_event(0x0008, 0, 0, 0, 0)
        time.sleep(0.12)
        u.mouse_event(0x0010, 0, 0, 0, 0)
    else:
        u.mouse_event(0x0002, 0, 0, 0, 0)
        time.sleep(0.12)
        u.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.5)


def click_into(hwnd):
    r = rect(hwnd)
    click((r[0] + r[2]) / 2, r[1] + 40)


def report(name, ok, extra=""):
    print(("PASS " if ok else "FAIL ") + name + (" | " + extra if extra else ""))
    return ok


def wait_dead(timeout=6.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        if not proc_alive():
            return True
        time.sleep(0.25)
    return False


def top_windows():
    out = []
    EP = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

    def cb(h, l):
        if u.IsWindowVisible(h):
            n = u.GetWindowTextLengthW(h)
            if n > 0:
                b = ctypes.create_unicode_buffer(n + 1)
                u.GetWindowTextW(h, b, n + 1)
                r = rect(h)
                if (r[2] - r[0]) > 200 and (r[3] - r[1]) > 100:
                    out.append((h, b.value, r))
        return True

    u.EnumWindows(EP(cb), 0)
    return out


res = []

# ---------- 1. 全新实例：悬浮按钮 -> 关闭 ----------
kill_all()
start()
time.sleep(3.2)
h = bar()
s = u.GetDpiForWindow(wt.HWND(h)) / 96.0
r = rect()
click((r[0] + r[2]) / 2, (r[1] + r[3]) / 2)  # 悬浮出按钮
click(r[2] - 15 * s, r[1] + (r[3] - r[1]) / 2 - 12 * s)  # 右上角 X
res.append(report("点右上角关闭按钮后进程结束", wait_dead()))

# ---------- 2. 再启一个：显隐跟随 ----------
start()
time.sleep(1.2)
h = bar()
s = u.GetDpiForWindow(wt.HWND(h)) / 96.0

wb0 = [w for w in top_windows() if "workbuddy" in w[1].lower()]
if wb0:
    u.ShowWindow(wt.HWND(wb0[0][0]), 9)
    click_into(wb0[0][0])
    time.sleep(1.5)
    res.append(report("WorkBuddy 前台时卡片可见", visible()))

np = subprocess.Popen(["notepad.exe"])
time.sleep(2.5)
cands = [w for w in top_windows() if "notepad" in w[1].lower()]
if cands:
    u.ShowWindow(wt.HWND(cands[0][0]), 9)
    click_into(cands[0][0])
    time.sleep(2.0)
    res.append(report("切到其它应用后自动隐藏", not visible()))
else:
    res.append(report("切到其它应用后自动隐藏", False, "找不到记事本"))
# 关窗口而不是 terminate 进程：新版记事本的窗口可能挂在另一个进程里，
# 只杀 Popen 的进程会留下看不见的空白窗口，占着前台挡住 WorkBuddy，害后面几轮假失败
if cands:
    u.PostMessageW(wt.HWND(cands[0][0]), 0x0010, 0, 0)  # WM_CLOSE
    time.sleep(1.0)
np.terminate()

wb = [w for w in top_windows() if "workbuddy" in w[1].lower()]
if wb:
    u.ShowWindow(wt.HWND(wb[0][0]), 9)
    click_into(wb[0][0])
    time.sleep(2.0)
    res.append(report("回到 WorkBuddy 后自动出现", visible()))
else:
    res.append(report("回到 WorkBuddy 后自动出现", False, "未找到 WorkBuddy 窗口"))

print("SUMMARY", sum(res), "/", len(res))
