# VibeRemote

用 **Siri Remote 控制 Mac**，把遥控事件映射为面向 AI Agent（Codex / WorkBuddy / Cursor…）的动作：批准、拒绝、继续、发送、语音输入、快捷键、AppleScript、Shell 命令。

## 两种输入源（可插拔）

| 输入源 | 链路 | 需要 | 状态 |
|---|---|---|---|
| **A2854 蓝牙直连**（推荐） | Siri Remote → 蓝牙 HID → macOS `IOHIDManager` → C 桥接 → Mapping Engine | 遥控器与 Mac 蓝牙配对 | ✅ 主力 |
| **tvOS App**（可选） | tvOS App → 局域网 WebSocket → Mapping Engine | Apple TV + 开发者账号 | ✅ 保留 |

两个来源产出完全相同的「遥控事件字符串」，下游只认事件名。因此换成网页遥控器、iPhone、Stream Deck 都只需加一个输入源，动作与 Preset 体系不用改。

```text
Siri Remote A2854 ──蓝牙 HID──> IOHIDManager ──C bridge──┐
tvOS App          ──WebSocket────────────────────────────┼──> Remote Event
网页遥控器 / 未来外设 ──WebSocket─────────────────────────┘
                                  │
                          Mapping Engine（Preset）
                                  │
      ┌────────────┬──────────┬───┴────────┬──────────────┐
   Approve      Reject     Continue      Voice Input    Shortcut / AppleScript / Shell
                                  │
                        Codex / WorkBuddy / Cursor / 终端
```

遥控事件：`up` `down` `left` `right` `center` `back` `playPause` `centerLongPress` `siri`

## 运行

```bash
npm install
npm run tauri dev        # 开发（Vite 2410 + Rust 自动重编）
npm run build            # 前端构建
npm run tauri:build      # 打包 + 固定签名标识符（产出 App 与 DMG）
```

> 打包请用 `tauri:build`（= `tauri build && bash scripts/sign.sh`）。
> `scripts/sign.sh` 会用 `--identifier com.webcoding.desktop` 重签，
> 让代码签名标识符与 Info.plist 的 CFBundleIdentifier 一致。
> **这一步不能省**：见下方「输入监控授权为什么会失效」。

产物：

```text
src-tauri/target/release/bundle/macos/VibeRemote.app
src-tauri/target/release/bundle/dmg/VibeRemote.dmg
```

> 打包出来的 **未签名、未公证**，首次打开可能被 Gatekeeper 拦下：右键 → 打开，或
> `xattr -d com.apple.quarantine VibeRemote.app` 后再启动。
> 需要消除拦截提示可做签名 + 公证：`codesign --deep --force --options runtime --sign -` 后 `xcrun notarytool` 提交。

## 用 A2854 直连（推荐路径）

1. 若遥控器还连着 Apple TV，先在那里取消配对；
2. Mac 系统设置 → **蓝牙**，让遥控器进入配对状态并连上；
3. 启动 VibeRemote；
4. 授权两项权限：
   - **隐私与安全性 → 输入监控**（读取遥控器按键，必填）
   - **隐私与安全性 → 辅助功能**（发送键盘事件、点击按钮）
   > 输入监控属于 macOS TCC 项目，**授权后必须重启 VibeRemote 才生效**。
5. 按一下遥控器任意键——状态页「HID 原始事件」应实时出现 usage，说明链路已通；
6. 到「遥控器映射」页确认每个按键的默认动作，按住确认键 0.6 秒可触发 `centerLongPress`。

硬件信息：VID `0x004C` / PID `0x0315`（蓝牙），USB 诊断 VID `0x05AC` 同样识别。

### 设备发现向导（App 内）

设备状态页顶部是首次使用的三步向导：

1. **让遥控器进入配对模式**：同时按住 `返回` + `音量+`，约 5 秒，白色指示灯开始闪烁 → 点「开始搜索」。
2. **配对**：向导会把系统蓝牙里已知的设备列出来（名称匹配 `Siri Remote`，
   或用 Apple VID `0x004C` + PID `0x0315` 兜底校验）。点「打开系统蓝牙设置」，
   在面板里点一下遥控器即可完成配对与连接。
3. **验证**：配对 → 链路连接 → `VibeRemote Ready`。此时按一下**确认键**，
   下方「HID 原始事件」应立即出现一行 usage——这一个动作同时验证了
   蓝牙、输入监控授权、A2854 匹配三层。

向导每 1.5 秒读一次系统蓝牙列表，所以在系统蓝牙里一旦连上，Wizard 会自动从
「配对中」推进到「已配对 → 连接中 → 可用」，不需要手动刷新。

