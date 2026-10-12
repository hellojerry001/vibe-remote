// 用 macOS Accessibility API（AXUIElement）在前台 App 的 AX 树里找按钮并点击。
//
// 为什么不用 osascript / System Events：
//   那条路要 TCC「自动化（Automation）」授权（System Events 去控制目标 App），
//   与「辅助功能」是两套独立开关；而且它同样只能看到 App 暴露出来的 AX 树。
//   AX API 只受「辅助功能」管辖（AXIsProcessTrusted），键盘注入能用的机器必定已授权。
//
// ⚠️ 关键前提 —— 前几版都失败在这里（2026-10-12 调研坐实）：
//   Electron / Chromium 应用**默认关闭** Web 内容的辅助功能树
//   （renderer accessibility 默认 off，只有检测到 VoiceOver 等辅助技术时才开）。
//   此时 AX 树只剩窗口框和菜单栏，DOM 里的「允许 / 拒绝」按钮根本不在树里——
//   osascript 与 AX 遍历看到的都是同一棵空树，所以两条路都点不到。
//   解法（Electron 官方文档 / justzht.com / AGI-Super-Team cua-driver 一致）：
//   向目标 App 的 AXUIElement 写 AXManualAccessibility = true，并配合
//   AXEnhancedUserInterface = true（有的 App 读前者、有的读后者，两个都要设），
//   Chromium 才会**异步**（约几百毫秒）把 DOM 按钮建进 AX 树，然后再遍历点击。
//   这两个属性同样只需要「辅助功能」授权，不需要「自动化」。
//
// ★ 2026-10-12 实机复测补充（前三处都是硬伤，逐条修）：
//   1) 旧版只用 `AXUIElementCopyAttributeValue(systemWide, kAXFocusedApplicationAttribute)`
//      解析目标 App；该调用在 Electron 目标上会直接失败 → 连属性都没设上就落到
//      osascript 兜底（表现为「耗时 270ms 的 not-found」）。现在三级解析：
//        ① system-wide 焦点 App（标准路径）
//        ② system-wide 焦点窗口 → AXUIElementGetPid → AXUIElementCreateApplication
//        ③ CGWindowListCopyWindowInfo 最靠前的可见窗口 owner pid（跳过自己）
//      失败时把 AXError 码写进出参，上层直接展示，不再两眼一黑。
//   2) Chromium 的 DOM button 有时吃不到 AXPress（返回 kAXErrorSuccess 但不触发
//      onClick）。追加坐标点击兜底：读 kAXPosition / kAXSize 算中心点，
//      CGEventPost 合成真实左键 down/up（与真实用户点击同一条路径）。
//   3) 模态确认卡在 macOS 上常以 **sheet** 挂在窗口上，kAXSheetAttribute 也要扫。
//
// 返回值：
//   1 = 点到（out = 按钮文案）
//   0 = 尽力了但没匹配到（out = 诊断串：来源/pid/窗口数/元素数/可点数/属性写入结果/角色清单）
//  -1 = 解析不到目标 App（out = 诊断串，含两个 AXError 码）
//  -2 = 辅助功能未授权（AXIsProcessTrusted = false）

#include <ApplicationServices/ApplicationServices.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

// ==================== 文本匹配 ====================

// role 白名单：Electron / 自定义弹窗里可点元素不一定是原生 button。
// 只接受「按下确实可能生效」的 role（非可点元素 AXPress 会返回错误，会被自动跳过）。
static int is_clickable_role(const char* role) {
  if (!role || !*role) return 0;
  return strcmp(role, "AXButton") == 0 ||
         strcmp(role, "AXRadioButton") == 0 ||
         strcmp(role, "AXCheckBox") == 0 ||
         strcmp(role, "AXLink") == 0 ||
         strcmp(role, "AXMenuItem") == 0 ||
         strcmp(role, "AXPopUpButton") == 0 ||
         strcmp(role, "AXDisclosureTriangle") == 0 ||
         strcmp(role, "AXStaticText") == 0;
}

