#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/hid/IOHIDManager.h>
// IOHIDCheckAccess / IOHIDRequestAccess：真实的「输入监控」权限查询，
// 不能用 IOHIDManagerOpen() 的失败结果反推（那会把"没配对设备""打开失败"
// 一路都误报成未授权）。
#include <IOKit/hidsystem/IOHIDLib.h>
#include <pthread.h>
#include <stdatomic.h>

typedef void (*wc_event_callback)(const char *name, uint32_t usage_page, uint32_t usage, long value);
typedef void (*wc_connection_callback)(int connected_count);

static wc_event_callback g_event_cb = NULL;
static wc_connection_callback g_connection_cb = NULL;
static IOHIDManagerRef g_manager = NULL;
static CFRunLoopRef g_run_loop = NULL;
static atomic_int g_connected = 0;
static pthread_t g_thread;
static atomic_bool g_running = false;
/// IOHIDManagerOpen 是否成功；与权限状态是两回事，必须分开上报
static atomic_bool g_opened = false;
/// 实际匹配上的遥控器 Product ID（0 表示还没匹配到），用于状态页显示真实值
static atomic_int g_matched_pid = 0;

/// Apple TV / Siri Remote 的 PID 白名单。
/// 依据：本机 `com.apple.driver.AppleBluetoothRemote` 的
/// `ProductIDArray = (614, 621, 788, 789)`，即 0x0266 / 0x026D / 0x0314 / 0x0315。
///
/// ⚠️ 实测 A2854（USB-C 第三代）上报的是 **0x0314**。之前只按 0x0315 过滤，
/// 结果 IOHIDManager 一台都匹配不上 —— 表现就是「遥控器能控制 Mac（macOS 原生
/// AppleEmbeddedBluetoothButtons 接管了），但 App 收不到任何按键，映射全部失效」。
static const int SIRI_REMOTE_PIDS[] = {0x0266, 0x026D, 0x0314, 0x0315};
static const int SIRI_REMOTE_VIDS[] = {0x004C, 0x05AC};
#define SIRI_REMOTE_PID_COUNT ((int)(sizeof(SIRI_REMOTE_PIDS) / sizeof(SIRI_REMOTE_PIDS[0])))
#define SIRI_REMOTE_VID_COUNT ((int)(sizeof(SIRI_REMOTE_VIDS) / sizeof(SIRI_REMOTE_VIDS[0])))

/// 一只遥控器会暴露 3 个 IOHIDDevice：Consumer Control / Digitizer(触摸板) / Sensor。
/// 只有 Consumer Control 这一路才产按键，计数和取值都必须只认它，
/// 否则状态页会显示「已连接（3 台）」，而且触摸板数据会淹掉按键。
static int is_consumer_control(IOHIDDeviceRef device) {
  if (!device) return 0;
  CFTypeRef page = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDPrimaryUsagePageKey));
  CFTypeRef usage = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDPrimaryUsageKey));
  int p = 0, u = 0;
  if (page && CFGetTypeID(page) == CFNumberGetTypeID()) {
    CFNumberGetValue((CFNumberRef)page, kCFNumberIntType, &p);
  }
  if (usage && CFGetTypeID(usage) == CFNumberGetTypeID()) {
    CFNumberGetValue((CFNumberRef)usage, kCFNumberIntType, &u);
  }
  return (p == 0x0C && u == 0x01);
}

/// 触摸板（Digitizer）。与 Consumer Control 同一只遥控器，但报文是连续流，
/// 由 Rust 侧 touchpad.rs 独立组装成鼠标事件，不进按键管线。
///
/// ⚠️ 假设它的 PrimaryUsage 是 0x0D/0x01，但实测可能不符（Apple 各代不一）——
/// 所以 value_callback 不再按「白名单设备」过滤，改为「只排除传感器」，
/// 其余接口全收，交给 Rust 侧 raw 抓取通道用真实数据定字段。
static int is_sensor(IOHIDDeviceRef device) {
  if (!device) return 0;
  CFTypeRef page = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDPrimaryUsagePageKey));
  int p = 0;
  if (page && CFGetTypeID(page) == CFNumberGetTypeID()) {
    CFNumberGetValue((CFNumberRef)page, kCFNumberIntType, &p);
  }
  // Sensor 页：陀螺仪 / 加速度计，高频洪泛，必须排除
  return (p == 0x20);
}

int wc_hid_listen_event_access(void) {
  switch (IOHIDCheckAccess(kIOHIDRequestTypeListenEvent)) {
    case kIOHIDAccessTypeGranted: return 1;
    case kIOHIDAccessTypeDenied: return 2;
    default: return 0;  // unknown：还没请求过
  }
}

void wc_hid_request_listen_event_access(void) {
  IOHIDRequestAccess(kIOHIDRequestTypeListenEvent);
}

int wc_hid_manager_open(void) {
  return atomic_load(&g_opened) ? 1 : 0;
}

int wc_hid_callback_active(void) {
  return (atomic_load(&g_running) && atomic_load(&g_opened)) ? 1 : 0;
}