> **为什么不能在 App 内直接配对？**
> macOS 13+ 上，进程只要碰蓝牙（`IOBluetooth.framework` 的
> `IOBluetoothDeviceInquiry` / `IOBluetoothDevicePair`，或 CoreBluetooth 的
> `CBCentralManager`），TCC 会走 `__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__`
> **直接 abort 整个进程**，不是返回错误码 —— 表现就是「点一下开始搜索，App 直接蹦掉」。
> 实测无效的规避手段：往 Info.plist 加 `NSBluetooth*UsageDescription`、ad-hoc 签名、
> 放到 `/Applications` 下、起完整 `NSApplication`。
> 所以扫描改成读 `system_profiler SPBluetoothDataType` 的输出（独立进程，不受同一 TCC 判定约束，
> 能拿到 名称 / 地址 / VID / PID / 是否连接）。
> 代价是：拿不到 RSSI，**也不能代按遥控器上的配对键**，配对只能去系统蓝牙面板完成。

### 三层状态刻意不合并

UI 上永远同时显示三行，各自独立判断通没通：

| 层 | 含义 | 判定来源 |
|---|---|---|
| **Bluetooth** | A2854 是否已在系统蓝牙列表里 | `system_profiler SPBluetoothDataType` 的解析结果 |
| **Connection** | 链路是否已连上 | `IOHIDManager` 侧接入设备数 |
| **HID Input** | 按键是否真的读得到 | 输入监控权限 + 输入回调是否在跑 |

状态机是十值的：`idle → instructions → scanning → found → pairing → paired → connecting → connected → hid_ready`，
外加 `failed`。全部由 Rust 侧真实读数推导（`remote/connection.rs`），
前端只渲染 `get_connection_state` 的返回值，不推断、不合并。
设备掉线会自动从 `hid_ready` 退回 `paired`，不会继续显示可用。

## 用 tvOS App（可选路径）

```bash
brew install xcodegen && xcodegen generate && open VibeRemote.xcodeproj
```

选 `WebCodingTV` target 运行到 Apple TV，与 Mac 同 Wi-Fi 即自动连接。**tvOS App 需保持前台**才能收事件，且真机运行需要 Apple Developer 账号。

## 内置自动更新

「检查更新」走 Tauri 官方 updater 插件：**在应用内下载安装包、替换应用、自动重启**，不跳转到 GitHub 下载页。
开关「有新版本时自动检查」会在启动时静默查一次，发现更新就直接装好。

```
前端      关于页 check() / update.downloadAndInstall()
Rust      tauri-plugin-updater
配置      tauri.conf.json 的 plugins.updater：
            - endpoints = https://raw.githubusercontent.com/hellojerry001/vibe-remote/main/update.json
            - pubkey    = 签名公钥（必填，否则拒绝安装）
            - bundle.createUpdaterArtifacts = true（默认 false，不打开就产不出更新包）
打包      src-tauri/target/release/bundle/macos/VibeRemote.app.tar.gz[.sig]
权限      capabilities/default.json 里 updater:default
```

`update.json` 是 updater 唯一的检查入口，格式固定为「静态多平台」：

```json
{
  "version": "0.2.0",
  "notes": "VibeRemote 0.2.0",
  "pub_date": "2026-09-26T10:57:47Z",
  "platforms": {
    "darwin-aarch64": {
      "url": "https://github.com/.../releases/download/v0.2.0/VibeRemote.app.tar.gz",
      "signature": "<.sig 文件内容>"
    }
  }
}
```

> 注意：`releases/latest` 那种 GitHub Release JSON **不能**直接用，
> updater 只认 `version|name` + `platforms{os-arch}`（或 `url`+`signature`）两种形状。
> `darwin-aarch64` 这个 key 必须存在，updater 会依次尝试 `darwin-aarch64-app`、`darwin-aarch64`。

**发布一个新版本的完整流程**（两个脚本，缺一不可）：

```bash
# 1) 生成签名密钥（只需一次），把公钥填进 tauri.conf.json 的 plugins.updater.pubkey
npx tauri signer generate -p "" -w ~/viberemote-updater/updater.key -v
#    私钥留着，后面打包要用

# 2) 升版本号（package.json / src-tauri/Cargo.toml / src-tauri/tauri.conf.json 三处）
# 3) 构建 + 签名 + 生成 update.json
TAURI_SIGNING_PRIVATE_KEY="$(cat ~/viberemote-updater/updater.key)" \
  scripts/release-build.sh

# 4) 把 update.json 提交并推到 main（endpoints 指向的就是 main 分支上的这个文件）
git add update.json && git commit -m "release: v0.2.0" && git push

# 5) 建 GitHub Release 并上传 VibeRemote.app.tar.gz / .sig / .dmg
scripts/publish-gh.sh 0.2.0
```

