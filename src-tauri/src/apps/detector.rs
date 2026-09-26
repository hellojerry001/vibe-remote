//! 前台 App 检测（用于状态页展示当前受控目标 + 按键派发前的焦点判断）。
//!
//! 全部走进程内 FFI（objc_msgSend → AppKit），零子进程、零 Apple Events：
//! - 子进程 osascript 一次 ~100ms，focus 轮询里翻倍成秒级延迟；
//! - Apple Events（`tell application ... to activate`）依赖 TCC「自动化」授权，
//!   重构建后授权失效时会被系统阻塞数秒 —— 热路径必须避开。
//!
//! ⚠️ objc_getClass / sel_registerName 需要 **\0 结尾** 的 C 字符串；
//! &str 直接 as_ptr 是读越界。统一走 `get_class` / `sel` 帮助函数（内部 CString）。

#[cfg(target_os = "macos")]
mod objc {
    use std::ffi::CString;
    use std::os::raw::{c_char, c_void};

    pub type Id = *mut c_void;
    pub type Sel = *const c_char;

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        pub fn objc_getClass(name: *const c_char) -> Id;
        pub fn sel_registerName(name: *const c_char) -> Sel;
        /// aarch64 上指针返回的无参方法
        #[link_name = "objc_msgSend"]
        pub fn msg_send0(receiver: Id, sel: Sel) -> Id;
        /// NSUInteger 返回（NSArray count）
        #[link_name = "objc_msgSend"]
        pub fn msg_send0_usize(receiver: Id, sel: Sel) -> usize;
        /// pid_t / int 返回（NSRunningApplication.processIdentifier）
        #[link_name = "objc_msgSend"]
        pub fn msg_send0_i32(receiver: Id, sel: Sel) -> i32;
        /// NSUInteger 参数、BOOL 返回（activateWithOptions:）
        #[link_name = "objc_msgSend"]
        pub fn msg_send1_usize_bool(receiver: Id, sel: Sel, arg: usize) -> u8;
        /// 指针参数、指针返回（objectAtIndexedSubscript:）
        #[link_name = "objc_msgSend"]
        pub fn msg_send1(receiver: Id, sel: Sel, arg: Id) -> Id;
        /// pid_t 参数、指针返回（runningApplicationWithProcessIdentifier:）
        #[link_name = "objc_msgSend"]
        pub fn msg_send1_i32(receiver: Id, sel: Sel, arg: i32) -> Id;
    }

    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    /// 类名 → Class。CString 保证 \0 结尾。
    pub unsafe fn get_class(name: &str) -> Id {
        let c = CString::new(name).expect("类名不含 \\0");
        objc_getClass(c.as_ptr())
    }

    /// selector 名 → Sel。CString 保证 \0 结尾。
    pub unsafe fn sel(name: &str) -> Sel {
        let c = CString::new(name).expect("selector 不含 \\0");
        sel_registerName(c.as_ptr())
    }

    /// NSString → Rust String；nil / 空指针返回 None
    unsafe fn utf8(obj: Id) -> Option<String> {
        if obj.is_null() {
            return None;
        }
        let s = msg_send0(obj, sel("UTF8String")) as *const c_char;
        if s.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr(s).to_string_lossy().into_owned())
    }

    pub struct FrontmostRaw {
        pub name: String,
        pub bundle_id: Option<String>,
    }

    /// NSWorkspace → sharedWorkspace → frontmostApplication → localizedName + bundleIdentifier。
    /// 任何一环拿不到都返回 None，调用方走慢速兜底。
    pub unsafe fn frontmost_via_objc() -> Option<FrontmostRaw> {
        let ws_cls = get_class("NSWorkspace");
        if ws_cls.is_null() {
            return None;
        }
        let ws = msg_send0(ws_cls, sel("sharedWorkspace"));
        if ws.is_null() {
            return None;
        }
        let app = msg_send0(ws, sel("frontmostApplication"));
        if app.is_null() {
            return None;
        }
        let name = utf8(msg_send0(app, sel("localizedName")))?;
        let bundle_id = utf8(msg_send0(app, sel("bundleIdentifier")));
        Some(FrontmostRaw { name, bundle_id })
    }

    pub struct RunningRaw {
        pub pid: i32,
        pub name: String,
        pub bundle_id: Option<String>,
    }

    /// 遍历 NSWorkspace.runningApplications，找 bundle id 或名字匹配的运行中 App。
    /// target 含 `.` 视为 bundle id 精确匹配；否则 bundle id / localizedName 任一命中即可。
    pub unsafe fn resolve_running_via_objc(target: &str) -> Option<RunningRaw> {
        let ws_cls = get_class("NSWorkspace");
        if ws_cls.is_null() {
            return None;
        }
        let ws = msg_send0(ws_cls, sel("sharedWorkspace"));
        if ws.is_null() {
            return None;
        }
        let apps = msg_send0(ws, sel("runningApplications"));
        if apps.is_null() {
            return None;
        }
        let count = msg_send0_usize(apps, sel("count"));
        for i in 0..count {
            let app = msg_send1(apps, sel("objectAtIndexedSubscript:"), i as Id);
            if app.is_null() {
                continue;
            }
            let bundle_id = utf8(msg_send0(app, sel("bundleIdentifier")));
            let name = utf8(msg_send0(app, sel("localizedName")));
            let bundle_matched = bundle_id
                .as_deref()
                .map(|b| b.eq_ignore_ascii_case(target))
                .unwrap_or(false);
            let name_matched = name
                .as_deref()
                .map(|n| n.eq_ignore_ascii_case(target))
                .unwrap_or(false);
            let matched = if target.contains('.') {
                bundle_matched
            } else {
                bundle_matched || name_matched
            };
            if matched {
                let pid = msg_send0_i32(app, sel("processIdentifier"));
                if pid > 0 {
                    return Some(RunningRaw {
                        pid,
                        name: name.unwrap_or_default(),
                        bundle_id,
                    });
                }
            }
        }
        None
    }

    /// NSRunningApplication.activateWithOptions（不走 Apple Events，不需要 TCC 自动化授权）。
    /// macOS 14 起软弃用但仍可用；失败返回 false，调用方走 osascript 兜底。
    pub unsafe fn activate_pid_via_objc(pid: i32) -> bool {
        let cls = get_class("NSRunningApplication");
        if cls.is_null() {
            return false;
        }
        let app = msg_send1_i32(cls, sel("runningApplicationWithProcessIdentifier:"), pid);
        if app.is_null() {
            return false;
        }
        // NSApplicationActivateAllWindows = 1 << 0
        msg_send1_usize_bool(app, sel("activateWithOptions:"), 1) != 0
    }
}

