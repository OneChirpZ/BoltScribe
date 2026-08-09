#[cfg(not(target_os = "macos"))]
use anyhow::anyhow;
use anyhow::Result;
#[cfg(target_os = "macos")]
use anyhow::{bail, Context};
#[cfg(target_os = "macos")]
use std::{process::Command, thread, time::Duration};

#[cfg(target_os = "macos")]
const RESTART_COREAUDIOD_SCRIPT: &str =
    r#"do shell script "/usr/bin/killall coreaudiod" with administrator privileges"#;
#[cfg(target_os = "macos")]
const SERVICE_RELAUNCH_SETTLE_DELAY: Duration = Duration::from_millis(800);

#[cfg(target_os = "macos")]
pub(crate) fn restart_service() -> Result<bool> {
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", RESTART_COREAUDIOD_SCRIPT])
        .output()
        .context("Failed to request macOS administrator authorization")?;

    if output.status.success() {
        // launchd restarts coreaudiod automatically. Give the replacement
        // process a brief head start before the UI refreshes the device list.
        thread::sleep(SERVICE_RELAUNCH_SETTLE_DELAY);
        return Ok(true);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if authorization_was_cancelled(&stderr) {
        return Ok(false);
    }

    let detail = stderr.trim();
    if detail.is_empty() {
        bail!(
            "macOS did not restart the system audio service (osascript exited with {})",
            output.status
        );
    }
    bail!("macOS did not restart the system audio service: {detail}");
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn restart_service() -> Result<bool> {
    Err(anyhow!(
        "Restarting the system audio service is only supported on macOS"
    ))
}

#[cfg(target_os = "macos")]
fn authorization_was_cancelled(stderr: &str) -> bool {
    // AppleScript error -128 is the stable cancellation code even when the
    // surrounding error message is localized by macOS.
    stderr.contains("(-128)")
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::authorization_was_cancelled;

    #[test]
    fn detects_localized_authorization_cancellation() {
        assert!(authorization_was_cancelled(
            "execution error: 用户已取消。 (-128)"
        ));
        assert!(!authorization_was_cancelled(
            "execution error: /usr/bin/killall failed. (1)"
        ));
    }
}
