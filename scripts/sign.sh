#!/usr/bin/env bash
#
# 固定「代码签名标识符」，解决 macOS TCC「授权后失效」问题。
#
# 背景：未用 Developer ID 签名的 App 走 ad-hoc 签名时，codesign 生成的
# 「Identifier」形如 webcoding-32ab98cee53d0536（带构建哈希），每重新编译一次
# 就会变。macOS 的「输入监控」权限（Input Monitoring / ListenEvent）就是按这个
# 签名标识符记账的，于是：授权 → 重新构建 → 系统眼里变成另一个 App → 权限消失，
# 表现为「系统设置里已经开了，App 却仍显示未授权」。
#
# 指定 --identifier 后 ad-hoc 签名也能得到稳定标识符，且和 Info.plist 的
# CFBundleIdentifier 一致。
#
# 用法：由 tauri.conf.json 的 postBuildCommand 调用，产物目录作为 $1 传入。

set -euo pipefail

BUNDLE_ID="com.webcoding.desktop"
# 注意：不要用 $0 推绝对路径，某些 shell 会把工程路径（含中文）转码破坏
BUNDLE_DIR="${1:-src-tauri/target/release/bundle}"

if [ ! -d "$BUNDLE_DIR" ]; then
  echo "[sign] 找不到产物目录 ${BUNDLE_DIR}，跳过固定标识符签名"
  exit 0
fi

APP="$(find "$BUNDLE_DIR" -maxdepth 2 -name '*.app' -type d 2>/dev/null | head -1)"
if [ -z "$APP" ]; then
  echo "[sign] 未找到 .app，跳过固定标识符签名"
  exit 0
fi

echo "[sign] 固定签名标识符 ${BUNDLE_ID} → ${APP}"
codesign --force --deep --sign - --identifier "${BUNDLE_ID}" "${APP}"

# ad-hoc 签名不能通过 --strict 校验，只做基本校验，失败不影响构建
codesign --verify --deep "${APP}" || echo "[sign] 注意：ad-hoc 签名校验有告警，属正常"

IDENT=$((codesign -dv "${APP}" 2>&1 || true) | sed -n 's/^Identifier=//p')
if [ "${IDENT}" = "${BUNDLE_ID}" ]; then
  echo "[sign] ✅ 签名标识符已固定为 ${IDENT}，重新构建不会丢失 TCC 授权"
else
  echo "[sign] ⚠️ 签名标识符为 ${IDENT}，与 ${BUNDLE_ID} 不一致"
fi
