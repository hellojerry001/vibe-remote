// 用 macOS Accessibility API（AXUIElement）直接在前台 App 的 AX 树里找按钮并 AXPress。
//
// 为什么不用 osascript / System Events：
//   那条路径要 TCC「自动化（Automation）」授权（System Events 去控制目标 App），
//   与「辅助功能（Accessibility）」是两套独立开关。很多机器只给了辅助功能、
//   没给自动化，osascript 就会 -10004、或卡在授权弹窗上被超时杀掉 → 点不到按钮，
//   表现就是「批准/拒绝没反应」。AX API 只受「辅助功能」管辖
//   （AXIsProcessTrusted 一套），不需要自动化；而键盘注入能用的机器辅助功能必定已授权。
//
// 返回值：1 = 点到；0 = 遍历过但没匹配到；-1 = 拿不到 AX（多半没有辅助功能授权）

#include <ApplicationServices/ApplicationServices.h>
#include <stdio.h>
#include <string.h>

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

int wc_ax_click_button(const char* const* words, int count, char* out, int outlen) {
  if (out && outlen > 0) out[0] = 0;
  AXUIElementRef sys = AXUIElementCreateSystemWide();
  if (!sys) return -1;
  CFTypeRef focused = NULL;
  AXError e = AXUIElementCopyAttributeValue(sys, kAXFocusedApplicationAttribute, &focused);
  CFRelease(sys);
  if (e != kAXErrorSuccess || !focused) return -1;

  AXUIElementRef app = (AXUIElementRef)focused;
  int result = 0;

  CFTypeRef windows = NULL;
  if (AXUIElementCopyAttributeValue(app, kAXWindowsAttribute, &windows) == kAXErrorSuccess && windows) {
    if (CFGetTypeID(windows) == CFArrayGetTypeID()) {
      result = press_in_array((CFArrayRef)windows, words, count, out, (size_t)outlen);
    }
    CFRelease(windows);
  }

  // 有些 App（或模态 sheet）不把弹窗挂在 kAXWindows 下，退回遍历 app 的 children
  if (!result) {
    CFTypeRef children = NULL;
    if (AXUIElementCopyAttributeValue(app, kAXChildrenAttribute, &children) == kAXErrorSuccess && children) {
      if (CFGetTypeID(children) == CFArrayGetTypeID()) {
        result = press_in_array((CFArrayRef)children, words, count, out, (size_t)outlen);
      }
      CFRelease(children);
    }
  }

  CFRelease(app);
  return result;
}