static int contains_any(const char* hay, const char* const* words, int count) {
  if (!hay || !*hay) return 0;
  for (int i = 0; i < count; i++) {
    if (words[i] && *words[i] && strstr(hay, words[i])) return 1;
  }
  return 0;
}

static int equals_any(const char* hay, const char* const* words, int count) {
  if (!hay || !*hay) return 0;
  for (int i = 0; i < count; i++) {
    if (words[i] && *words[i] && strcmp(hay, words[i]) == 0) return 1;
  }
  return 0;
}

// ==================== 诊断统计 ====================

#define MAX_ROLES 10

typedef struct {
  int elements;  // 访问过的元素总数
  int windows;   // 窗口数
  int clickable; // 可点元素数
  char roles[MAX_ROLES][40];  // 见过的 role（去重，用于判断「树是不是空的」）
  int role_count;
} WalkStats;

static void note_role(WalkStats* st, const char* role) {
  if (!st || !role || !*role || st->role_count >= MAX_ROLES) return;
  for (int i = 0; i < st->role_count; i++) {
    if (strcmp(st->roles[i], role) == 0) return;
  }
  snprintf(st->roles[st->role_count], sizeof(st->roles[0]), "%s", role);
  st->role_count++;
}

// ==================== 点击 ====================

/// 在屏幕坐标上合成一次真实左键点击（CGEventPost 走 HID 事件流，
/// 与真实用户点击同一条路径，能触发 Chromium 的 DOM onClick）。
static int click_at_point(int x, int y) {
  CGPoint pt = CGPointMake((CGFloat)x, (CGFloat)y);
  CGEventRef down = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseDown, pt, kCGMouseButtonLeft);
  CGEventRef up = CGEventCreateMouseEvent(NULL, kCGEventLeftMouseUp, pt, kCGMouseButtonLeft);
  if (!down || !up) {
    if (down) CFRelease(down);
    if (up) CFRelease(up);
    return 0;
  }
  CGEventPost(kCGHIDEventTap, down);
  CGEventPost(kCGHIDEventTap, up);
  CFRelease(down);
  CFRelease(up);
  return 1;
}

/// 点击一个 AX 元素：先 AXPress（原生 button 最可靠）；
/// 失败再读坐标合成真实鼠标点击（Chromium DOM button 的兜底）。
/// 两条路都不做「重复点」——任一路成功即返回，避免双击。
static int click_element(AXUIElementRef el) {
  if (AXUIElementPerformAction(el, kAXPressAction) == kAXErrorSuccess) return 1;

  // AXPosition / AXSize 返回的是 AXValue（kAXValueCGPointType / kAXValueCGSizeType），
  // 不是 CFNumber——用 kCFNumberCGPointType 读会直接编译不过。
  CFTypeRef pos_v = NULL, size_v = NULL;
  int ok = 0;
  if (AXUIElementCopyAttributeValue(el, kAXPositionAttribute, &pos_v) == kAXErrorSuccess && pos_v &&
      AXUIElementCopyAttributeValue(el, kAXSizeAttribute, &size_v) == kAXErrorSuccess && size_v &&
      AXValueGetType((AXValueRef)pos_v) == kAXValueCGPointType &&
      AXValueGetType((AXValueRef)size_v) == kAXValueCGSizeType) {
    CGPoint p = CGPointZero;
    CGSize s = CGSizeZero;
    if (AXValueGetValue((AXValueRef)pos_v, kAXValueCGPointType, &p) &&
        AXValueGetValue((AXValueRef)size_v, kAXValueCGSizeType, &s) &&
        s.width > 1.0 && s.height > 1.0) {
      ok = click_at_point((int)(p.x + s.width / 2.0), (int)(p.y + s.height / 2.0));
    }
  }
  if (pos_v) CFRelease(pos_v);
  if (size_v) CFRelease(size_v);
  return ok;
}

