"""打包 Windows 版：编译 → 写入打包时间 → 复制成 dist/Tokenbar.exe → 签名。

发布只上传 exe（macOS 的 dmg 由 build_macos.sh 产出，release.py 会一起传）。
配置与统计文件由程序首次运行时自动生成在 `%APPDATA%\Tokenbar\`（内含版本号与打包时间），
所以不随包附带配置文件与说明文本。

用法：
  python scripts/package_win.py                 # 全流程（编译 + 打包 + 签名）
  python scripts/package_win.py --no-build      # 用现成的 target/release/tokenbar.exe
  python scripts/package_win.py --no-sign       # 不签名（本地验证用）
"""

import argparse
import datetime
import hashlib
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

try:  # 管道下也能按顺序看到日志
    sys.stdout.reconfigure(line_buffering=True)
except Exception:
    pass

ROOT = Path(__file__).resolve().parents[1]
BUILT = ROOT / "target" / "release" / "tokenbar.exe"
OUT = ROOT / "dist" / "Tokenbar.exe"


def version():
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        raise SystemExit("Cargo.toml 里没找到 version")
    return m.group(1)


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build(stamp):
    env = dict(os.environ)
    # build.rs 会把它写进 exe，程序再写进 ini 的 [meta] packaged=
    env["TOKENBAR_BUILD_TIME"] = stamp
    print(f"[1/3] 编译 release（打包时间 {stamp}）…")
    r = subprocess.run(["cargo", "build", "--release"], cwd=ROOT, env=env)
    if r.returncode != 0:
        raise SystemExit("编译失败")


def sign(exe):
    """返回 "signed" / "skipped" / "failed" """
    print("[3/3] 代码签名…")
    r = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "sign_win.py"), str(exe), "--allow-unsigned"],
        cwd=ROOT,
    )
    if r.returncode == 0:
        return "signed"
    if r.returncode == 3:
        return "skipped"
    print("  签名失败（不影响打包，但发出去会没有签名）")
    return "failed"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-build", action="store_true", help="不重新编译")
    ap.add_argument("--no-sign", action="store_true", help="跳过签名")
    ap.add_argument("--time", help="指定打包时间（默认现在）")
    args = ap.parse_args()

    ver = version()
    stamp = args.time or datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    print(f"Tokenbar v{ver}")

    if args.no_build:
        if not BUILT.is_file():
            raise SystemExit(f"缺少 {BUILT}，先跑 cargo build --release")
        print("[1/3] 跳过编译，用现成产物")
    else:
        build(stamp)

    print("[2/3] 复制到 dist/Tokenbar.exe")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(BUILT, OUT)

    state = "skipped" if args.no_sign else sign(OUT)

    print("\n打包完成")
    print(f"  文件   {OUT}")
    print(f"  版本   v{ver}")
    if args.no_build:
        print("  打包时间 沿用上次编译（exe 里记着的那一刻）")
    else:
        print(f"  打包时间 {stamp}")
    print(f"  大小   {OUT.stat().st_size/1024/1024:.2f} MiB")
    print(f"  sha256 {sha(OUT)}")
    print(
        "  签名   "
        + {
            "signed": "已签名",
            "skipped": "未签名（没证书 / 手动跳过；SmartScreen 会提示「未知发布者」）",
        }.get(state, "签名失败")
    )
    print("\n下一步：python scripts/release.py   # 上传 exe（有 dmg 一起传）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
