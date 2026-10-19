"""Tokenbar 端到端交互测试（v1.3：悬浮按钮 + 独立设置窗口）。"""

import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
import time
from pathlib import Path

from PIL import ImageGrab

HERE = Path(__file__).parent
sys.path.insert(0, str(HERE))
from winutil import find_bar  # noqa: E402

u = ctypes.windll.user32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

EXE = Path(os.environ.get("TOKENBAR_EXE", r"F:/Projects/Tokenbar/target/debug/tokenbar.exe"))
# 配置目录默认跟程序放一起（跑测试时用独立目录，别碰到用户自己的设置）
CFG = Path(os.environ.get("TOKENBAR_CONFIG_DIR", str(EXE.parent / "_e2e_cfg")))
INI = CFG / "tokenbar.ini"

# 设置窗口里的布局坐标（逻辑 pt，与 src/settings.rs 一致）
WIN_W, WIN_H = 420.0, 700.0
PAD = 20.0
ROW_H = 36.0
LIST_TOP = 318.0
BY = WIN_H - 18.0 - 34.0

results = []


def report(name, ok, extra=""):
    results.append(bool(ok))
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f"  ({extra})" if extra else ""))


# ---------------------------------------------------------------- 基础

def proc_alive():
    out = subprocess.run(
        ["tasklist", "/FI", "IMAGENAME eq Tokenbar.exe"], capture_output=True
    ).stdout.decode("gbk", errors="ignore")
    return "Tokenbar.exe" in out


def kill():
    subprocess.run(["taskkill", "/F", "/IM", "Tokenbar.exe"], capture_output=True)
    time.sleep(1.2)


def start():
    CFG.mkdir(parents=True, exist_ok=True)
    if INI.exists():
        INI.unlink()
    stats = INI.with_name("tokenbar-stats.json")
    if stats.exists():
        stats.unlink()
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
        if find_bar(include_hidden=True):
            time.sleep(1.2)
            return wait_bar_ready()
        time.sleep(0.25)
    return False


def bar():
    return find_bar(include_hidden=True)


def wait_bar_ready(timeout=12.0):
    """等窗口落到底部默认位置（eframe 建窗后要 2~3 秒才摆好）"""
    sh = u.GetSystemMetrics(1)
    t0 = time.time()
    last = None
    steady = 0.0
    while time.time() - t0 < timeout:
        h = bar()
        if h:
            r = rect(h)
            if last == r:
                steady += 0.3
            else:
                steady = 0.0
            last = r
            if steady >= 1.2 and sh - r[3] < 400 and r[2] - r[0] > 600:
                return True
        time.sleep(0.3)
    return False


def rect(hwnd):
    r = wt.RECT()
    u.GetWindowRect(wt.HWND(hwnd), ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def client_origin(hwnd):
    pt = wt.POINT(0, 0)
    u.ClientToScreen(wt.HWND(hwnd), ctypes.byref(pt))
    return (pt.x, pt.y)


def scale(hwnd):
    return u.GetDpiForWindow(wt.HWND(hwnd)) / 96.0


def click(x, y, hold=0.1):
    u.SetCursorPos(int(x), int(y))
    time.sleep(0.3)
    u.mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(hold)
    u.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.65)


def click_bar(lx, ly, hold=0.1):
    h = bar()
    l, t, _, _ = rect(h)
    s = scale(h)
    click(l + lx * s, t + ly * s, hold)


def move(x, y):
    u.SetCursorPos(int(x), int(y))
    time.sleep(0.5)


def settings():
    for _ in range(20):
        hwnd = u.FindWindowW(None, "Tokenbar 设置")
        if hwnd:
            return hwnd
        time.sleep(0.2)
    return 0


def click_set(lx, ly, hold=0.1):
    h = settings()
    ox, oy = client_origin(h)
    s = scale(h)
    click(ox + lx * s, oy + ly * s, hold)


def ini_text():
    try:
        return INI.read_text(encoding="utf-8")
    except FileNotFoundError:
        return ""


def ini_get(key):
    for line in ini_text().splitlines():
        if line.startswith(key + "="):
            return line.split("=", 1)[1].strip()
    return None


def bar_size():
    l, t, r, b = rect(bar())
    s = scale(bar())
    return ((r - l) / s, (b - t) / s)


def grab_bar():
    l, t, r, b = rect(bar())
    return ImageGrab.grab(bbox=(l, t, r, b)).convert("RGB")


