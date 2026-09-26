//! 鼠标模拟：直接 FFI CoreGraphics（与 keyboard.rs 同一套 CGEvent 思路）。
//! 供触摸板→光标管线使用；需要辅助功能权限（与键盘模拟同体系）。

use std::os::raw::{c_int, c_uint, c_void};

const HID_TAP: c_uint = 0;
const kCGEventLeftMouseDown: c_uint = 1;
const kCGEventLeftMouseUp: c_uint = 2;
const kCGEventMouseMoved: c_uint = 5;
/// CGEventCreateScrollWheelEvent 的单位：像素（1 = line）
const kCGScrollEventUnitPixel: c_uint = 0;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    /// 事件坐标（对刚 create 的 event 而言就是当前光标位置）
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        event_type: c_uint,
        position: CGPoint,
        button: c_uint,
    ) -> *mut c_void;
    /// C 侧是 variadic；arm64 上按固定 arity 声明即可覆盖 wheelCount=1
    fn CGEventCreateScrollWheelEvent(
        source: *mut c_void,
        units: c_uint,
        wheel_count: c_uint,
        wheel1: c_int,
    ) -> *mut c_void;
    fn CGEventPost(tap: c_uint, event: *mut c_void);
    fn CFRelease(cf: *mut c_void);
}

fn post(ev: *mut c_void) {
    unsafe {
        if !ev.is_null() {
            CGEventPost(HID_TAP, ev);
            CFRelease(ev);
        }
    }
}

pub fn cursor_location() -> (f64, f64) {
    unsafe {
        let ev = CGEventCreate(std::ptr::null_mut());
        if ev.is_null() {
            return (0.0, 0.0);
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev);
        (p.x, p.y)
    }
}

/// 按增量移动光标
pub fn move_by(dx: f64, dy: f64) {
    let (x, y) = cursor_location();
    let target = CGPoint { x: x + dx, y: y + dy };
    post(unsafe { CGEventCreateMouseEvent(std::ptr::null_mut(), kCGEventMouseMoved, target, 0) });
}

pub fn left_click() {
    let (x, y) = cursor_location();
    let at = CGPoint { x, y };
    post(unsafe {
        CGEventCreateMouseEvent(std::ptr::null_mut(), kCGEventLeftMouseDown, at, 0)
    });
    post(unsafe { CGEventCreateMouseEvent(std::ptr::null_mut(), kCGEventLeftMouseUp, at, 0) });
}

/// 像素级滚动（正 = 向上）。触摸板双指滚动用；v1 暂未启用，保留给报文分析后接上。
#[allow(dead_code)]
pub fn scroll_pixels(delta_y: f64) {
    post(unsafe {
        CGEventCreateScrollWheelEvent(
            std::ptr::null_mut(),
            kCGScrollEventUnitPixel,
            1,
            delta_y as c_int,
        )
    });
}
