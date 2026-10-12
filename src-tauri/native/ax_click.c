// 用 macOS Accessibility API（AXUIElement）在前台 App 的 AX 树里找按钮并 AXPress。
//
// 为什么不用 osascript / System Events：
//   那条路要 TCC「自动化（Automation）」授权（System Events 去控制目标 App），
//   与「辅助功能」是两套独立开关；而且它同样只能看到 App 暴露出来的 AX 树。
//   AX API 只受「辅助功能」管辖（AXIsProcessTrusted），键盘注入能用的机器必定已授权。
//
// ⚠️ 关键前提 —— 此前两版都失败在这里（2026-10-12 调研坐实）：
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
// 返回值：1 = 点到；0 = 遍历过但没匹配到（已含启用 AX 树 + 重试）；-1 = 拿不到 AX（多半没辅助功能授权）

#include <ApplicationServices/ApplicationServices.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

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

// 递归查找并 AXPress。exact=1 只做精确匹配，exact=0 做子串匹配。
static int search_press(AXUIElementRef el, const char* const* words, int count,
                        char* out, size_t outlen, int depth, int exact) {
  if (!el || depth > 60) return 0;

  char role[64] = {0}, title[256] = {0}, value[256] = {0}, desc[256] = {0};
  int has_role = copy_str(el, kAXRoleAttribute, role, sizeof(role));
  copy_str(el, kAXTitleAttribute, title, sizeof(title));
  copy_str(el, kAXValueAttribute, value, sizeof(value));
  copy_str(el, kAXDescriptionAttribute, desc, sizeof(desc));

  if (has_role && is_clickable_role(role)) {
    const char* cands[3] = {title, value, desc};
    for (int c = 0; c < 3; c++) {
      if (!cands[c][0]) continue;
      int hit = exact ? equals_any(cands[c], words, count)
                      : contains_any(cands[c], words, count);
      // 只有 AXPress 真的成功才算命中；非可点元素会返回错误，自动跳过继续找。
      if (hit && AXUIElementPerformAction(el, kAXPressAction) == kAXErrorSuccess) {
        snprintf(out, outlen, "%s", cands[c]);
        return 1;
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
        if (search_press(child, words, count, out, outlen, depth + 1, exact)) { found = 1; break; }
      }
    }
    CFRelease(children);
  }
  return found;
}

// 在一组元素上做「先精确、再子串」两遍匹配
static int press_in_array(CFArrayRef arr, const char* const* words, int count,
                          char* out, size_t outlen) {
  CFIndex n = CFArrayGetCount(arr);
  for (int pass = 0; pass < 2; pass++) {
    int exact = (pass == 0);
    for (CFIndex i = 0; i < n; i++) {
      AXUIElementRef el = (AXUIElementRef)CFArrayGetValueAtIndex(arr, i);
      if (search_press(el, words, count, out, outlen, 0, exact)) return 1;
    }
  }
  return 0;
}

// 对 app 做一次完整查找：先 kAXWindows，再退回 kAXChildren（模态 sheet 用）
static int press_once(AXUIElementRef app, const char* const* words, int count,
                      char* out, size_t outlen) {
  CFTypeRef windows = NULL;
  int result = 0;
  if (AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &windows) == kAXErrorSuccess && windows) {
    if (CFGetTypeID(windows) == CFArrayGetTypeID()) {
      result = press_in_array((CFArrayRef)windows, words, count, out, outlen);
    }
    CFRelease(windows);
  }
  if (!result) {
    CFTypeRef children = NULL;
    if (AXUIElementCopyAttributeValue(app, kAXChildrenAttribute, &children) == kAXErrorSuccess && children) {
      if (CFGetTypeID(children) == CFArrayGetTypeID()) {
        result = press_in_array((CFArrayRef)children, words, count, out, outlen);
      }
      CFRelease(children);
    }
  }
  return result;
}

int wc_ax_click_button(const char* const* words, int count, char* out, int outlen) {
  if (out && outlen > 0) out[0] = 0;
  AXUIElementRef sys = AXUIElementCreateSystemWide();
  if (!sys) return -1;
  CFTypeRef focused = NULL;
  AXError e = AXUIElementCopyAttributeValue(sys, kAXFocusedApplicationAttribute, &focused);
  CFRelease(sys);
  if (e != kAXErrorSuccess || !focused) return -1;

  AXUIElementRef app = (AXUIElementRef)focused;

  // 让 Electron / Chromium 把 Web 内容暴露到 AX 树 —— 这是能点到 DOM 按钮的前提。
  // 两个属性都写：有的 App 读 AXManualAccessibility，有的读 AXEnhancedUserInterface。
  // 幂等，已开启的 App 再设一次无害；一旦开启，Chromium 会保持到进程重启。
  AXUIElementSetAttributeValue(app, CFSTR("AXManualAccessibility"), kCFBooleanTrue);
  AXUIElementSetAttributeValue(app, CFSTR("AXEnhancedUserInterface"), kCFBooleanTrue);

  // Chromium 是异步建树的：先立刻试一次（原生 App 树是现成的，快），
  // 失败则退避重试（Electron 首建约几百毫秒）。最坏多花约 1.3s，只发生在审批动作上。
  int result = press_once(app, words, count, out, (size_t)outlen);
  const int delays_ms[] = {300, 500, 500};
  for (int i = 0; i < 3 && !result; i++) {
    usleep((useconds_t)(delays_ms[i] * 1000));
    result = press_once(app, words, count, out, (size_t)outlen);
  }

  CFRelease(app);
  return result;
}
