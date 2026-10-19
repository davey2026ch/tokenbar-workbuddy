"""给 Tokenbar.exe 做代码签名（Windows Authenticode + 时间戳）。

为什么需要真证书：
  只有「受信任 CA 签发、且已经攒下下载信誉」的签名才能真正消掉 SmartScreen 的
  "Windows 已保护你的电脑"。自签名 / 无签名都会照样弹窗。这里把流程做全，
  证书到位后一条命令就能签，不用改代码。

证书从哪来（按顺序找）：
  1) --pfx 路径 + --password（或环境变量 TOKENBAR_PFX_PASS）
  2) 环境变量 TOKENBAR_CERT_SHA1：证书指纹（装在 CurrentUser / LocalMachine 的「个人」里）
  3) 自动挑：证书库里唯一一张「带私钥 + 用途是代码签名」的证书

用法：
  python scripts/sign_win.py dist/Tokenbar.exe
  python scripts/sign_win.py dist/Tokenbar.exe --pfx my.pfx --password xxx
  python scripts/sign_win.py --info            # 只看有哪些证书可用

返回码：0 已签名并验证通过；3 没找到证书、跳过（打包脚本据此提示"未签"）；其它 失败。
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

CODE_SIGNING_EKU = "1.3.6.1.5.5.7.3.3"
# 时间戳服务器，按顺序试（国内网络下 digicert 成功率较高）
TSA = [
    "http://timestamp.digicert.com",
    "http://timestamp.sectigo.com",
    "http://timestamp.acs.microsoft.com",
]


def find_signtool():
    """PATH 里没有就去 Windows SDK 目录里挑版本号最大的那个"""
    out = subprocess.run(["where", "signtool"], capture_output=True)
    if out.returncode == 0 and out.stdout.strip():
        first = out.stdout.decode("mbcs", errors="ignore").strip().splitlines()
        if first:
            return Path(first[0].strip())
    best = None
    for root in (
        r"C:\Program Files (x86)\Windows Kits\10\bin",
        r"C:\Program Files\Windows Kits\10\bin",
    ):
        p = Path(root)
        if not p.is_dir():
            continue
        for child in p.iterdir():
            exe = child / "x64" / "signtool.exe"
            if not exe.is_file():
                continue
            key = []
            for part in child.name.split("."):
                try:
                    key.append(int(part))
                except ValueError:
                    key.append(0)
            if best is None or key > best[0]:
                best = (key, exe)
    return best[1] if best else None


def list_certs():
    """返回 [(指纹, 主题, 有效期, 是否有私钥)]，只含代码签名用途的"""
    ps = (
        "Get-ChildItem Cert:\\CurrentUser\\My, Cert:\\LocalMachine\\My -ErrorAction SilentlyContinue | "
        "ForEach-Object { "
        "$eku = ($_.EnhancedKeyUsageList | ForEach-Object { $_.ObjectId } ) -join ','; "
        "if ($_.HasPrivateKey -and $eku -like '*" + CODE_SIGNING_EKU + "*') { "
        "\"$($_.Thumbprint)|$($_.Subject)|$($_.NotAfter.ToString('yyyy-MM-dd'))\" } }"
    )
    out = subprocess.run(
        ["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="ignore",
    )
    rows = []
    for line in out.stdout.splitlines():
        line = line.strip()
        if not line or "|" not in line:
            continue
        parts = line.split("|")
        if len(parts) >= 3:
            rows.append((parts[0], parts[1], parts[2]))
    return rows


def pick_cert(args):
    """返回 (身份说明, signtool 参数列表)"""
    if args.pfx:
        pfx = Path(args.pfx)
        if not pfx.is_file():
            raise SystemExit(f"找不到证书文件：{pfx}")
        pwd = args.password or os.environ.get("TOKENBAR_PFX_PASS", "")
        if not pwd:
            raise SystemExit("PFX 需要密码：用 --password 或环境变量 TOKENBAR_PFX_PASS 传入")
        return f"pfx {pfx.name}", ["/f", str(pfx), "/p", pwd]

    want = (args.thumbprint or os.environ.get("TOKENBAR_CERT_SHA1", "")).replace(" ", "").upper()
    certs = list_certs()
    if want:
        for thumb, subject, until in certs:
            if thumb.upper() == want:
                return f"{subject}（{until} 到期）", ["/sha1", thumb]
        raise SystemExit(f"证书库里没有指纹为 {want} 的代码签名证书（可用 --info 查看）")
    if len(certs) == 1:
        thumb, subject, until = certs[0]
        return f"{subject}（{until} 到期）", ["/sha1", thumb]
    if not certs:
        return None, None
    listing = "\n".join(f"  {t}  {s}  {u} 到期" for t, s, u in certs)
    raise SystemExit(f"证书库里有不止一张代码签名证书，请用 --thumbprint 指定：\n{listing}")


def sign(signtool, exe, cert_args):
    for tsa in TSA:
        cmd = [
            str(signtool),
            "sign",
            "/fd",
            "sha256",
            "/td",
            "sha256",
            "/tr",
            tsa,
            *cert_args,
            str(exe),
        ]
        print("  签名中…", tsa)
        r = subprocess.run(cmd, capture_output=True, text=True, errors="ignore")
        if r.returncode == 0:
            print("  签名成功")
            return True
        tail = (r.stdout + r.stderr).strip().splitlines()[-4:]
        print("  失败：", " / ".join(tail))
    return False


def verify(signtool, exe):
    r = subprocess.run(
        [str(signtool), "verify", "/pa", "/v", str(exe)],
        capture_output=True,
        text=True,
        errors="ignore",
    )
    ok = r.returncode == 0
    text = r.stdout + r.stderr
    print("  验证：", "通过" if ok else "不通过")
    for line in text.splitlines():
        s = line.strip()
        if s.startswith(("Issued to", "Issued by", "Signing Certificate Chain", "Successfully", "Error")):
            print("   ", s)
    return ok


def main():
    ap = argparse.ArgumentParser(description="给 Tokenbar.exe 做代码签名")
    ap.add_argument("exe", nargs="?", help="要签的 exe（一般是 dist/Tokenbar.exe）")
    ap.add_argument("--pfx", help="pfx/p12 证书文件")
    ap.add_argument("--password", help="pfx 密码")
    ap.add_argument("--thumbprint", help="证书指纹（装在证书库里的那种）")
    ap.add_argument("--info", action="store_true", help="只看可用证书，不签名")
    ap.add_argument("--allow-unsigned", action="store_true", help="没证书时返回 3 而不是 2（打包脚本用）")
    args = ap.parse_args()

    signtool = find_signtool()
    certs = list_certs()

    if args.info:
        print("signtool:", signtool or "没找到（装个 Windows SDK）")
        if not certs:
            print("证书库：没有可用的代码签名证书")
        for thumb, subject, until in certs:
            print(f"证书：{thumb}  {subject}  {until} 到期")
        return 0

    if not args.exe:
        ap.error("要给哪个 exe 签名？")
    exe = Path(args.exe)
    if not exe.is_file():
        print("找不到文件：", exe)
        return 1
    if not signtool:
        print("找不到 signtool.exe（装 Windows SDK 或把 SDK 的 bin\\x64 加进 PATH）")
        return 1

    print(f"目标：{exe}  ({exe.stat().st_size/1024/1024:.2f} MiB)")
    who, cert_args = pick_cert(args)
    if who is None:
        msg = (
            "没有可用的代码签名证书，跳过签名。\n"
            "  说明：SmartScreen 只认「受信任 CA 签发 + 有下载信誉」的签名，\n"
            "        自签名或没有签名都会照样提示。买到证书后重跑本脚本即可。\n"
            "  用法：python scripts/sign_win.py dist/Tokenbar.exe --pfx 证书.pfx --password 密码"
        )
        print(msg)
        return 3 if args.allow_unsigned else 2

    print("签名证书：", who)
    if not sign(signtool, exe, cert_args):
        return 1
    return 0 if verify(signtool, exe) else 1


if __name__ == "__main__":
    sys.exit(main())