static const char *event_name(uint32_t page, uint32_t usage) {
  if (page == 0x0C) {
    switch (usage) {
      case 0x42: return "up";
      case 0x43: return "down";
      case 0x44: return "left";
      case 0x45: return "right";
      case 0x80: return "select";
      case 0xCD: return "playPause";
      case 0xE9: return "volumeUp";
      case 0xEA: return "volumeDown";
      case 0xE2: return "mute";
      case 0x30: return "power";
      case 0x04: return "siri";
      case 0x40: return "tv";
      default: break;
    }
  }
  if (page == 0x01 && usage == 0x86) return "back";
  // 触摸板（Digitizers 页 0x0D）：只挑手势判别需要的字段转发，
  // 其余（contact identifier / 压力等）丢弃，避免高频洪泛。
  // 命名约定 touch* —— Rust 侧 hid.rs 据此路由给 touchpad.rs。
  if (page == 0x0D) {
    switch (usage) {
      case 0x42: return "touchTip";     // tip switch（接触）
      case 0x43: return "touchInRange"; // in range（悬停）
      case 0x54: return "touchContact"; // contact count（手指数量）
      default: return NULL;
    }
  }
  // 0x0C/0x0060 与 0x0C/0x0004 是这颗遥控器独有的两个键位（TV / Siri），
  // 待按实测 usage 后再落到具体事件名，先保持 raw：raw 不会占用映射槽位，
  // 宁可不动，也好过猜错把「Siri」当成「返回」。
  return "raw";
}

static void value_callback(void *context, IOReturn result, void *sender, IOHIDValueRef value) {
  (void)context; (void)result; (void)sender;
  if (!g_event_cb || !value) return;
  IOHIDElementRef element = IOHIDValueGetElement(value);
  if (!element) return;
  // 只排除传感器（高频洪泛）；Consumer / Digitizer / 其他接口全收。
  // 字段对不对得上不靠猜：Rust 侧 raw 抓取通道全量记录 page/usage/value。
  IOHIDDeviceRef device = IOHIDElementGetDevice(element);
  if (device && is_sensor(device)) return;
  uint32_t page = IOHIDElementGetUsagePage(element);
  uint32_t usage = IOHIDElementGetUsage(element);
  // 页 0 / usage 0xFFFFFFFF 是 ReportID 0 的填充元素，不是按键
  if (page == 0 || usage == 0 || usage == 0xFFFFFFFF) return;
  const char *name = event_name(page, usage);
  // X/Y are Generic Desktop usages, only within a Digitizer collection.
  if (page == 0x01 && (usage == 0x30 || usage == 0x31) && !IOHIDElementIsRelative(element)) {
    for (IOHIDElementRef parent = IOHIDElementGetParent(element); parent;
         parent = IOHIDElementGetParent(parent)) {
      if (IOHIDElementGetUsagePage(parent) == 0x0D) {
        name = usage == 0x30 ? "touchX" : "touchY";
        break;
      }
    }
  }
  if (!name) return;  // 触摸板未识别字段：丢弃，不进管线
  long integer_value = IOHIDValueGetIntegerValue(value);
  g_event_cb(name, page, usage, integer_value);
}

static void device_added(void *context, IOReturn result, void *sender, IOHIDDeviceRef device) {
  (void)context; (void)result; (void)sender;
  if (!device) return;
  // 诊断：把每个匹配到的接口的主 usage 上报给 Rust 抓取通道，
  // 「系统到底暴露了哪几个接口、各是什么」一眼可见，不用猜。
  CFTypeRef dp = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDPrimaryUsagePageKey));
  CFTypeRef du = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDPrimaryUsageKey));
  int p = 0, u = 0;
  if (dp && CFGetTypeID(dp) == CFNumberGetTypeID()) CFNumberGetValue((CFNumberRef)dp, kCFNumberIntType, &p);
  if (du && CFGetTypeID(du) == CFNumberGetTypeID()) CFNumberGetValue((CFNumberRef)du, kCFNumberIntType, &u);
  if (g_event_cb && (p || u)) g_event_cb("deviceSeen", (uint32_t)p, (uint32_t)u, 1);
  if (!is_consumer_control(device)) return;
  CFTypeRef pid_ref = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDProductIDKey));
  if (pid_ref && CFGetTypeID(pid_ref) == CFNumberGetTypeID()) {
    int pid = 0;
    CFNumberGetValue((CFNumberRef)pid_ref, kCFNumberIntType, &pid);
    if (pid) atomic_store(&g_matched_pid, pid);
  }
  int count = atomic_fetch_add(&g_connected, 1) + 1;
  if (g_connection_cb) g_connection_cb(count);
}

static void device_removed(void *context, IOReturn result, void *sender, IOHIDDeviceRef device) {
  (void)context; (void)result; (void)sender;
  if (!is_consumer_control(device)) return;
  int old = atomic_load(&g_connected);
  int count = old > 0 ? atomic_fetch_sub(&g_connected, 1) - 1 : 0;
  if (count < 0) { atomic_store(&g_connected, 0); count = 0; }
  if (count == 0) atomic_store(&g_matched_pid, 0);
  if (g_connection_cb) g_connection_cb(count);
}

