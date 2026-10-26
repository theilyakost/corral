use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TASK: &str = "Corral";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const AGENT_DLL: &str = "corral_agent.dll";

fn schtasks(args: &[&str]) -> Result<Output, String> {
    Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())
}

fn check(output: Output) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if text.is_empty() { format!("schtasks: {}", output.status) } else { text })
}

pub fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Programs").join("Corral")
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("corral.exe")
}

pub fn is_installed_copy() -> bool {
    let current = std::env::current_exe().and_then(|p| p.canonicalize());
    let installed = installed_exe().canonicalize();
    matches!((current, installed), (Ok(a), Ok(b)) if a == b)
}

pub fn enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK]).is_ok_and(|o| o.status.success())
}

pub fn disable() -> Result<(), String> {
    check(schtasks(&["/Delete", "/TN", TASK, "/F"])?)
}

pub fn enable() -> Result<(), String> {
    let exe = install()?;
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let file = std::env::temp_dir().join("corral-task.xml");
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(task_xml(&exe, &user).encode_utf16().flat_map(u16::to_le_bytes));
    fs::write(&file, bytes).map_err(|e| e.to_string())?;
    let result = schtasks(&["/Create", "/TN", TASK, "/XML", &file.to_string_lossy(), "/F"]).and_then(check);
    _ = fs::remove_file(&file);
    result
}

pub fn run_task() -> Result<(), String> {
    check(schtasks(&["/Run", "/TN", TASK])?)
}

fn install() -> Result<PathBuf, String> {
    let target = installed_exe();
    if is_installed_copy() {
        return Ok(target);
    }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = install_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("{}\n\n{e}", dir.display()))?;
    copy(&current, &target)?;
    copy(&current.with_file_name(AGENT_DLL), &dir.join(AGENT_DLL))?;
    Ok(target)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to).map(|_| ()).map_err(|e| format!("{} → {}\n\n{e}", from.display(), to.display()))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn task_xml(exe: &Path, user: &str) -> String {
    let command = escape(&exe.to_string_lossy());
    let directory = escape(&exe.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
    let user = escape(user);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Corral: desktop icon groups</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>4</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <WorkingDirectory>{directory}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_xml_escapes_paths() {
        let xml = task_xml(Path::new(r"C:\A & B\corral.exe"), r"PC\Илья");
        assert!(xml.contains(r"<Command>C:\A &amp; B\corral.exe</Command>"));
        assert!(xml.contains(r"<WorkingDirectory>C:\A &amp; B</WorkingDirectory>"));
        assert!(xml.contains(r"<UserId>PC\Илья</UserId>"));
        assert!(xml.contains("<Priority>4</Priority>"));
    }
}
