"""重构前后视觉回归：同一份固定数据 + 同一份 ini，截同一块像素做对比。

用法: python vis_capture.py <exe> <out.png>
"""
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "tools"))
from winutil import find_bar  # noqa: E402

u = ctypes.windll.user32
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    pass

ROOT = Path(__file__).resolve().parent
# 截图用的假数据与临时运行目录都放在 tools/.cache（已 gitignore），不污染仓库
CACHE = ROOT / ".cache"
FIXTURE = CACHE / "fixture-home"
CONV = "conv-vis-0001-aaaa-bbbb"
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

LOG = {
    0: {"conv": CONV, "req": "R1", "msg": "M1", "model": "vis-model", "chat": "chat"},
}


def _json(conv, req, msg, model, typ, **kw):
    base = {
        "conversation_id": conv,
        "conversation_request_id": req,
        "prompt_request_id": "",
        "trace_id": "",
        "message_id": msg,
        "model": model,
        "agent_name": "cli",
        "scene": "craft",
        "request_type": typ,
        "report_version": "1",
        "report_source": "agent-cli",
    }
    base.update(kw)
    return "{" + ",".join(f'"{k}":"{v}"' for k, v in base.items()) + "}"


def build_fixture():
    import datetime

    if FIXTURE.exists():
        shutil.rmtree(FIXTURE, ignore_errors=True)
    day = datetime.date.today().isoformat()
    log_dir = FIXTURE / ".workbuddy" / "logs" / day
    log_dir.mkdir(parents=True)
    conv_dir = FIXTURE / ".workbuddy" / "projects" / "fake"
    conv_dir.mkdir(parents=True)
    (FIXTURE / ".workbuddy" / "uid").mkdir(parents=True)

    base = datetime.datetime.now().replace(hour=10, minute=0, second=0, microsecond=0)

    def ts(ms):
        return (base + datetime.timedelta(milliseconds=ms)).strftime("%Y/%m/%d %H:%M:%S.%f")[:-3]

    def line(ms, kind, body):
        return f"[{ts(ms)}] [Info] [pid=1] [ModelProvider]  [SseMonitor#01] {kind}: {body}\n"

    log = ""
    log += line(0, "qua.start", _json(CONV, "R1", "M1", "vis-model", "chat"))
    log += line(
        3000, "qua.end",
        _json(CONV, "R1", "M1", "vis-model", "chat", total_cost_ms="3000", ttft_ms="1000",
              completion_tokens="500", prompt_tokens="10000"),
    )
    log += line(4000, "qua.start", _json(CONV, "R1", "M2", "vis-model", "chat"))
    log += line(
        6000, "qua.end",
        _json(CONV, "R1", "M2", "vis-model", "chat", total_cost_ms="2000", ttft_ms="400",
              completion_tokens="300", prompt_tokens="12000"),
    )
    log += line(60000, "qua.start", _json(CONV, "R2", "M3", "vis-model", "chat"))
    log += line(
        61400, "qua.end",
        _json(CONV, "R2", "M3", "vis-model", "chat", total_cost_ms="1400", ttft_ms="800",
              completion_tokens="200", prompt_tokens="13000"),
    )
    (log_dir / "vis.log").write_text(log, encoding="utf-8")

    def usage(req, msg, inp, out, read):
        return (
            '{"type":"assistant","uuid":"u-%s","session_id":"c1","message":{"id":"%s",'
            '"role":"assistant","model":"vis-model","content":[],"usage":{'
            '"input_tokens":%d,"output_tokens":%d,"cache_creation_input_tokens":0,'
            '"cache_read_input_tokens":%d}},"_requestId":"%s"}\n'
            % (msg, msg, inp, out, read, req)
        )

    (conv_dir / f"{CONV}.jsonl").write_text(
        usage("R1", "M1", 10000, 500, 6000)
        + usage("R1", "M2", 12000, 300, 9000)
        + usage("R2", "M3", 13000, 200, 4000),
        encoding="utf-8",
    )
    (FIXTURE / ".workbuddy" / "uid" / "startup-route.json").write_text(
        '{"path":"/conversation/%s","savedAt":1}' % CONV, encoding="utf-8"
    )


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
    ini = os.environ.get("TOKENBAR_VIS_INI") or INI
    (run_dir / "tokenbar.ini").write_text(ini, encoding="utf-8")

    subprocess.run(["taskkill", "/F", "/IM", "tokenbar.exe"], capture_output=True)
    time.sleep(1.2)

    env = dict(os.environ)
    env["USERPROFILE"] = str(FIXTURE)
    env["HOME"] = str(FIXTURE)
    # 配置目录就放在运行目录里（和截图用的 ini 是同一个）
    env["TOKENBAR_CONFIG_DIR"] = str(run_dir)
    subprocess.Popen([str(local)], cwd=str(run_dir), env=env,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    hwnd = None
    for _ in range(40):
        hwnd = find_bar(include_hidden=True)
        if hwnd:
            break
        time.sleep(0.25)
    if not hwnd:
        print("window not found")
        return 1
    time.sleep(4.0)
    u.SetCursorPos(30, 30)
    time.sleep(1.0)
    hwnd = find_bar(include_hidden=True)

    from PIL import ImageGrab

    r = wt.RECT()
    u.GetWindowRect(wt.HWND(hwnd), ctypes.byref(r))
    img = ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom)).convert("RGB")
    img.save(out)
    print(f"saved {out} size={img.size} rect={(r.left, r.top, r.right, r.bottom)}")

    subprocess.run(["taskkill", "/F", "/IM", "tokenbar.exe"], capture_output=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
