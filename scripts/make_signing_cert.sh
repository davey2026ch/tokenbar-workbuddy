#!/usr/bin/env bash
# 生成一张固定的自签代码签名证书，并导入登录钥匙串（只需做一次）。
#
# 为什么需要它：
#   macOS 记权限不看路径，看「代码签名要求」。用 ad-hoc 签名（codesign -s -）时，
#   这个要求绑在文件内容的哈希（cdhash）上 —— 每打一次包都会变，系统就当成另一个
#   应用，于是「屏幕录制 / 自动化」这类授权在每次更新后都要重新给一遍。
#   换成固定证书后，要求变成「证书指纹 + bundle id」，跟文件内容无关，更新不再重授权。
#
# 用法：
#   bash scripts/make_signing_cert.sh                          # 默认身份 Tokenbar Self-Signed
#   TOKENBAR_SIGN_IDENTITY="别的名字" bash scripts/make_signing_cert.sh
#
# 说明：自签证书不会被系统信任（find-identity -v 里看不到，会标 CSSMERR_TP_NOT_TRUSTED），
#   但 codesign 能正常用它签名，且不需要任何密码弹窗。代价是从网上下载的包首次打开
#   仍会被 Gatekeeper 拦一次（那一层只有 Apple 开发者账号 + 公证才能消除）。
set -euo pipefail

IDENTITY="${TOKENBAR_SIGN_IDENTITY:-Tokenbar Self-Signed}"
KEYCHAIN="${HOME}/Library/Keychains/login.keychain-db"
P12_PASS="tokenbar"

if security find-identity -p codesigning 2>/dev/null | grep -qF "$IDENTITY"; then
  echo "已存在证书：${IDENTITY}"
  security find-identity -p codesigning 2>/dev/null | grep -F "$IDENTITY"
  echo "指纹：$(security find-certificate -c "$IDENTITY" -p 2>/dev/null | openssl x509 -noout -fingerprint -sha1 2>/dev/null | cut -d= -f2)"
  exit 0
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"

cat > openssl.cnf <<EOF
[ req ]
distinguished_name = dn
x509_extensions = ext
prompt = no
[ dn ]
CN = ${IDENTITY}
O = Tokenbar
[ ext ]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF

openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem \
    -days 3650 -nodes -config openssl.cnf 2>/dev/null

# 这三个参数不能省：openssl 3.x 默认用 AES 打包 p12，security import 会报
# "MAC verification failed"（跟密码无关，是算法不兼容）
openssl pkcs12 -export -out cert.p12 -inkey key.pem -in cert.pem \
    -passout "pass:${P12_PASS}" \
    -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1 \
    -name "$IDENTITY"

security import cert.p12 -k "$KEYCHAIN" -P "$P12_PASS" -T /usr/bin/codesign -A >/dev/null

echo "已导入证书：${IDENTITY}"
security find-identity -p codesigning 2>/dev/null | grep -F "$IDENTITY"
echo "指纹：$(security find-certificate -c "$IDENTITY" -p 2>/dev/null | openssl x509 -noout -fingerprint -sha1 2>/dev/null | cut -d= -f2)"
