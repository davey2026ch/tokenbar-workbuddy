"""把 dist/ 里的发布包发到 Gitee Release（只发 exe 与 dmg）。

  - 版本号取自 Cargo.toml，tag 自动用 v<版本>
  - 上传：dist/Tokenbar.exe + dist/Tokenbar*.dmg（有哪个传哪个）
  - 已经存在的同名 release 会复用：先删掉旧的 exe/dmg 附件，再传新的
  - Gitee 会给每个 release 自动挂「源码 zip / tar.gz」，那是平台生成的下载链接，
    不是附件，API 删不掉，也不该当成发布包
  - 令牌只从参数/环境变量读，绝不写进仓库

用法：
  set GITEE_TOKEN=xxxx            (Windows)
  python scripts/release.py                    # 发当前版本
  python scripts/release.py --dry-run          # 只看会做什么
  python scripts/release.py --notes "修复xxx"  # 自定义更新说明

返回码：0 成功；1 失败。
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

try:  # 管道下也能按顺序看到日志
    sys.stdout.reconfigure(line_buffering=True)
except Exception:
    pass

ROOT = Path(__file__).resolve().parents[1]
OWNER, REPO = "mrpu2020", "tokenbar-workbuddy"
API = f"https://gitee.com/api/v5/repos/{OWNER}/{REPO}"
EXE = ROOT / "dist" / "Tokenbar.exe"
UPLOAD_TIMEOUT = 600
# 只发这两类：程序本体（Windows exe / macOS dmg）
UPLOAD_EXTS = (".exe", ".dmg")


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


def call(method, url, token, data=None, raw=None, content_type=None, timeout=60):
    body = None
    headers = {"Authorization": f"token {token}", "User-Agent": "Tokenbar-release"}
    if raw is not None:
        body = raw
        headers["Content-Type"] = content_type
    elif data is not None:
        body = json.dumps(data).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            text = r.read().decode("utf-8", "ignore")
            return r.status, (json.loads(text) if text.strip().startswith(("{", "[")) else text)
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", "ignore")
        return e.code, text


def multipart(fields, file_path, file_field="file"):
    """手搓 multipart/form-data，避免引第三方库"""
    boundary = "----TokenbarBoundary7d91f3"
    out = []
    for k, v in fields.items():
        out.append(f"--{boundary}\r\n".encode())
        out.append(f'Content-Disposition: form-data; name="{k}"\r\n\r\n'.encode())
        out.append(str(v).encode() + b"\r\n")
    out.append(f"--{boundary}\r\n".encode())
    out.append(
        f'Content-Disposition: form-data; name="{file_field}"; filename="{file_path.name}"\r\n'.encode()
    )
    out.append(b"Content-Type: application/octet-stream\r\n\r\n")
    out.append(file_path.read_bytes())
    out.append(b"\r\n")
    out.append(f"--{boundary}--\r\n".encode())
    return b"".join(out), f"multipart/form-data; boundary={boundary}"


def git_log(n=6):
    r = subprocess.run(
        ["git", "log", "--oneline", f"-{n}"], cwd=ROOT, capture_output=True
    )
    if r.returncode != 0:
        return ""
    lines = r.stdout.decode("utf-8", "ignore").strip().splitlines()
    return "\n".join("- " + line.split(" ", 1)[-1] for line in lines)


def uploads(exe):
    """这次要传的文件：exe + dist 里的 dmg（有哪个传哪个）"""
    files = [exe]
    if exe.parent.is_dir():
        files += sorted(p for p in exe.parent.glob("Tokenbar*.dmg") if p.is_file())
    return files


def is_product(name):
    """是不是我们自己的发布包（源码 zip/tar.gz 不算）"""
    return name.lower().endswith(UPLOAD_EXTS)


def body_of(ver, files, notes):
    exe = next((f for f in files if f.suffix.lower() == ".exe"), None)
    dmgs = [f for f in files if f.suffix.lower() == ".dmg"]
    lines = [f"**v{ver}**", "", notes, "", "---", "**下载**"]
    if exe:
        lines.append(
            f"- `{exe.name}`（{exe.stat().st_size/1024/1024:.2f} MiB，"
            f"sha256 `{sha(exe)}`）— Windows 10/11 x64，解压即用"
        )
    for d in dmgs:
        chip = ""
        if "arm64" in d.name:
            chip = " — Apple 芯片"
        elif "x86_64" in d.name:
            chip = " — Intel"
        lines.append(
            f"- `{d.name}`（{d.stat().st_size/1024/1024:.2f} MiB，"
            f"sha256 `{sha(d)}`）{chip}"
        )
    lines += [
        "",
        "Windows 旧版会自动提示更新；macOS 在设置里点「检测更新」，按钮变「去下载」后换 dmg。",
        "",
        "配置与统计文件：Windows `%APPDATA%\\Tokenbar\\`，macOS `~/.tokenbar/`。",
        "",
        "> 页面最下方 Gitee 自动附的「源码 zip / tar.gz」是平台生成的下载链接，"
        "不是发布包；程序请下上面的 exe / dmg。",
    ]
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--token", default=os.environ.get("GITEE_TOKEN", ""))
    ap.add_argument("--file", default=str(EXE))
    ap.add_argument("--notes", default="")
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()

    token = args.token.strip()
    exe = Path(args.file)
    ver = version()
    tag = f"v{ver}"

    if not exe.is_file():
        print("找不到要发布的文件：", exe)
        print("先跑：python scripts/package_win.py")
        return 1
    if not token:
        print("缺少令牌：设置环境变量 GITEE_TOKEN，或用 --token 传入")
        return 1

    files = uploads(exe)
    body = body_of(ver, files, args.notes.strip() or git_log())

    print(f"发布 v{ver} → {OWNER}/{REPO}")
    print(f"  只传 exe / dmg，共 {len(files)} 个：")
    for f in files:
        print(f"    {f.name}  ({f.stat().st_size/1024/1024:.2f} MiB)")
    if args.dry_run:
        print("  [dry-run] 不实际请求 Gitee")
        print("\n--- 发行说明预览 ---\n" + body)
        return 0

    # 1) 找已有的同名 release（Gitee 对不存在的 tag 返回 200 + null，不是 404）
    status, data = call("GET", f"{API}/releases/tags/{tag}", token)
    if status == 200 and isinstance(data, dict) and data.get("id"):
        rid = data["id"]
        print(f"  复用已有 release（id={rid}）")
        # 正文不会自动更新，显式改一次
        st, _ = call(
            "PATCH",
            f"{API}/releases/{rid}",
            token,
            data={"tag_name": tag, "name": tag, "body": body, "prerelease": False},
        )
        print("  更新发行说明：", "ok" if st == 200 else st)
        # 附件不能覆盖，只能删了重传；只删我们自己的 exe/dmg，
        # Gitee 自带的源码 zip/tar.gz 没有 id（不是附件），也不该动
        for a in data.get("assets", []):
            name = a.get("name", "")
            if not is_product(name) or "id" not in a:
                continue
            st, _ = call("DELETE", f"{API}/releases/{rid}/attach_files/{a['id']}", token)
            print(f"  删除旧附件 {name}：", "ok" if st in (200, 204) else st)
    elif status == 404 or data in (None, "", "null"):
        st, data = call(
            "POST",
            f"{API}/releases",
            token,
            data={
                "tag_name": tag,
                "name": tag,
                "body": body,
                "target_commitish": "main",
                "prerelease": False,
            },
        )
        if st not in (200, 201) or not isinstance(data, dict):
            print("建 release 失败：", st, data)
            return 1
        rid = data["id"]
        print(f"  新建 release（id={rid}）")
    else:
        print("查 release 失败：", status, data)
        return 1

    # 2) 逐个上传（exe / dmg），别的文件一概不发
    for f in files:
        print(f"  上传 {f.name} …")
        payload, ctype = multipart({"access_token": token}, f)
        st, data = call(
            "POST",
            f"{API}/releases/{rid}/attach_files",
            token,
            raw=payload,
            content_type=ctype,
            timeout=UPLOAD_TIMEOUT,
        )
        if st not in (200, 201):
            print("  上传失败：", st, str(data)[:400])
            return 1
    print("  上传完成")

    # 3) 复核：latest 是不是刚发的这个，附件是不是只有 exe / dmg
    st, latest = call("GET", f"{API}/releases/latest", token)
    if st == 200 and isinstance(latest, dict):
        names = [a.get("name", "") for a in latest.get("assets", [])]
        products = [n for n in names if is_product(n)]
        print(f"  线上最新：{latest.get('tag_name')}  附件：{names}")
        if latest.get("tag_name") != tag:
            print(f"  注意：latest 仍是 {latest.get('tag_name')}（可能有更晚的版本）")
        want = sorted(f.name for f in files)
        if sorted(products) != want:
            print(f"  注意：发布包跟预期不一致：{sorted(products)} != {want}")
    else:
        print("  复核失败：", st, data)

    print("\n说明：Gitee 会自动给每个 release 挂「源码 zip / tar.gz」下载链接，")
    print("      那是平台行为、不属于附件，API 删不掉；本脚本只上传 exe / dmg。")
    print(f"\n完成：https://gitee.com/{OWNER}/{REPO}/releases/tag/{tag}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