/// 自动管理 autorelease pool 地跑一段 FFI 逻辑。
/// localizedName 等可能返回 autoreleased 对象，HID 事件线程没有 runloop，
/// 不 drain 会慢慢泄漏。pool 类都拿不到时返回 None（FFI 环境异常）。
#[cfg(target_os = "macos")]
fn with_autorelease_pool<T>(f: impl FnOnce() -> T) -> Option<T> {
    let pool_cls = unsafe { objc::get_class("NSAutoreleasePool") };
    if pool_cls.is_null() {
        return None;
    }
    unsafe {
        let pool: objc::Id = objc::msg_send0(pool_cls, objc::sel("new"));
        let result = f();
        let _: objc::Id = objc::msg_send0(pool, objc::sel("drain"));
        Some(result)
    }
}

/// 前台 App 信息：localizedName + bundleIdentifier。
#[derive(Debug, Clone)]
pub struct FrontmostInfo {
    pub name: String,
    pub bundle_id: Option<String>,
}

/// 进程内快速查询当前前台 App（微秒级）。
///
/// 之前每次按键都走 osascript 子进程查前台（~100ms/次，切焦点最坏 ~900ms），
/// 是「遥控器延迟严重」的主根因。返回 `None` 表示 FFI 不可用，调用方应兜底。
#[cfg(target_os = "macos")]
pub fn frontmost_fast() -> Option<FrontmostInfo> {
    with_autorelease_pool(|| unsafe {
        objc::frontmost_via_objc().map(|raw| FrontmostInfo {
            name: raw.name,
            bundle_id: raw.bundle_id,
        })
    })
    .flatten()
}