// 取 CFString 属性到 buf（UTF8）。返回 1 表示拿到非空字符串。
static int copy_str(AXUIElementRef el, CFStringRef attr, char* buf, size_t len) {
  buf[0] = 0;
  CFTypeRef v = NULL;
  if (AXUIElementCopyAttributeValue(el, attr, &v) != kAXErrorSuccess || !v) return 0;
  int ok = 0;
  if (CFGetTypeID(v) == CFStringGetTypeID()) {
    ok = CFStringGetCString((CFStringRef)v, buf, len, kCFStringEncodingUTF8);
  }
  CFRelease(v);
  return ok;
}

// ==================== 遍历 ====================

// 递归查找并点击。exact=1 只做精确匹配，exact=0 做子串匹配。
static int search_press(AXUIElementRef el, const char* const* words, int count,
                        char* out, size_t outlen, int depth, int exact, WalkStats* st) {
  if (!el || depth > 60) return 0;
  if (st) st->elements++;

  char role[64] = {0}, title[256] = {0}, value[256] = {0}, desc[256] = {0};
  int has_role = copy_str(el, kAXRoleAttribute, role, sizeof(role));
  copy_str(el, kAXTitleAttribute, title, sizeof(title));
  copy_str(el, kAXValueAttribute, value, sizeof(value));
  copy_str(el, kAXDescriptionAttribute, desc, sizeof(desc));

  if (has_role) {
    if (st) note_role(st, role);
    if (is_clickable_role(role)) {
      if (st) st->clickable++;
      const char* cands[3] = {title, value, desc};
      for (int c = 0; c < 3; c++) {
        if (!cands[c][0]) continue;
        int hit = exact ? equals_any(cands[c], words, count)
                        : contains_any(cands[c], words, count);
        if (hit && click_element(el)) {
          snprintf(out, outlen, "%s", cands[c]);
          return 1;
        }
      }
    }
  }

  CFTypeRef children = NULL;
  int found = 0;
  if (AXUIElementCopyAttributeValue(el, kAXChildrenAttribute, &children) == kAXErrorSuccess && children) {
    if (CFGetTypeID(children) == CFArrayGetTypeID()) {
      CFIndex n = CFArrayGetCount((CFArrayRef)children);
      for (CFIndex i = 0; i < n; i++) {
        AXUIElementRef child = (AXUIElementRef)CFArrayGetValueAtIndex((CFArrayRef)children, i);
        if (search_press(child, words, count, out, outlen, depth + 1, exact, st)) {
          found = 1;
          break;
        }
      }
    }
    CFRelease(children);
  }
  return found;
}

// 在一组元素上做「先精确、再子串」两遍匹配
static int press_in_array(CFArrayRef arr, const char* const* words, int count,
                          char* out, size_t outlen, WalkStats* st) {
  CFIndex n = CFArrayGetCount(arr);
  for (int pass = 0; pass < 2; pass++) {
    int exact = (pass == 0);
    for (CFIndex i = 0; i < n; i++) {
      AXUIElementRef el = (AXUIElementRef)CFArrayGetValueAtIndex(arr, i);
      if (search_press(el, words, count, out, outlen, 0, exact, st)) return 1;
    }
  }
  return 0;
}

// 取 owner 上的数组属性并遍历
static int press_attr(AXUIElementRef owner, CFStringRef attr, const char* const* words, int count,
                      char* out, size_t outlen, WalkStats* st) {
  CFTypeRef v = NULL;
  int result = 0;
  if (AXUIElementCopyAttributeValue(owner, attr, &v) == kAXErrorSuccess && v) {
    if (CFGetTypeID(v) == CFArrayGetTypeID()) {
      result = press_in_array((CFArrayRef)v, words, count, out, outlen, st);
    }
    CFRelease(v);
  }
  return result;
}

