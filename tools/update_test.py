"""自动更新的端到端测试：本地桩服务器冒充 Gitee，把整条链路跑一遍。

  场景 A：启动 3 秒自动检查 → 弹提示卡 → 点一下下载 → 校验 → 点「立即重启」
          → 小工进程换 exe → 新版本自己起来
  场景 B：云端没有新版时不打扰；在设置里点「检测更新」能查出来并弹卡

用法: python update_test.py [exe]
"""

import ctypes
import ctypes.wintypes as wt
import hashlib
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from winutil import find_bar  # noqa: E402

u = ctypes.windll.user32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

try:
    sys.stdout.reconfigure(line_buffering=True)
except Exception:
    pass

ROOT = HERE.parent
EXE = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/debug/tokenbar.exe").resolve()
RUN = ROOT / "tools" / ".cache" / "run" / "update"
NOTICE_TITLE = "Tokenbar 更新"
SETTINGS_TITLE = "Tokenbar 设置"

# 桩服务器当前"在线上的版本"：old → 老版本（不该提示），new → 9.9.9（该提示）
MODE = {"v": "old"}
PAYLOAD = {"v": b"", "sha": ""}
results = []


def report(name, ok, extra=""):
    results.append(bool(ok))
    print(f"[{'PASS' if ok else 'FAIL'}] {name}" + (f"  ({extra})" if extra else ""))


# ---------------------------------------------------------------- 桩服务器

