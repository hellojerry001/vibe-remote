#!/usr/bin/env bash
# 建 GitHub Release 并上传签名后的更新产物
# 用法：GITHUB_TOKEN=xxx scripts/publish-gh.sh [版本号]
#   不传 GITHUB_TOKEN 时，会从 git credential-osxkeychain 里取 github.com 的 token。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')}"
TAG="v${VERSION}"
REPO_SLUG="hellojerry001/vibe-remote"
API="https://api.github.com/repos/${REPO_SLUG}"

if [ -z "${GITHUB_TOKEN:-}" ]; then
  GITHUB_TOKEN="$(printf 'protocol=https\nhost=github.com\n' \
    | git credential-osxkeychain get 2>/dev/null | grep '^password=' | cut -d= -f2-)"
fi
if [ -z "$GITHUB_TOKEN" ]; then
  echo "!! 拿不到 GITHUB_TOKEN" >&2
  exit 1
fi

BUNDLE="$ROOT/src-tauri/target/release/bundle/macos"
# macOS 的 updater 产物固定叫 VibeRemote.app.tar.gz（不带版本号）
PKG="VibeRemote.app.tar.gz"

json() { /Users/jerry/.workbuddy/binaries/python/versions/3.13.12/bin/python3 -c 'import json,sys; print(json.load(sys.stdin).get(sys.argv[1], ""))' "$1"; }

# 幂等：同名 tag 的旧 Release 先删掉
EXISTING="$(curl -sS -X GET "$API/releases/tags/$TAG" -H "Authorization: Bearer $GITHUB_TOKEN" | json id)"
if [ -n "$EXISTING" ]; then
  echo "==> 删除已存在的 Release $TAG (id=$EXISTING)"
  curl -sS -X DELETE "$API/releases/$EXISTING" -H "Authorization: Bearer $GITHUB_TOKEN" -o /dev/null
fi

echo "==> 创建 Release $TAG"
CREATE="$(curl -sS -X POST "$API/releases" \
  -H "Authorization: Bearer $GITHUB_TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"tag_name\":\"$TAG\",\"name\":\"VibeRemote $VERSION\",\"body\":\"VibeRemote $VERSION\",\"draft\":false,\"prerelease\":false}")"
RELEASE_ID="$(printf '%s' "$CREATE" | json id)"
if [ -z "$RELEASE_ID" ]; then
  echo "!! 创建 Release 失败：$CREATE" >&2
  exit 1
fi

BUNDLE_ROOT="$ROOT/src-tauri/target/release/bundle"
for ASSET in "macos/$PKG" "macos/$PKG.sig" "dmg/VibeRemote_${VERSION}_aarch64.dmg"; do
  SRC="$BUNDLE_ROOT/$ASSET"
  if [ ! -f "$SRC" ]; then
    echo "==> 跳过（不存在）$(basename "$ASSET")"
    continue
  fi
  NAME="$(basename "$ASSET")"
  echo "==> 上传 $NAME"
  curl -sS --fail -X POST "https://uploads.github.com/repos/${REPO_SLUG}/releases/$RELEASE_ID/assets?name=$NAME" \
    -H "Authorization: Bearer $GITHUB_TOKEN" \
    -H 'Content-Type: application/octet-stream' \
    --data-binary "@$SRC" -o /dev/null
done

echo "==> 完成：https://github.com/${REPO_SLUG}/releases/tag/$TAG"