static CFMutableDictionaryRef number_match(int vendor, int product) {
  CFMutableDictionaryRef dict = CFDictionaryCreateMutable(kCFAllocatorDefault, 0,
    &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
  CFNumberRef v = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &vendor);
  CFNumberRef p = CFNumberCreate(kCFAllocatorDefault, kCFNumberIntType, &product);
  CFDictionarySetValue(dict, CFSTR(kIOHIDVendorIDKey), v);
  CFDictionarySetValue(dict, CFSTR(kIOHIDProductIDKey), p);
  CFRelease(v); CFRelease(p);
  return dict;
}

static void hid_teardown(void) {
  if (g_manager) {
    if (g_run_loop) {
      IOHIDManagerUnscheduleFromRunLoop(g_manager, g_run_loop, kCFRunLoopDefaultMode);
    }
    IOHIDManagerClose(g_manager, kIOHIDOptionsTypeNone);
    CFRelease(g_manager);
    g_manager = NULL;
  }
  if (g_run_loop) {
    CFRelease(g_run_loop);
    g_run_loop = NULL;
  }
  atomic_store(&g_connected, 0);
  atomic_store(&g_matched_pid, 0);
  atomic_store(&g_opened, false);
}

static void *monitor_thread(void *unused) {
  (void)unused;
  g_manager = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
  if (!g_manager) { atomic_store(&g_running, false); return NULL; }

  // VID × PID 全组合，别只写一个 PID —— 不同代遥控器的 PID 不一样
  CFMutableDictionaryRef dicts[SIRI_REMOTE_VID_COUNT * SIRI_REMOTE_PID_COUNT];
  int n = 0;
  for (int v = 0; v < SIRI_REMOTE_VID_COUNT; v++) {
    for (int p = 0; p < SIRI_REMOTE_PID_COUNT; p++) {
      dicts[n++] = number_match(SIRI_REMOTE_VIDS[v], SIRI_REMOTE_PIDS[p]);
    }
  }
  CFArrayRef matches = CFArrayCreate(kCFAllocatorDefault, (const void **)dicts, n,
    &kCFTypeArrayCallBacks);
  IOHIDManagerSetDeviceMatchingMultiple(g_manager, matches);
  CFRelease(matches);
  for (int i = 0; i < n; i++) CFRelease(dicts[i]);

  IOHIDManagerRegisterDeviceMatchingCallback(g_manager, device_added, NULL);
  IOHIDManagerRegisterDeviceRemovalCallback(g_manager, device_removed, NULL);
  IOHIDManagerRegisterInputValueCallback(g_manager, value_callback, NULL);

  g_run_loop = CFRunLoopGetCurrent();
  CFRetain(g_run_loop);
  IOHIDManagerScheduleWithRunLoop(g_manager, g_run_loop, kCFRunLoopDefaultMode);
  IOReturn open_result = IOHIDManagerOpen(g_manager, kIOHIDOptionsTypeNone);
  if (open_result != kIOReturnSuccess) {
    // 打开失败不等于未授权：可能是没配对 A2854、也可能是权限被拒。
    // 真实权限状态由 wc_hid_listen_event_access() 单独上报。
    if (g_event_cb) {
      g_event_cb("hidOpenError", 0, 0, (long)open_result);
    }
    hid_teardown();
    atomic_store(&g_running, false);
    return NULL;
  }
  atomic_store(&g_opened, true);
  CFRunLoopRun();
  hid_teardown();
  atomic_store(&g_running, false);
  return NULL;
}

int wc_siri_remote_start(wc_event_callback event_cb, wc_connection_callback connection_cb) {
  if (atomic_exchange(&g_running, true)) return 0;
  g_event_cb = event_cb;
  g_connection_cb = connection_cb;
  int rc = pthread_create(&g_thread, NULL, monitor_thread, NULL);
  if (rc != 0) { atomic_store(&g_running, false); return rc; }
  return 0;
}

void wc_siri_remote_stop(void) {
  if (!atomic_load(&g_running)) return;
  if (g_run_loop) CFRunLoopStop(g_run_loop);
  pthread_join(g_thread, NULL);
}

void wc_siri_remote_restart(void) {
  wc_siri_remote_stop();
  wc_siri_remote_start(g_event_cb, g_connection_cb);
}

int wc_siri_remote_connected_count(void) {
  return atomic_load(&g_connected);
}

/// 实际匹配到的遥控器 PID（0 = 还没匹配到）。
/// 状态页据此显示真实值，而不是写死的期望值 —— 别的代次的遥控器也能一眼看出。
int wc_siri_remote_matched_pid(void) {
  return atomic_load(&g_matched_pid);
}

/// 匹配用的 VID / PID 白名单，供诊断面板回显「我在等什么」
int wc_siri_remote_pid_count(void) { return SIRI_REMOTE_PID_COUNT; }
int wc_siri_remote_pid_at(int index) {
  if (index < 0 || index >= SIRI_REMOTE_PID_COUNT) return -1;
  return SIRI_REMOTE_PIDS[index];
}