class Stub(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path.startswith("/api"):
            ver = "9.9.9" if MODE["v"] == "new" else "0.0.1"
            body = json.dumps(
                {
                    "id": 1,
                    "tag_name": f"v{ver}",
                    "name": f"v{ver}",
                    "body": "桩数据",
                    "assets": [
                        # 故意混进源码包，验证"只认 exe"
                        {"name": f"v{ver}.zip", "browser_download_url": f"http://127.0.0.1:{PORT}/src.zip"},
                        {"name": "Tokenbar.exe", "browser_download_url": f"http://127.0.0.1:{PORT}/Tokenbar.exe"},
                    ],
                }
            ).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path.startswith("/Tokenbar.exe"):
            data = PAYLOAD["v"]
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        else:
            self.send_response(404)
            self.end_headers()


def serve():
    srv = HTTPServer(("127.0.0.1", PORT), Stub)
    srv.serve_forever()


# ---------------------------------------------------------------- 窗口

def win_rect(hwnd):
    r = wt.RECT()
    u.GetWindowRect(wt.HWND(hwnd), ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def client_origin(hwnd):
    pt = wt.POINT(0, 0)
    u.ClientToScreen(wt.HWND(hwnd), ctypes.byref(pt))
    return pt.x, pt.y


def scale(hwnd):
    return u.GetDpiForWindow(wt.HWND(hwnd)) / 96.0


def client_size(hwnd):
    r = wt.RECT()
    u.GetClientRect(wt.HWND(hwnd), ctypes.byref(r))
    s = scale(hwnd)
    return r.right / s, r.bottom / s


def find(title, wait=20.0):
    t0 = time.time()
    while time.time() - t0 < wait:
        h = u.FindWindowW(None, title)
        if h:
            return h
        time.sleep(0.2)
    return 0


def click_win(hwnd, lx, ly, hold=0.12):
    ox, oy = client_origin(hwnd)
    s = scale(hwnd)
    u.SetCursorPos(int(ox + lx * s), int(oy + ly * s))
    time.sleep(0.35)
    u.mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(hold)
    u.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.8)


def grab(hwnd, out):
    from PIL import ImageGrab

    l, t, r, b = win_rect(hwnd)
    img = ImageGrab.grab(bbox=(l, t, r, b)).convert("RGB")
    img.save(out)
    return img.size


def overlaps(a, b):
    return not (a[3] <= b[1] or a[1] >= b[3] or a[2] <= b[0] or a[0] >= b[2])


def kill():
    subprocess.run(["taskkill", "/F", "/IM", "Tokenbar.exe"], capture_output=True)
    time.sleep(1.0)


def copy_exe(dst, patched=False):
    data = EXE.read_bytes()
    if patched:
        cur = subprocess.run([str(EXE), "--version"], capture_output=True).stdout.decode().strip()
        old = cur.encode("utf-16-le")
        new = PATCH_VER.encode("utf-16-le")
        assert len(old) == len(new), f"补丁长度不一致：{cur} vs {PATCH_VER}"
        assert data.count(old) >= 1, "exe 里找不到版本串"
        data = data.replace(old, new)
    dst.write_bytes(data)


def start(run_dir, api):
    env = dict(os.environ)
    env["TOKENBAR_GITEE_API"] = api
    # 配置目录跟着 run_dir 走，别碰用户自己的 %APPDATA%\Tokenbar
    env["TOKENBAR_CONFIG_DIR"] = str(run_dir)
    p = subprocess.Popen(
        [str(run_dir / "Tokenbar.exe")],
        cwd=str(run_dir),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_bar()
    return p


def wait_bar(timeout=25.0):
    t0 = time.time()
    while time.time() - t0 < timeout:
        h = find_bar(include_hidden=True)
        if h:
            time.sleep(1.0)
            return h
        time.sleep(0.3)
    return 0


def fresh(name):
    d = RUN / name
    if d.exists():
        shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    copy_exe(d / "Tokenbar.exe")
    theme = os.environ.get("TOKENBAR_TEST_THEME", "light")
    (d / "tokenbar.ini").write_text(
        f"[window]\nx=560\ny=760\n\n[general]\ntheme={theme}\nlayout=horizontal\nfollow_workbuddy=0\n",
        encoding="utf-8",
    )
    return d


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def ini(d):
    try:
        return (d / "tokenbar.ini").read_text(encoding="utf-8")
    except FileNotFoundError:
        return ""


def ini_get(d, key):
    for line in ini(d).splitlines():
        if line.startswith(key + "="):
            return line.split("=", 1)[1].strip()
    return None


# ---------------------------------------------------------------- 场景 A

def case_auto():
    print("\n--- 场景 A：启动自动检查 → 提示 → 下载 → 重启 ---")
    d = fresh("auto")
    kill()
    MODE["v"] = "new"
    start(d, f"http://127.0.0.1:{PORT}/api")

    notice = find(NOTICE_TITLE, wait=14.0)
    report("启动后自动弹出更新提示", bool(notice), f"hwnd={notice}")
    if not notice:
        return

    bar = find_bar(include_hidden=True)
    nr, br = win_rect(notice), win_rect(bar)
    w, h = client_size(notice)
    report("提示卡尺寸 366x60", abs(w - 366) < 2 and abs(h - 60) < 2, f"{w:.0f}x{h:.0f}")
    report("提示卡不压住 bar", not overlaps(nr, br), f"notice={nr} bar={br}")
    report("提示卡在 bar 正上方居中",
           abs((nr[0] + nr[2]) / 2 - (br[0] + br[2]) / 2) < 24 and nr[3] <= br[1],
           f"dx={(nr[0]+nr[2])/2-(br[0]+br[2])/2:.0f}")
    report("提示卡不抢焦点", u.GetForegroundWindow() != notice, "前台窗口不是提示卡")
    print("  截图:", grab(notice, ROOT / "tools" / ".cache" / "notice_found.png"))

    # 点整卡中间 = 开始下载
    click_win(notice, w * 0.5, h * 0.5)
    new = d / "Tokenbar.exe.new"
    side = d / "tokenbar-update.json"
    ok = False
    for _ in range(60):
        if new.exists() and side.exists() and new.stat().st_size > 1_000_000:
            ok = True
            break
        time.sleep(0.25)
    report("下载完成并落盘", ok, f"{new.stat().st_size if new.exists() else 0} 字节")
    report("待更新版本记录正确",
           ok and json.loads(side.read_text())["version"] == PATCH_VER,
           side.read_text() if side.exists() else "缺文件")
    time.sleep(1.2)
    print("  截图:", grab(notice, ROOT / "tools" / ".cache" / "notice_ready.png"))

    # 点「立即重启」
    click_win(notice, 366 - 14 - 39, 30)
    ok = False
    for _ in range(80):
        if (d / "Tokenbar.exe").exists() and sha(d / "Tokenbar.exe") == PAYLOAD["sha"]:
            ok = True
            break
        time.sleep(0.25)
    report("重启后 exe 已换成新版", ok, sha(d / "Tokenbar.exe")[:12])
    # 新版自己起来要重新建窗口，给它几秒
    back = 0
    for _ in range(40):
        back = find_bar(include_hidden=True)
        if back:
            break
        time.sleep(0.25)
    report("换完又自己起来了", bool(back), f"hwnd={back}")
    report("换完删掉待更新标记", not (d / "tokenbar-update.json").exists())
    report("ini 记录了版本号与打包时间",
           (ini_get(d, "version") or "").startswith("1.") and len(ini_get(d, "packaged") or "") >= 10,
           f"version={ini_get(d,'version')} packaged={ini_get(d,'packaged')}")
    kill()


# ---------------------------------------------------------------- 场景 B

def case_settings():
    print("\n--- 场景 B：没新版不打扰；设置里手动检测 ---")
    d = fresh("settings")
    kill()
    MODE["v"] = "old"
    start(d, f"http://127.0.0.1:{PORT}/api")
    time.sleep(5.0)
    report("云端没有新版时不弹提示", find(NOTICE_TITLE, wait=0.5) == 0)

    # 开设置窗口：悬停出按钮 → 点下面的齿轮
    bar = find_bar(include_hidden=True)
    bw, bh = client_size(bar)
    click_win(bar, bw * 0.5, bh * 0.5, hold=0.05)
    click_win(bar, bw - 15.0, bh * 0.5 + 12.0)
    st = find(SETTINGS_TITLE, wait=10.0)
    report("设置窗口打开", bool(st), f"hwnd={st}")
    if not st:
        return
    report("设置窗口 420x700", abs(client_size(st)[0] - 420) < 2 and abs(client_size(st)[1] - 700) < 2,
           f"{client_size(st)[0]:.0f}x{client_size(st)[1]:.0f}")

    # 线上出现新版 → 点「检测更新」
    btn = (420 - 20 - 14 - 42, 592)
    MODE["v"] = "new"
    click_win(st, *btn)
    time.sleep(1.6)
    report("设置开着时不弹提示卡", find(NOTICE_TITLE, wait=0.5) == 0)
    print("  截图:", grab(st, ROOT / "tools" / ".cache" / "settings_found.png"))

    # 同一个按钮此时已变成「更新」
    click_win(st, *btn)
    new = d / "Tokenbar.exe.new"
    ok = False
    for _ in range(80):
        if new.exists() and new.stat().st_size > 1_000_000:
            ok = True
            break
        time.sleep(0.25)
    report("从设置里下载成功", ok, f"{new.stat().st_size if new.exists() else 0} 字节")
    report("下载的就是新版", ok and sha(new) == PAYLOAD["sha"], sha(new)[:12] if ok else "-")
    time.sleep(1.2)
    print("  截图:", grab(st, ROOT / "tools" / ".cache" / "settings_ready.png"))

    # 关掉设置（取消）→ 提示卡接班显示"已下载，重启生效"
    click_win(st, 420 - 20 - 92 - 8 - 78 + 39, 648 + 17)
    time.sleep(1.5)
    report("关掉设置后提示卡接管", bool(find(NOTICE_TITLE, wait=6.0)))

    # 点 bar 上的 ✕ 正常退出：退出时应该顺手把 exe 换掉（不重启）
    bar = find_bar(include_hidden=True)
    bw, bh = client_size(bar)
    click_win(bar, bw * 0.5, bh * 0.5, hold=0.05)
    click_win(bar, bw - 15.0, bh * 0.5 - 12.0)
    for _ in range(40):
        if sha(d / "Tokenbar.exe") == PAYLOAD["sha"]:
            break
        time.sleep(0.25)
    time.sleep(0.5)
    report("关闭程序时自动换好 exe",
           sha(d / "Tokenbar.exe") == PAYLOAD["sha"], sha(d / "Tokenbar.exe")[:12])


# ---------------------------------------------------------------- 场景 C

def case_pending():
    print("\n--- 场景 C：下载好后被强制关掉，下次启动接着提示 ---")
    d = fresh("pending")
    kill()
    # 伪造"上次已经下完、还没来得及换"
    (d / "Tokenbar.exe.new").write_bytes(PAYLOAD["v"])
    (d / "tokenbar-update.json").write_text(
        json.dumps({"version": PATCH_VER, "file": "Tokenbar.exe.new"}), encoding="utf-8"
    )
    MODE["v"] = "new"
    start(d, f"http://127.0.0.1:{PORT}/api")
    notice = find(NOTICE_TITLE, wait=12.0)
    report("启动就提示『已下载待重启』", bool(notice), f"hwnd={notice}")
    if notice:
        print("  截图:", grab(notice, ROOT / "tools" / ".cache" / "notice_resume.png"))
    # 这次点 ✕ 退出（正常退出路径），exe 应该被换掉
    kill()
    time.sleep(0.5)


def main():
    global PORT, PATCH_VER
    if not EXE.exists():
        print("找不到 exe:", EXE)
        return 1
    PATCH_VER = "9.9.9"
    PORT = 18731
    PAYLOAD["v"] = EXE.read_bytes()
    tmp = RUN / "payload.bin"
    RUN.mkdir(parents=True, exist_ok=True)
    copy_exe(tmp, patched=True)
    PAYLOAD["v"] = tmp.read_bytes()
    PAYLOAD["sha"] = sha(tmp)
    tmp.unlink()

    threading.Thread(target=serve, daemon=True).start()
    time.sleep(0.4)

    case_auto()
    case_settings()
    case_pending()

    kill()
    passed = sum(1 for x in results if x)
    print(f"\n{passed}/{len(results)} 通过")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