/// 扫每个窗口的 sheet（macOS 模态 sheet；WorkBuddy 的确认卡常是它而不是独立窗口）
static int press_sheets(AXUIElementRef app, const char* const* words, int count,
                        char* out, size_t outlen, WalkStats* st) {
  CFTypeRef wins = NULL;
  int result = 0;
  if (AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &wins) == kAXErrorSuccess && wins) {
    if (CFGetTypeID(wins) == CFArrayGetTypeID()) {
      CFArrayRef arr = (CFArrayRef)wins;
      CFIndex n = CFArrayGetCount(arr);
      for (CFIndex i = 0; i < n && !result; i++) {
        AXUIElementRef w = (AXUIElementRef)CFArrayGetValueAtIndex(arr, i);
        // kAXSheetAttribute 在部分 SDK 未导出，直接用字符串常量
        result = press_attr(w, CFSTR("AXSheetAttribute"), words, count, out, outlen, st);
      }
    }
    CFRelease(wins);
  }
  return result;
}

// 对 app 做一次完整查找：焦点窗口 → 全部窗口 → 各窗口 sheet → 主窗口 → children 兜底
static int press_once(AXUIElementRef app, const char* const* words, int count,
                      char* out, size_t outlen, WalkStats* st) {
  int result = 0;

  result = press_attr(app, kAXFocusedWindowAttribute, words, count, out, outlen, st);
  if (!result) result = press_attr(app, kAXWindowsAttribute, words, count, out, outlen, st);

  if (!result) {
    // 顺手统计窗口数（诊断用）
    CFTypeRef wins = NULL;
    if (AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &wins) == kAXErrorSuccess && wins) {
      if (CFGetTypeID(wins) == CFArrayGetTypeID() && st) {
        st->windows += (int)CFArrayGetCount((CFArrayRef)wins);
      }
      CFRelease(wins);
    }
    result = press_sheets(app, words, count, out, outlen, st);
  }
  if (!result) result = press_attr(app, kAXMainWindowAttribute, words, count, out, outlen, st);
  if (!result) result = press_attr(app, kAXChildrenAttribute, words, count, out, outlen, st);
  return result;
}

// ==================== 入口 ====================

