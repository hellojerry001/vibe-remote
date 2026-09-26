//! Shell 命令执行。注意：没有超时保护，映射长驻命令（如 `sleep 999`）会挂住动作线程，请慎用。

pub fn run(command: &str) -> Result<String, String> {
    let output = std::process::Command::new("/bin/zsh")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(format!(
            "exit {}{}",
            output.status,
            if stderr.is_empty() { String::new() } else { format!(": {stderr}") }
        ))
    }
}
