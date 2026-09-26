fn main() {
    tauri_build::build();

    // A2854 直连（仅 macOS）：
    //   · siri_remote_bridge.c —— IOHIDManager 监听 HID 按键 + HID 权限查询
    //
    // 蓝牙那层**故意没有** .m 桥接：macOS 13+ 上进程内碰蓝牙（IOBluetooth.framework
    // 或 CoreBluetooth）会被 TCC 直接 abort，system_profiler 子进程才是唯一可行路径。
    // 详见 remote/bluetooth.rs 顶部说明。
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "macos" {
        cc::Build::new()
            .file("native/siri_remote_bridge.c")
            .file("native/remote_multitouch.c")
            .compile("siri_remote_bridge");

        println!("cargo:rustc-link-lib=framework=IOKit");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rerun-if-changed=native/siri_remote_bridge.c");
        println!("cargo:rerun-if-changed=native/remote_multitouch.c");
    }
}
