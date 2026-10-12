//! 辅助功能动作：等价于旧 Swift 版「遍历前台 App UI 树找按钮」的能力，
//! 实现上走 System Events（osascript），同样是辅助功能权限体系，但免写 C 接口。

use super::applescript;

pub const APPROVE_WORDS: &[&str] = &[
    "approve",
    "allow",
    "confirm",
    "accept",
    "批准",
    "允许",
    "确认",
    "同意",
    "确定",
    "好",
    "是",
    "ok",
    "yes",
];
pub const REJECT_WORDS: &[&str] = &[
    "reject",
    "decline",
    "deny",
    "don't allow",
    "do not allow",
    "not now",
    "cancel",
    "不允许",
    "拒绝",
    "取消",
    "否",
    "no",
    "不用",
    "稍后",
    "暂不",
];
/// 「全部允许」的按钮文案变体；WorkBuddy 改文案时在这里补
pub const APPROVE_ALL_WORDS: &[&str] = &[
    "全部允许",
    "总是允许",
    "允许全部",
    "始终允许",
    "allow all",
    "always allow",
    "allow always",
];

#[cfg(target_os = "macos")]
unsafe extern "C" {
    /// 进程内 AX 点击：在会话焦点 App 的 AX 树里找文案含关键词的可点元素并 AXPress。
    /// 返回 1=点到 / 0=遍历过没找到 / -1=拿不到 AX（多半没辅助功能授权）。
    /// 只用「辅助功能」授权，不需「自动化」——这是相对 osascript 路径的关键优势。
    fn wc_ax_click_button(
        words: *const *const std::os::raw::c_char,
        count: std::os::raw::c_int,
        out: *mut std::os::raw::c_char,
        outlen: std::os::raw::c_int,
    ) -> std::os::raw::c_int;
}

/// 进程内 AX 点击。返回值语义：
/// - `Some(Ok(name))`：点到，按钮文案是 name；
/// - `Some(Err(reason))`：AX 可用、已尽力（含启用 Electron AX 树 + 退避重试）但没点到，
///   reason 是可读原因，直接抛给上层，**不再跑 osascript**（省掉无谓的秒级挂起）；
/// - `None`：AX 整体不可用（拿不到焦点 App，多半没辅助功能授权），调用方才去兜底 osascript。
#[cfg(target_os = "macos")]
fn ax_click_frontmost_button(words: &[&str]) -> Option<Result<String, String>> {
    use std::ffi::{CStr, CString};
    let cwords: Vec<CString> = words
        .iter()
        .filter_map(|w| CString::new(*w).ok())
        .collect();
    let ptrs: Vec<*const std::os::raw::c_char> = cwords.iter().map(|c| c.as_ptr()).collect();
    let mut out = [0i8; 512];
    let rc = unsafe {
        wc_ax_click_button(
            ptrs.as_ptr(),
            ptrs.len() as std::os::raw::c_int,
            out.as_mut_ptr(),
            out.len() as std::os::raw::c_int,
        )
    };
    match rc {
        1 => {
            let name = unsafe { CStr::from_ptr(out.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            Some(Ok(name))
        }
        0 => Some(Err(
            "AX 树中未找到匹配按钮（已启用 Electron 辅助功能树并重试）".to_string(),
        )),
        _ => None,
    }
}

/// 在前台 App 的所有窗口中查找标题含关键词的按钮并点击。
/// 返回 Ok("clicked:按钮名")；找不到/超时返回 Err。
///
/// 首选进程内 AX API（只需辅助功能授权）；仅当 AX 整体不可用时，
/// 才退回 osascript System Events（那条路要「自动化」授权）。
pub fn click_frontmost_button(words: &[&str], timeout: std::time::Duration) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        match ax_click_frontmost_button(words) {
            Some(Ok(name)) => return Ok(format!("clicked:{name}")),
            Some(Err(reason)) => return Err(reason),
            None => { /* AX 不可用：落到 osascript 兜底 */ }
        }
    }
    let result = applescript::run_with_timeout(&build_script(words), timeout)?;
    if result.starts_with("clicked:") {
        Ok(result)
    } else {
        Err(result)
    }
}

fn build_script(words: &[&str]) -> String {
    let keywords = words
        .iter()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"tell application "System Events"
	set procName to name of first application process whose frontmost is true
	tell application process procName
		repeat with w in windows
			try
				set els to entire contents of w
				repeat with el in els
					try
						set c to class of el
						-- Electron / 自定义弹窗里可点的元素不一定是原生 button：
						-- 可能是 UI element、static text（带 onClick）、radio/checkbox。
						-- 这些都纳入候选，避免「按钮存在但 class 不是 button 而点不到」。
						if c is button or c is UI element or c is static text or c is radio button or c is checkbox then
							set n to name of el
							if n is not missing value and n is not "" then
								-- 第一遍精确匹配：避免「允许」优先匹配到「始终允许」
								repeat with kw in {{{keywords}}}
									if n is (kw as string) then
										try
											click el
										on error
											try
												perform action 1 of el
											end try
										end try
										return "clicked:" & n
									end if
								end repeat
								-- 第二遍子串兜底。AppleScript 文本比较默认就大小写不敏感，
								-- 直接 contains 即可；不要用 `my lower of x` 这种写法——
								-- `of` 是属性访问语法，会被当成取属性而不是 handler 调用，
								-- 整段脚本直接编译失败（此前批准/拒绝全失效的根因）。
								repeat with kw in {{{keywords}}}
									if n contains (kw as string) then
										try
											click el
										on error
											try
												perform action 1 of el
											end try
										end try
										return "clicked:" & n
									end if
								end repeat
							end if
						end if
					end try
				end repeat
			end try
		end repeat
	end tell
end tell
return "not-found""#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_contains_keywords() {
        let s = build_script(&["approve", "允许"]);
        assert!(s.contains("\"approve\""));
        assert!(s.contains("\"允许\""));
        assert!(s.contains("entire contents"));
    }
}