int wc_ax_click_button(const char* const* words, int count, char* out, int outlen) {
  if (out && outlen > 0) out[0] = 0;

  // 权限前置：没有辅助功能授权时，后面每一步都会以 -25211 失败，
  // 提前返回独立错误码，让上层给出「去系统设置授权」而不是「找不到按钮」。
  if (!AXIsProcessTrusted()) {
    snprintf(out, outlen, "not-trusted");
    return -2;
  }

  // ---- 三级解析目标 App ----
  AXUIElementRef app = NULL;
  const char* src = "none";
  pid_t pid = -1;
  char appname[128] = {0};
  int err_focused_app = 0, err_focused_win = 0;

  AXUIElementRef sys = AXUIElementCreateSystemWide();
  if (!sys) {
    snprintf(out, outlen, "no-app; src=sysnull; trusted=1");
    return -1;
  }

  // ① system-wide 焦点 App（标准路径；Electron 目标上偶发直接失败）
  CFTypeRef focused = NULL;
  AXError e1 = AXUIElementCopyAttributeValue(sys, kAXFocusedApplicationAttribute, &focused);
  if (e1 == kAXErrorSuccess && focused) {
    app = (AXUIElementRef)focused;  // 所有权转移，下面统一 CFRelease(app)
    src = "focused-app";
  } else {
    err_focused_app = (int)e1;
  }

  // ② system-wide 焦点窗口 → pid → 建 app 元素（不依赖 focused-app 属性）
  if (!app) {
    CFTypeRef fw = NULL;
    AXError e2 = AXUIElementCopyAttributeValue(sys, kAXFocusedWindowAttribute, &fw);
    if (e2 == kAXErrorSuccess && fw) {
      pid_t p = -1;
      if (AXUIElementGetPid((AXUIElementRef)fw, &p) == kAXErrorSuccess && p > 0) {
        pid = p;
        app = AXUIElementCreateApplication(p);
        src = "focused-window";
      }
      CFRelease(fw);
    } else {
      err_focused_win = (int)e2;
    }
  }

  // ③ 可见窗口列表里最靠前的 owner（跳过自己与 Dock/菜单栏层），纯 C 不依赖 AppKit
  if (!app) {
    CFArrayRef wins = CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, kCGNullWindowID);
    if (wins) {
      CFIndex n = CFArrayGetCount(wins);
      pid_t self_pid = getpid();
      for (CFIndex i = 0; i < n && i < 60; i++) {
        CFDictionaryRef d = (CFDictionaryRef)CFArrayGetValueAtIndex(wins, i);
        CFNumberRef layer_n = (CFNumberRef)CFDictionaryGetValue(d, kCGWindowLayer);
        int layer = 0;
        if (layer_n) CFNumberGetValue(layer_n, kCFNumberIntType, &layer);
        if (layer < 0) continue;  // Dock / 菜单栏 / 桌面
        CFNumberRef pid_n = (CFNumberRef)CFDictionaryGetValue(d, kCGWindowOwnerPID);
        pid_t p = -1;
        if (pid_n) CFNumberGetValue(pid_n, kCFNumberIntType, &p);
        if (p <= 0 || p == self_pid) continue;
        app = AXUIElementCreateApplication(p);
        pid = p;
        src = "window-list";
        break;
      }
      CFRelease(wins);
    }
  }
  CFRelease(sys);

  if (!app) {
    snprintf(out, outlen,
             "no-app; trusted=1; errFocusedApp=%d; errFocusedWin=%d",
             err_focused_app, err_focused_win);
    return -1;
  }

  // app 可读标识：bundle id 优先，退回 title，再退回 pid
  copy_str(app, CFSTR("AXBundleIdentifier"), appname, sizeof(appname));
  if (!appname[0]) copy_str(app, kAXTitleAttribute, appname, sizeof(appname));
  if (pid < 0) AXUIElementGetPid(app, &pid);
  if (!appname[0]) snprintf(appname, sizeof(appname), "pid:%d", (int)pid);

  // ---- 打开 Electron / Chromium 的 Web 内容 AX 树 ----
  // 两个属性都写：有的 App 读 AXManualAccessibility，有的读 AXEnhancedUserInterface。
  // 幂等，已开启的 App 再设一次无害；一旦开启，Chromium 会保持到进程重启。
  // 返回值可能是 -25205（AttributeUnsupported，Electron bug #37465），记录下来供诊断。
  AXError e_manual = AXUIElementSetAttributeValue(app, CFSTR("AXManualAccessibility"), kCFBooleanTrue);
  AXError e_eui = AXUIElementSetAttributeValue(app, CFSTR("AXEnhancedUserInterface"), kCFBooleanTrue);

  // ---- 退避重试（Chromium 异步建树）----
  // 先立刻试一次（原生 App 树是现成的，快），失败则退避重试
  // （Electron 首建约几百毫秒）。最坏多花约 2.8s，只发生在审批动作上。
  WalkStats st;
  memset(&st, 0, sizeof(st));
  int result = press_once(app, words, count, out, (size_t)outlen, &st);
  const int delays_ms[] = {400, 800, 1600};
  for (int i = 0; i < 3 && !result; i++) {
    usleep((useconds_t)(delays_ms[i] * 1000));
    result = press_once(app, words, count, out, (size_t)outlen, &st);
  }

  if (!result) {
    // 角色清单：判断「树是空的」还是「文案不匹配」
    char roles[256] = {0};
    size_t rl = 0;
    for (int i = 0; i < st.role_count && rl + 1 < sizeof(roles); i++) {
      int w = snprintf(roles + rl, sizeof(roles) - rl, "%s%s", i ? "," : "", st.roles[i]);
      if (w > 0) rl += (size_t)w;
    }
    snprintf(out, outlen,
             "no-match; src=%s; app=%s; pid=%d; win=%d; els=%d; clickable=%d; "
             "manual=%d; eui=%d; roles=%s",
             src, appname, (int)pid, st.windows, st.elements, st.clickable,
             (int)e_manual, (int)e_eui, st.role_count ? roles : "(none)");
  }

  CFRelease(app);
  return result;
}