/// 目标 App 在运行时的解析结果。
#[derive(Debug, Clone)]
pub struct RunningTarget {
    pub pid: i32,
    pub bundle_id: String,
}

/// 按名字或 bundle id 找运行中的目标 App。名字匹配对「可执行名 ≠ 显示名」的
/// App（如 WorkBuddy 的可执行名是 Electron）用 localizedName，与系统一致。
#[cfg(target_os = "macos")]
pub fn resolve_running_target(target: &str) -> Option<RunningTarget> {
    with_autorelease_pool(|| unsafe {
        objc::resolve_running_via_objc(target).map(|raw| RunningTarget {
            pid: raw.pid,
            bundle_id: raw.bundle_id.unwrap_or_else(|| raw.name.clone()),
        })
    })
    .flatten()
}

/// FFI 激活运行中的 App。返回 false 表示失败（调用方走 osascript 兜底）。
#[cfg(target_os = "macos")]
pub fn activate_pid(pid: i32) -> bool {
    with_autorelease_pool(|| unsafe { objc::activate_pid_via_objc(pid) }).unwrap_or(false)
}

/// 前台判定（按 bundle id，不受可执行名影响）。
///
/// WorkBuddy 的 CFBundleExecutable 是 "Electron"，System Events 报的进程名
/// 也是 "Electron" —— 按显示名 "WorkBuddy" 匹配前台永远失败，会退化成
/// 每次按键都走 activate + 轮询。
#[cfg(target_os = "macos")]
pub fn is_front_bundle(bundle_id: &str) -> bool {
    match frontmost_fast() {
        Some(front) => front
            .bundle_id
            .map(|b| b.eq_ignore_ascii_case(bundle_id))
            .unwrap_or(false),
        None => false,
    }
}

/// 慢速兜底：osascript 子进程查询（~100ms）。只给状态页展示用；
/// 按键热路径一律走 `frontmost_fast`。
pub fn frontmost_app() -> Result<String, String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(
            "tell application \"System Events\" to get name of first application process whose frontmost is true",
        )
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 热路径 FFI 必须真的能工作：前台 App 一定能拿到名字和 bundle id。
    /// 这个测试守住「\0 结尾」一类的静默失败 —— 之前 FFI 全程返回 None
    /// 还能让测试通过（走了 None 分支），延迟问题就这么溜进发布的。
    #[test]
    #[cfg(target_os = "macos")]
    fn fast_frontmost_must_work() {
        let info = frontmost_fast().expect("frontmost_fast 不可用 —— FFI 链路断了");
        assert!(!info.name.is_empty());
        assert!(
            info.bundle_id.is_some(),
            "前台 App {} 没有 bundle id",
            info.name
        );
    }

    /// WorkBuddy 实测 CFBundleExecutable=Electron，前台判定必须按 bundle id 而不是名字
    #[test]
    #[cfg(target_os = "macos")]
    fn resolves_running_app_by_bundle_id_or_name() {
        // Finder 一定在运行，bundle id 恒为 com.apple.finder
        let by_bid = resolve_running_target("com.apple.finder");
        assert!(by_bid.is_some(), "按 bundle id 解析运行中 App 失败");
        assert_eq!(by_bid.unwrap().bundle_id, "com.apple.finder");

        // 按名字解析：用前台 App 自己的名字（locale 无关 —— 中文系统里
        // Finder 的 localizedName 是「访达」，写死英文会挂）
        let front = frontmost_fast().expect("FFI 链路断了");
        let by_name = resolve_running_target(&front.name);
        assert!(by_name.is_some(), "按名字解析运行中 App 失败");

        // 不存在的目标必须返回 None，不能瞎编
        assert!(resolve_running_target("com.definitely.not.running.app").is_none());
    }
}