def bar_bg():
    """取卡片底部留白处的平均色，用来判断主题（避开文字）"""
    img = grab_bar()
    w, h = img.size
    px = [img.getpixel((x, y)) for x in range(24, 70) for y in range(h - 7, h - 3)]
    return tuple(sum(c[i] for c in px) // len(px) for i in range(3))


def overlaps(a, b):
    return not (a[3] <= b[1] or a[1] >= b[3] or a[2] <= b[0] or a[0] >= b[2])


def client_size(hwnd):
    r = wt.RECT()
    u.GetClientRect(wt.HWND(hwnd), ctypes.byref(r))
    s = scale(hwnd)
    return (r.right / s, r.bottom / s)


def open_settings():
    w, h = bar_size()
    click_bar(w * 0.5, h * 0.5, hold=0.05)
    time.sleep(0.4)
    click_bar(w - 15.0, h * 0.5 + 12.0)
    return settings() != 0


# ---------------------------------------------------------------- 用例

def main():
    if not EXE.exists():
        print(f"找不到 {EXE}")
        return 1
    kill()
    if not start():
        print("启动失败")
        return 1

    # 1. 默认位置：底部居中、底部留白 ~118pt
    l, t, r, b = rect(bar())
    s = scale(bar())
    sw = u.GetSystemMetrics(0)
    sh = u.GetSystemMetrics(1)
    report("默认水平居中", abs((l + r) / 2 - sw / 2) < 40, f"centerX={(l + r) / 2:.0f}")
    report("底部留白 118pt", abs((sh - b) / s - 118) < 6, f"gap={(sh - b) / s:.1f}pt")
    w0, h0 = bar_size()
    report("横排高度 76pt", abs(h0 - 76) < 2, f"h={h0:.1f}")

    # 2. 悬浮才出现右侧按钮
    move(40, 40)
    time.sleep(0.9)
    before = grab_bar()
    body_l, body_t, _, _ = rect(bar())
    move(body_l + (w0 * 0.5) * s, body_t + (h0 * 0.5) * s)
    time.sleep(0.9)
    after = grab_bar()
    zone_x = int((w0 - 30) * s)
    diff = sum(
        1
        for x in range(zone_x, min(after.width, before.width))
        for y in range(after.height)
        if before.getpixel((x, y)) != after.getpixel((x, y))
    )
    report("悬浮出现设置/关闭按钮", diff > 150, f"changed={diff}px")

    # 3. 点齿轮 → 独立设置窗口
    opened = open_settings()
    report("点设置按钮弹出独立窗口", opened, f"hwnd={settings()}")
    if not opened:
        kill()
        return 1
    report("设置窗口不遮挡 bar（横排）", not overlaps(rect(settings()), rect(bar())))
    cw, ch = client_size(settings())
    report("设置窗口 420x700", abs(cw - WIN_W) < 2 and abs(ch - WIN_H) < 2, f"{cw:.0f}x{ch:.0f}")

    # 4. 竖排：实时生效
    click_set(PAD + 195 + 92.5, 149)
    time.sleep(0.9)
    w1, h1 = bar_size()
    report("点竖排后 bar 立即变高", abs(h1 - 170) < 8, f"h={h1:.1f}")
    report("竖排已生效", ini_get("layout") == "vertical", ini_get("layout"))
    report("设置窗口不遮挡 bar（竖排）", not overlaps(rect(settings()), rect(bar())),
           f"set={rect(settings())} bar={rect(bar())}")

    # 5. 主题：实时生效（"跟随系统"可能是深也可能是浅，挑相反的那个点）
    px_before = bar_bg()
    light_now = sum(px_before) > 400
    click_set(210 if light_now else 336, 55)
    time.sleep(0.9)
    px_after = bar_bg()
    report(
        "点主题后 bar 立即换色",
        (sum(px_after) > 400) != light_now and abs(sum(px_after) - sum(px_before)) > 60,
        f"{px_before} -> {px_after}",
    )

    # 6. 勾选"缓存命中"（第 6 行，默认是关的）→ 竖排下多一行，卡片变高
    _, h2 = bar_size()
    click_set(WIN_W - PAD - 14, LIST_TOP + ROW_H * 5 + ROW_H * 0.5 - 2)
    time.sleep(0.9)
    _, h3 = bar_size()
    report("勾选缓存指标后 bar 多一行", h3 > h2 + 15, f"{h2:.0f} -> {h3:.0f}")
    report("勾选缓存已生效", ini_get("cache") == "1", ini_get("cache"))

    # 7. 取消：全部回滚
    click_set(WIN_W - PAD - 92 - 8 - 78 + 39, BY + 17)
    time.sleep(1.1)
    report("取消后设置窗口关闭", settings() == 0)
    w4, h4 = bar_size()
    report("取消后 bar 回到横排 76pt", abs(h4 - 76) < 2, f"h={h4:.1f}")
    report("取消后 layout 回滚", ini_get("layout") == "horizontal", ini_get("layout"))
    report("取消后 theme 回滚", ini_get("theme") == "system", ini_get("theme"))
    report("取消后指标显示回滚", ini_get("cache") == "0", ini_get("cache"))
    px_cancel = bar_bg()
    report(
        "取消后配色回滚到跟随系统",
        (sum(px_cancel) > 400) == light_now,
        f"{px_cancel} vs {px_before}",
    )

    # 8. 重开：拖动首行到底 + 确定
    if not open_settings():
        report("再次打开设置窗口", False)
        kill()
        return 1
    ox, oy = client_origin(settings())
    s2 = scale(settings())
    x = int(ox + 120 * s2)
    y0 = int(oy + (LIST_TOP + 16) * s2)
    u.SetCursorPos(x, y0)
    time.sleep(0.4)
    u.mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.3)
    for k in range(1, 13):
        u.SetCursorPos(x, int(y0 + k * 15 * s2))
        time.sleep(0.05)
    time.sleep(0.3)
    u.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.8)
    click_set(WIN_W - PAD - 46, BY + 17)
    time.sleep(1.1)
    order = ini_get("order") or ""
    report("拖动后顺序写入 ini", order.startswith("duration"), order)
    report("确定后设置窗口关闭", settings() == 0)

    # 9. bar 上的关闭按钮 → 进程退出
    w5, h5 = bar_size()
    click_bar(w5 * 0.5, h5 * 0.5, hold=0.05)
    time.sleep(0.5)
    click_bar(w5 - 15.0, h5 * 0.5 - 12.0)
    ok = False
    for _ in range(24):
        if not proc_alive():
            ok = True
            break
        time.sleep(0.25)
    report("点关闭按钮退出进程", ok)

    kill()
    passed = sum(1 for x in results if x)
    print(f"\n{passed}/{len(results)} 通过")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
