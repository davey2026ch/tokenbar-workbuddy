#!/usr/bin/env bash
# 在 macOS 上把 Tokenbar 打成 .app 与 .dmg，默认两个架构各一份：
#   dist/Tokenbar-macos-arm64.dmg    Apple Silicon（M 系列）
#   dist/Tokenbar-macos-x86_64.dmg   Intel
# （文件名不带版本号：版本看 Release 页/设置窗口，包名保持稳定）
#
# 用法：
#   bash scripts/build_macos.sh                # 双架构（默认）
#   bash scripts/build_macos.sh --arm64        # 只打 M 芯片
#   bash scripts/build_macos.sh --intel        # 只打 Intel
#   bash scripts/build_macos.sh --skip-build   # 复用已有二进制，只重新组装/签名（调试用）
#
# 签名：用固定自签证书（默认 "Tokenbar Self-Signed"），不要用 ad-hoc ——
# ad-hoc 的指定要求绑在 cdhash 上，每次重新打包都变，系统会当成另一个应用，
# 于是「屏幕录制 / 自动化」这类授权每次更新都要重给一遍。
# 证书不存在时脚本会提示先生成：bash scripts/make_signing_cert.sh
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="Tokenbar"
BUNDLE_ID="local.tokenbar.app"
SIGN_IDENTITY="${TOKENBAR_SIGN_IDENTITY:-Tokenbar Self-Signed}"
DIST="dist"
STAGE="${DIST}/_stage"
ICNS="${DIST}/${APP_NAME}.icns"

VERSION="$(sed -nE 's/^version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' Cargo.toml | head -1)"
[ -n "${VERSION}" ] || { echo "Cargo.toml 里没找到 version"; exit 1; }
BUILD_NUMBER="$(date -u +%Y%m%d%H%M)"
# 打包时间要烤进二进制（设置窗口「版本」里显示的那一行），
# 不传的话 cargo 会复用上次编译的旧时间戳
BUILD_STAMP="$(date '+%Y-%m-%d %H:%M:%S')"

TARGETS=()
SKIP_BUILD=0
for arg in "$@"; do
  case "$arg" in
    --arm64) TARGETS=(aarch64-apple-darwin) ;;
    --intel) TARGETS=(x86_64-apple-darwin) ;;
    --skip-build) SKIP_BUILD=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "不认识的参数：$arg"; exit 2 ;;
  esac
done
if [ ${#TARGETS[@]} -eq 0 ]; then
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
fi

# ---------------------------------------------------------------- 图标
# 一张 1024 的 PNG 出全套 .icns（sips + iconutil，系统自带，不用装东西）
make_icns() {
  [ -f "assets/icon-1024.png" ] || { echo "缺 assets/icon-1024.png，先跑 python tools/make_icon.py"; exit 1; }
  local set="${STAGE}/icon.iconset"
  rm -rf "$set" && mkdir -p "$set"
  local s
  for s in 16 32 128 256 512; do
    sips -z $s $s assets/icon-1024.png --out "${set}/icon_${s}x${s}.png" >/dev/null
    local d=$((s * 2))
    sips -z $d $d assets/icon-1024.png --out "${set}/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$set" -o "$ICNS"
  rm -rf "$set"
}

# ---------------------------------------------------------------- 签名
have_identity() {
  security find-identity -p codesigning 2>/dev/null | grep -qF "$SIGN_IDENTITY"
}

sign_app() {
  local app="$1"
  if have_identity; then
    codesign --force --deep --sign "$SIGN_IDENTITY" "$app"
    echo "    签名身份：${SIGN_IDENTITY}"
  else
    echo "    !! 没找到证书「${SIGN_IDENTITY}」→ 退回 ad-hoc（更新后要重新授权）"
    echo "       生成：bash scripts/make_signing_cert.sh"
    codesign --force --deep --sign - "$app"
  fi
}

# 关键验收点：指定要求必须绑在证书指纹上，出现 cdhash 就是 ad-hoc，直接失败
verify_signature() {
  local app="$1"
  codesign --verify --deep --strict "$app" 2>&1 | sed 's/^/    /' || true
  local dr
  dr="$(codesign -dr - "$app" 2>&1 | grep 'designated' || true)"
  echo "    DR: ${dr#*=> }"
  case "$dr" in
    *cdhash*)
      echo "    !! 签名是 ad-hoc（cdhash），更新后系统会重新要授权"
      return 1
      ;;
    *"certificate root"*) return 0 ;;
    *) echo "    !! 指定要求里没有证书指纹，签名不可用"; return 1 ;;
  esac
}

