"""设置窗口视觉回归：老版本 vs 新版本，同一份配置截同一块像素。

用法: python vis_settings.py <exe> <out.png>
"""
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from vis_capture import CACHE, FIXTURE, build_fixture  # noqa: E402
from winutil import find_bar  # noqa: E402

u = ctypes.windll.user32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

INI = """[window]
x=300
y=300

[general]
theme=light
layout=horizontal
follow_workbuddy=0

[metrics]
order=ttft,duration,tps,input,output,cache
ttft=1
duration=1
tps=1
input=1
output=1
cache=1
"""


def click(x, y, hold=0.1):
    u.SetCursorPos(int(x), int(y))
    time.sleep(0.3)
    u.mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(hold)
    u.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.7)


def settings_hwnd():
    for _ in range(25):
        h = u.FindWindowW(None, "Tokenbar 设置")
        if h:
            return h
        time.sleep(0.2)
    return 0


def main():
    exe = Path(sys.argv[1]).resolve()
    out = Path(sys.argv[2]).resolve()
    build_fixture()
    run_dir = CACHE / "run" / out.stem
    if run_dir.exists():
        shutil.rmtree(run_dir, ignore_errors=True)
    run_dir.mkdir(parents=True)
    local = run_dir / "tokenbar.exe"
    shutil.copy2(exe, local)
    (run_dir / "tokenbar.ini").write_text(
        os.environ.get("TOKENBAR_VIS_INI") or INI, encoding="utf-8"
    )

    subprocess.run(["taskkill", "/F", "/IM", "tokenbar.exe"], capture_output=True)
    time.sleep(1.2)
    env = dict(os.environ)
    env["USERPROFILE"] = str(FIXTURE)
    env["HOME"] = str(FIXTURE)
    env["TOKENBAR_CONFIG_DIR"] = str(run_dir)
    subprocess.Popen(
        [str(local)], cwd=str(run_dir), env=env,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    hwnd = 0
    for _ in range(40):
        hwnd = find_bar(include_hidden=True)
        if hwnd:
            break
        time.sleep(0.25)
    if not hwnd:
        print("bar not found")
        return 1
    time.sleep(4.0)

    r = wt.RECT()
    u.GetWindowRect(wt.HWND(hwnd), ctypes.byref(r))
    s = u.GetDpiForWindow(wt.HWND(hwnd)) / 96.0
    w, h = (r.right - r.left) / s, (r.bottom - r.top) / s
    # 悬停出现按钮，再点下方的齿轮
    click(r.left + w * 0.5 * s, r.top + h * 0.5 * s, hold=0.05)
    click(r.left + (w - 15.0) * s, r.top + (h * 0.5 + 12.0) * s)
    sh = settings_hwnd()
    if not sh:
        print("settings not found")
        return 1
    time.sleep(1.5)
    # 鼠标移开，避免 hover 高亮
    u.SetCursorPos(20, 20)
    time.sleep(0.8)

    from PIL import ImageGrab

    r2 = wt.RECT()
    u.GetWindowRect(wt.HWND(sh), ctypes.byref(r2))
    img = ImageGrab.grab(bbox=(r2.left, r2.top, r2.right, r2.bottom)).convert("RGB")
    img.save(out)
    print(f"saved {out} size={img.size}")
    subprocess.run(["taskkill", "/F", "/IM", "tokenbar.exe"], capture_output=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
