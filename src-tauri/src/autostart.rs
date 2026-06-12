use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::Command;

const LABEL: &str = "com.luiss0606.clipboard-saver";

fn plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

pub fn is_enabled() -> bool {
    plist_path().exists()
}

/// Writes a LaunchAgent plist pointing at the current executable.
/// The plist is only written, not bootstrapped with launchctl: bootstrapping
/// would launch a second instance immediately. RunAtLoad takes effect at the
/// next login/boot, which is the behavior we want.
pub fn enable() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        exe = exe.display()
    );
    let path = plist_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, plist)
}

pub fn disable() -> io::Result<()> {
    // Unregister in case a previous login already bootstrapped the agent;
    // failure is fine (it may simply not be loaded).
    let _ = Command::new("sh")
        .arg("-c")
        .arg(format!("launchctl bootout gui/$(id -u)/{LABEL}"))
        .output();
    fs::remove_file(plist_path())
}