# ---------------------------------------------------------------- 组装
assemble() {
  local target="$1" arch="$2"
  local bin="target/${target}/release/tokenbar"
  local app="${STAGE}/${arch}/${APP_NAME}.app"

  [ -f "$bin" ] || { echo "缺二进制：$bin"; exit 1; }
  rm -rf "${STAGE:?}/${arch}"
  mkdir -p "${app}/Contents/MacOS" "${app}/Contents/Resources"
  cp "$bin" "${app}/Contents/MacOS/${APP_NAME}"
  chmod +x "${app}/Contents/MacOS/${APP_NAME}"
  cp "$ICNS" "${app}/Contents/Resources/${APP_NAME}.icns"

  cat > "${app}/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleIconFile</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${BUILD_NUMBER}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
    <key>NSAppleEventsUsageDescription</key>
    <string>用来判断 WorkBuddy 是否在最前面，好让卡片跟着显示或隐藏。</string>
</dict>
</plist>
PLIST

  # 去掉 Finder 复制过来的扩展属性，避免签名后 xattr 变化导致校验失败
  xattr -cr "$app" 2>/dev/null || true
}

make_dmg() {
  local arch="$1" name="Tokenbar-macos-$1.dmg"
  local src="${STAGE}/${arch}"
  local out="${DIST}/${name}"
  rm -f "$out"
  ln -sfn /Applications "${src}/Applications"
  hdiutil create -quiet -volname "${APP_NAME}" -srcfolder "$src" -ov -format UDZO "$out"
  rm -f "${src}/Applications"
  echo "    ${out}  ($(du -h "$out" | cut -f1 | tr -d ' '))"
  echo "    sha256 $(shasum -a 256 "$out" | cut -d' ' -f1)"
}

# ---------------------------------------------------------------- 主流程
mkdir -p "$DIST"
echo "==> Tokenbar ${VERSION}（构建号 ${BUILD_NUMBER}）"
make_icns
echo "    图标：$ICNS"

for target in "${TARGETS[@]}"; do
  case "$target" in
    aarch64-apple-darwin) arch="arm64" ;;
    x86_64-apple-darwin) arch="x86_64" ;;
    *) echo "不支持的 target：$target"; exit 2 ;;
  esac
  last_arch="$arch"

  if [ "$SKIP_BUILD" -eq 0 ]; then
    echo "==> 编译 ${target}"
    rustup target add "$target" >/dev/null 2>&1 || true
    TOKENBAR_BUILD_TIME="$BUILD_STAMP" cargo build --release --target "$target"
  fi

  echo "==> 组装 ${arch}"
  assemble "$target" "$arch"
  echo "==> 签名并校验"
  sign_app "${STAGE}/${arch}/${APP_NAME}.app"
  verify_signature "${STAGE}/${arch}/${APP_NAME}.app" || exit 1
  echo "==> 打包 dmg"
  make_dmg "$arch"
done

# 顺手放一份解包的 app，方便本机直接双击试
rm -rf "${DIST}/${APP_NAME}.app"
cp -R "${STAGE}/${last_arch}/${APP_NAME}.app" "${DIST}/${APP_NAME}.app"

echo
echo "完成。产物在 ${DIST}/"
ls -1 "${DIST}"/*.dmg 2>/dev/null || true
