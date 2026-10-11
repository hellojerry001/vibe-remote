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

/// 在前台 App 的所有窗口中查找标题含关键词的按钮并点击。
/// 返回 Ok("clicked:按钮名")；找不到/超时返回 Err。
pub fn click_frontmost_button(words: &[&str], timeout: std::time::Duration) -> Result<String, String> {
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
