//! 键盘模拟：直接 FFI CoreGraphics 的 CGEvent（与旧 Swift 版 ActionExecutor 行为一致），
//! 避免 CGEvent 相关 crate 的 API 版本不确定性。

use std::os::raw::{c_uint, c_void};
use std::time::Duration;

const HID_TAP: c_uint = 0;

pub const FLAG_COMMAND: u64 = 1 << 20;
pub const FLAG_SHIFT: u64 = 1 << 17;
pub const FLAG_ALTERNATE: u64 = 1 << 19;
pub const FLAG_CONTROL: u64 = 1 << 18;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(source: *mut c_void, virtual_key: u16, key_down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPost(tap: c_uint, event: *mut c_void);
    fn CFRelease(cf: *mut c_void);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u32;
}

pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

pub fn press_key(code: u16, flags: u64) {
    unsafe {
        let down = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, true);
        if down.is_null() {
            return;
        }
        let up = CGEventCreateKeyboardEvent(std::ptr::null_mut(), code, false);
        if flags != 0 {
            CGEventSetFlags(down, flags);
            if !up.is_null() {
                CGEventSetFlags(up, flags);
            }
        }
        CGEventPost(HID_TAP, down);
        // down/up 背靠背 posting 会被部分 App（尤其 Electron）合并或丢弃，
        // 中间垫 10ms 显著提升触达率，对体感延迟几乎无影响
        std::thread::sleep(Duration::from_millis(10));
        if !up.is_null() {
            CGEventPost(HID_TAP, up);
        }
        CFRelease(down);
        if !up.is_null() {
            CFRelease(up);
        }
    }
}

/// 双击 Control 触发 macOS 听写（需系统设置里听写快捷键为「连按两次 Control 键」）
pub fn double_tap_control() {
    for _ in 0..2 {
        press_key(59, FLAG_CONTROL);
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// macOS 虚拟键码（与旧 Swift 版保持一致的映射表）
pub fn key_code(name: &str) -> Option<u16> {
    match name.trim().to_lowercase().as_str() {
        "return" | "enter" => Some(36),
        "tab" => Some(48),
        "space" => Some(49),
        "escape" | "esc" => Some(53),
        "delete" | "backspace" => Some(51),
        "forwarddelete" => Some(117),
        "left" => Some(123),
        "right" => Some(124),
        "down" => Some(125),
        "up" => Some(126),
        "home" => Some(115),
        "end" => Some(119),
        "pageup" => Some(116),
        "pagedown" => Some(121),
        "ctrl" | "control" => Some(59),
        "cmd" | "command" => Some(55),
        "shift" => Some(56),
        "opt" | "option" | "alt" => Some(58),
        "a" => Some(0),
        "s" => Some(1),
        "d" => Some(2),
        "f" => Some(3),
        "h" => Some(4),
        "g" => Some(5),
        "z" => Some(6),
        "x" => Some(7),
        "c" => Some(8),
        "v" => Some(9),
        "b" => Some(11),
        "q" => Some(12),
        "w" => Some(13),
        "e" => Some(14),
        "r" => Some(15),
        "y" => Some(16),
        "t" => Some(17),
        "1" => Some(18),
        "2" => Some(19),
        "3" => Some(20),
        "4" => Some(21),
        "6" => Some(22),
        "5" => Some(23),
        "=" => Some(24),
        "9" => Some(25),
        "7" => Some(26),
        "-" => Some(27),
        "8" => Some(28),
        "0" => Some(29),
        "]" => Some(30),
        "o" => Some(31),
        "u" => Some(32),
        "[" => Some(33),
        "i" => Some(34),
        "p" => Some(35),
        "l" => Some(37),
        "j" => Some(38),
        "'" => Some(39),
        "k" => Some(40),
        ";" => Some(41),
        "\\" => Some(42),
        "," => Some(43),
        "/" => Some(44),
        "n" => Some(45),
        "m" => Some(46),
        "." => Some(47),
        _ => None,
    }
}

/// 解析 "Cmd+Shift+P" 形式的快捷键，返回（键码，修饰键 flags）
pub fn parse_shortcut(spec: &str) -> Option<(u16, u64)> {
    let binding = spec.to_lowercase();
    let parts: Vec<&str> = binding.split('+').map(str::trim).collect();
    let key = parts.last()?;
    let code = key_code(key)?;
    let mut flags = 0u64;
    for token in &parts[..parts.len() - 1] {
        flags |= match *token {
            "cmd" | "command" | "⌘" => FLAG_COMMAND,
            "shift" | "⇧" => FLAG_SHIFT,
            "opt" | "option" | "alt" | "⌥" => FLAG_ALTERNATE,
            "ctrl" | "control" | "⌃" => FLAG_CONTROL,
            _ => 0,
        };
    }
    Some((code, flags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shortcut() {
        assert_eq!(parse_shortcut("Cmd+Enter"), Some((36, FLAG_COMMAND)));
        assert_eq!(
            parse_shortcut("ctrl+shift+p"),
            Some((35, FLAG_CONTROL | FLAG_SHIFT))
        );
        assert_eq!(parse_shortcut("escape"), Some((53, 0)));
        assert_eq!(parse_shortcut("nope+"), None);
    }

    #[test]
    fn resolves_key_names() {
        assert_eq!(key_code("return"), Some(36));
        assert_eq!(key_code("UP"), Some(126));
        assert_eq!(key_code("x"), Some(7));
        assert_eq!(key_code("f13"), None);
    }
}
