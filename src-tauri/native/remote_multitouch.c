// Siri Remote touch frames use macOS MultitouchSupport, not HID value callbacks.
// Private ABI reference: https://github.com/lauschue/Remotastic/blob/main/MultitouchSupport.h
// Dynamically loaded so missing symbols disable touch without breaking buttons.
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/IOKitLib.h>
#include <dlfcn.h>
#include <math.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stddef.h>
#include <unistd.h>

typedef struct { float x, y; } WCPoint;
typedef struct { WCPoint position, velocity; } Vector;
typedef struct {
  int32_t frame;
  double timestamp;
  int32_t path, state, finger, hand;
  Vector normalized;
  float quality;
  int32_t field9;
  float angle, major, minor;
  Vector absolute;
  int32_t field14, field15;
  float density;
} Contact;
_Static_assert(sizeof(Contact) == 96, "Unexpected MultitouchSupport contact ABI");
_Static_assert(offsetof(Contact, normalized) == 32, "Unexpected contact layout");
typedef void (*FrameCallback)(CFTypeRef, Contact *, size_t, double, size_t);
typedef void (*OutputCallback)(uint64_t, int, double, double);
static CFArrayRef (*create_list)(void);
static int (*get_id)(CFTypeRef, uint64_t *);
static int (*device_start)(CFTypeRef, int);
static int (*device_stop)(CFTypeRef);
static void (*register_frame)(CFTypeRef, FrameCallback);
static void (*unregister_frame)(CFTypeRef, FrameCallback);
static OutputCallback output;
static atomic_int device_count, error_code;
static atomic_ullong frames;
static atomic_bool started;

static int64_t number(io_registry_entry_t entry, CFStringRef key) {
  CFTypeRef value = IORegistryEntryCreateCFProperty(entry, key, kCFAllocatorDefault, 0);
  int64_t result = -1;
  if (value) {
    if (CFGetTypeID(value) == CFNumberGetTypeID())
      CFNumberGetValue(value, kCFNumberSInt64Type, &result);
    CFRelease(value);
  }
  return result;
}

// Match the actual registry identity, never just dimensions or "external" status.
static int remote_ids(uint64_t *ids, int cap) {
  io_iterator_t iterator = 0;
  if (IOServiceGetMatchingServices(MACH_PORT_NULL,
      IOServiceMatching("AppleMultitouchDevice"), &iterator) != KERN_SUCCESS) return 0;
  int count = 0;
  io_registry_entry_t entry;
  while ((entry = IOIteratorNext(iterator))) {
    int64_t pid = number(entry, CFSTR("ProductID"));
    int64_t family = number(entry, CFSTR("Family ID"));
    int64_t id = number(entry, CFSTR("Multitouch ID"));
    if (family == 145 && id >= 0 && count < cap &&
        (pid == 0x0266 || pid == 0x026D || pid == 0x0314 || pid == 0x0315))
      ids[count++] = (uint64_t)id;
    IOObjectRelease(entry);
  }
  IOObjectRelease(iterator);
  return count;
}

static void on_frame(CFTypeRef device, Contact *touches, size_t count, double time, size_t frame) {
  (void)time; (void)frame;
  if (count > 16 || (count && !touches)) return;
  uint64_t id = 0;
  if (get_id(device, &id) != 0) return;
  int active = 0;
  double x = 0, y = 0;
  for (size_t i = 0; i < count; i++) {
    if (touches[i].state != 3 && touches[i].state != 4) continue;
    double px = touches[i].normalized.position.x, py = touches[i].normalized.position.y;
    if (!isfinite(px) || !isfinite(py)) return;
    x = px; y = py; active++;
  }
  atomic_fetch_add(&frames, 1);
  output(id, active, x, y);
}

static void *monitor(void *unused) {
  (void)unused;
  void *library = dlopen("/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport", RTLD_NOW | RTLD_LOCAL);
  if (!library) { atomic_store(&error_code, -1); return NULL; }
#define LOAD(variable, symbol) do { *(void **)(&variable) = dlsym(library, symbol); if (!variable) { atomic_store(&error_code, -2); return NULL; } } while (0)
  LOAD(create_list, "MTDeviceCreateList");
  LOAD(get_id, "MTDeviceGetDeviceID");
  LOAD(device_start, "MTDeviceStart");
  LOAD(device_stop, "MTDeviceStop");
  LOAD(register_frame, "MTRegisterContactFrameCallback");
  LOAD(unregister_frame, "MTUnregisterContactFrameCallback");
#undef LOAD
  CFTypeRef devices[8] = {0};
  uint64_t attached_ids[8] = {0};
  for (;;) {
    uint64_t ids[8];
    int count = remote_ids(ids, 8);
    for (int i = 0; i < 8; i++) {
      if (!devices[i]) continue;
      int found = 0;
      for (int j = 0; j < count; j++) if (ids[j] == attached_ids[i]) found = 1;
      if (!found) {
        device_stop(devices[i]);
        unregister_frame(devices[i], on_frame);
        output(attached_ids[i], -1, 0, 0);
        CFRelease(devices[i]);
        devices[i] = NULL;
      }
    }
    CFArrayRef list = create_list();
    if (list) {
      for (CFIndex i = 0; i < CFArrayGetCount(list); i++) {
        CFTypeRef device = CFArrayGetValueAtIndex(list, i);
        uint64_t id = 0;
        if (get_id(device, &id) != 0) continue;
        int remote = 0, exists = 0, slot = -1;
        for (int j = 0; j < count; j++) if (id == ids[j]) remote = 1;
        for (int j = 0; j < 8; j++) {
          if (devices[j] && attached_ids[j] == id) exists = 1;
          if (!devices[j]) slot = j;
        }
        if (!remote || exists || slot < 0) continue;
        register_frame(device, on_frame);
        int rc = device_start(device, 0);
        if (rc != 0) { unregister_frame(device, on_frame); atomic_store(&error_code, rc); continue; }
        devices[slot] = CFRetain(device);
        attached_ids[slot] = id;
        atomic_store(&error_code, 0);
      }
      CFRelease(list);
    }
    int attached = 0;
    for (int i = 0; i < 8; i++) if (devices[i]) attached++;
    atomic_store(&device_count, attached);
    sleep(2);
  }
  return NULL;
}

void wc_multitouch_start(OutputCallback callback) {
  if (atomic_exchange(&started, true)) return;
  output = callback;
  pthread_t thread;
  int rc = pthread_create(&thread, NULL, monitor, NULL);
  if (rc != 0) { atomic_store(&error_code, rc); return; }
  pthread_detach(thread);
}
int wc_multitouch_devices(void) { return atomic_load(&device_count); }
int wc_multitouch_error(void) { return atomic_load(&error_code); }
uint64_t wc_multitouch_frames(void) { return atomic_load(&frames); }
