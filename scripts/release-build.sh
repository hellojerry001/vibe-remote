#!/usr/bin/env bash
# 构建 + 签名打包 VibeRemote，并生成 updater 用的 update.json
# 用法：
#   TAURI_SIGNING_PRIVATE_KEY="$(cat ~/viberemote-updater/updater.key" \
#     scripts/release-build.sh [版本号]      # 不传则取 tauri.conf.json 里的 version
#
# 产物：
#   src-tauri/target/release/bundle/macos/VibeRemote_<v>_aarch64-apple-darwin.app.tar.gz
#   src-tauri/target/release/bundle/macos/VibeRemote_<v>_aarch64-apple-darwin.app.tar.gz.sig
#   update.json（仓库根目录，updater 检查入口）
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET="${1:-}"
if [ -z "$TARGET" ]; then
  TARGET="$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
fi

# Manifest/tag must describe the actual bundled version.
ACTUAL_VERSION="$(/usr/bin/python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json"))["version"])')"
if [ "$TARGET" != "$ACTUAL_VERSION" ]; then
  echo "!! 请求版本 $TARGET 与应用版本 $ACTUAL_VERSION 不一致，请先同步版本号" >&2
  exit 1
fi

export PATH="$HOME/.cargo/bin:$PATH"

# macOS 上 dmg 步骤会往 /Volumes 写临时文件，沙箱会拦，所以需要提权跑
# 私钥用 TAURI_SIGNING_PRIVATE_KEY 内联传（_PATH 变体在本版 CLI 不生效），
# 空密码必须显式写成空串，否则 CLI 会尝试弹 Keychain 解密
: "${TAURI_SIGNING_PRIVATE_KEY:=}"
if [ -z "$TAURI_SIGNING_PRIVATE_KEY" ] && [ -f "${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/viberemote-updater/updater.key}" ]; then
  TAURI_SIGNING_PRIVATE_KEY="$(cat "${TAURI_SIGNING_PRIVATE_KEY_PATH:-$HOME/viberemote-updater/updater.key}")"
fi
# 必须 export：tauri CLI 是子进程，shell 局部变量它看不到
export TAURI_SIGNING_PRIVATE_KEY
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-}"

echo "==> 打包 VibeRemote ${TARGET}（带 updater 签名）"
npx tauri build

BUNDLE="$ROOT/src-tauri/target/release/bundle/macos"
# 注意：macOS 的 updater 产物固定叫 <ProductName>.app.tar.gz，不带版本号
PKG="VibeRemote.app.tar.gz"
SIG="${PKG}.sig"

if [ ! -f "$BUNDLE/$PKG" ] || [ ! -f "$BUNDLE/$SIG" ]; then
  echo "!! 没找到更新产物，检查 $BUNDLE" >&2
  ls -la "$BUNDLE" >&2
  exit 1
fi

# Tauri must sign the .app before creating the updater archive. Signing only
# the loose .app afterwards leaves the published archive with the old signature.
VERIFY_DIR="$(mktemp -d)"
trap 'rm -rf "$VERIFY_DIR"' EXIT
tar -xzf "$BUNDLE/$PKG" -C "$VERIFY_DIR"
codesign --verify --deep --strict "$VERIFY_DIR/VibeRemote.app"
BUNDLE_ID="$(/usr/bin/python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json"))["identifier"])')"
SIGN_ID="$(codesign -dv "$VERIFY_DIR/VibeRemote.app" 2>&1 | sed -n 's/^Identifier=//p')"
if [ "$SIGN_ID" != "$BUNDLE_ID" ]; then
  echo "!! 更新包应用签名标识符不匹配：$SIGN_ID" >&2
  exit 1
fi

PUBDATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
REPO_SLUG="hellojerry001/vibe-remote"
URL="https://github.com/${REPO_SLUG}/releases/download/v${TARGET}/${PKG}"

/Users/jerry/.workbuddy/binaries/python/versions/3.13.12/bin/python3 - "$BUNDLE/$PKG" "$BUNDLE/$SIG" "$TARGET" "$PUBDATE" "$URL" "$ROOT/update.json" <<'PY'
import json, sys
pkg, sig, version, pubdate, url, out = sys.argv[1:7]
with open(sig) as f:
    signature = f.read().strip()
data = {
    "version": version,
    "notes": f"VibeRemote {version}",
    "pub_date": pubdate,
    "platforms": {
        "darwin-aarch64": {"url": url, "signature": signature},
    },
}
with open(out, "w") as f:
    json.dump(data, f, indent=2)
    f.write("\n")
print("==> update.json 已写入", out)
PY

echo "==> 产物："
ls -la "$BUNDLE/$PKG" "$BUNDLE/$SIG"
echo "==> 下一步：把 update.json 提交到 main，再跑 scripts/publish-gh.sh $TARGET"
