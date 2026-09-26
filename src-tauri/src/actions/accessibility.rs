//! 辅助功能动作：等价于旧 Swift 版「遍历前台 App UI 树找按钮」的能力，
//! 实现上走 System Events（osascript），同样是辅助功能权限体系，但免写 C 接口。

use super::applescript;

pub const APPROVE_WORDS: &[&str] = &["approve", "allow", "confirm", "accept", "批准", "允许", "确认"];
pub const REJECT_WORDS: &[&str] = &["reject", "decline", "deny", "cancel", "拒绝", "取消"];
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
						if class of el is button then
							set n to name of el
							if n is not missing value then
								repeat with kw in {{{keywords}}}
									if n contains (kw as string) then
										click el
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