> macOS 的 updater 产物固定叫 `VibeRemote.app.tar.gz`，不带版本号；
> dmg 则叫 `VibeRemote_<version>_aarch64.dmg`。
> 打包时 dmg 步骤会写 `/Volumes`，需要绕过沙箱执行 `tauri build`。

## 关键实现

- **键盘模拟**：直接 FFI CoreGraphics `CGEvent`（不依赖高层 crate，避免 API 版本坑）。
- **批准 / 拒绝**：System Events 在前台 App 中查找 Approve / Allow / 批准 / Reject / Decline / 取消 等按钮并点击，找不到回退 Return / Escape。
- **语音输入**：双击 Control，需在「系统设置 → 键盘 → 听写」把快捷键设为「连按两次 Control 键」。遥控器麦克风不由系统开放给第三方，本方案走「触发 macOS 听写」而非直接录音。
- **后台常驻**：关闭窗口只是隐藏，托盘菜单或全局快捷键 `Cmd+Shift+W` 唤回。
- **配置**：`~/Library/Application Support/com.webcoding.desktop/webcoding-config.json`，内置 Codex / WorkBuddy / 自定义三个 Preset。

## 输入监控授权为什么会「系统设置里开着、App 却说未授权」

这是本项目踩过的最大的坑，根因有两层：

**1. ad-hoc 签名的标识符每次构建都会变。**
未用 Developer ID 签名时，codesign 生成的标识符形如 `webcoding-32ab98cee53d0536`（带构建哈希）。
macOS 的「输入监控」（Input Monitoring / `kIOHIDRequestTypeListenEvent`）就是按这个**签名标识符**记账的，
不是按 bundle id。于是：授权 → 重新构建 → 系统眼里变成另一个 App → 勾选自动失效。
修复：`scripts/sign.sh` 用 `--identifier com.webcoding.desktop` 固定签名标识符。
后续每次 `npm run tauri:build` 都会重新签一次，**授权不会因重新构建丢失**。

**2. 权限判断不能用 `IOHIDManagerOpen()` 的失败结果反推。**
"没配对 A2854""设备被占用""打开失败"都会被误报成未授权。
本项目改为先用 `IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)` 取真实三态
（`granted` / `denied` / `unknown`），`IOHIDManagerOpen()` 的失败另外单独上报。

前端「设备诊断」区域的所有字段（Input Monitoring / Accessibility / A2854 Matched /
IOHIDManager / Input Callback / Last HID Event / Bundle ID / App Path）都由
Rust 经 `get_system_status` 真实读取，前端不做任何推断。

> 历史行为备注：旧版 App 在权限不足时只能靠重启恢复。现在有 3 秒看门狗，
> 用户在系统设置里勾选后会自动重开 HID 监听；极端情况下 macOS 仍要求重启一次。

## 已知限制

- 「输入监控」权限必须重启 App；Xcode 直接 `Run` 启动时是调试进程，同样受限；
- 触控板使用 macOS 私有 MultitouchSupport 框架，系统升级可能改变接口；双指滚动暂不支持；
- Shell / AppleScript 映射无超时保护；
- 局域网无配对认证，同网段两台 Mac 会 Bonjour 服务名冲突；
- 长按判定为 600ms，双击手势尚未实现。

## 触控板鼠标控制

在「遥控器映射」中设置启用开关、灵敏度（0.1–30 倍）和轻点点击，即时保存生效。需要 macOS 辅助功能权限。
单指滑动移动鼠标，短于 300ms 且移动很小的轻点触发左键。物理确认键仍走 Preset 映射，并抑制当前触摸的轻点，避免重复操作。
多指接触取消当前手势，全部抬起后重新开始；断开连接会清空触摸状态。
中央圆形触控区域通过 MultitouchSupport 接收完整触摸帧，使用 IORegistry 中遥控器的 ProductID、Family ID（145）及 Multitouch ID 匹配，排除 Mac 内置触控板。每两秒重新发现设备，支持断开重连。标准 HID Digitizer 绝对坐标仅作后备。
设备状态页显示触控连接、接收帧数和鼠标移动发送次数；diag.json 的 touchpad 字段也保存同样的真实计数。默认灵敏度 10 时，横跨整个触摸面约移动 1000 屏幕坐标单位。
私有 ABI 参考：[Remotastic MultitouchSupport 声明](https://github.com/lauschue/Remotastic/blob/main/MultitouchSupport.h)。框架或符号不可用时报告错误，不影响按键功能。
