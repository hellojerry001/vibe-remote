//! AppleScript 执行：通过 osascript 子进程，天然隔离崩溃风险。

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn run(script: &str) -> Result<String, String> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| e.to_string())?;
    collect(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    )
}

/// 带超时的执行：超时 kill 子进程并返回 Err("timeout:...")。
///
/// 用途：AX「entire contents」扫描在大窗口上可能秒级，热路径（确认/返回键）
/// 必须有界——超时后调用方走按键回退，最坏延迟 = 超时值。
pub fn run_with_timeout(script: &str, timeout: Duration) -> Result<String, String> {
    let mut child = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timeout:{:?}，已回退", timeout));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e.to_string()),
        }
    };

    // 子进程已退出，管道读到 EOF，不会阻塞
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut p) = child.stdout.take() {
        use std::io::Read;
        let _ = p.read_to_string(&mut stdout);
    }
    if let Some(mut p) = child.stderr.take() {
        use std::io::Read;
        let _ = p.read_to_string(&mut stderr);
    }
    collect(status.success(), &stdout, &stderr)
}

fn collect(success: bool, stdout: &str, stderr: &str) -> Result<String, String> {
    let stdout = stdout.trim();
    let stderr = stderr.trim();
    if success {
        Ok(if stdout.is_empty() { stderr.to_string() } else { stdout.to_string() })
    } else if stderr.is_empty() {
        Err(format!("osascript 失败：{stdout}"))
    } else {
        Err(stderr.to_string())
    }
}
