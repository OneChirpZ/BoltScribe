use anyhow::Result;

const APP_NAME: &str = "BoltScribe";

pub(crate) fn apply_launch_at_login(enabled: bool) -> Result<()> {
    platform::apply_launch_at_login(enabled)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::APP_NAME;
    use anyhow::{anyhow, Context, Result};
    use std::fs;
    use std::path::PathBuf;

    const LAUNCH_AGENT_LABEL: &str = "cn.local.boltscribe";

    pub(crate) fn apply_launch_at_login(enabled: bool) -> Result<()> {
        if enabled {
            install_launch_agent()
        } else {
            remove_launch_agent()
        }
    }

    fn install_launch_agent() -> Result<()> {
        let path = launch_agent_path()?;
        if path.exists() && !is_managed_launch_agent(&path)? {
            return Err(anyhow!(
                "Launch agent {} already exists and is not managed by BoltScribe",
                path.display()
            ));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        fs::write(&path, launch_agent_plist())
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }

    fn remove_launch_agent() -> Result<()> {
        let path = launch_agent_path()?;
        if !path.exists() {
            return Ok(());
        }
        if !is_managed_launch_agent(&path)? {
            return Err(anyhow!(
                "Launch agent {} is not managed by BoltScribe",
                path.display()
            ));
        }
        fs::remove_file(&path).with_context(|| format!("Failed to remove {}", path.display()))?;
        Ok(())
    }

    fn is_managed_launch_agent(path: &PathBuf) -> Result<bool> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        Ok(raw.contains(LAUNCH_AGENT_LABEL) && raw.contains(APP_NAME))
    }

    fn launch_agent_path() -> Result<PathBuf> {
        Ok(dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot resolve user home directory"))?
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
    }

    fn launch_agent_plist() -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_AGENT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-a</string>
    <string>{APP_NAME}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn launch_agent_plist_targets_boltscribe() {
            let plist = launch_agent_plist();
            assert!(plist.contains("<string>cn.local.boltscribe</string>"));
            assert!(plist.contains("<string>BoltScribe</string>"));
            assert!(plist.contains("<string>/usr/bin/open</string>"));
        }
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::APP_NAME;
    use anyhow::{anyhow, Context, Result};
    use std::io;
    use std::path::Path;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE_NAME: &str = "BoltScribe";

    pub(crate) fn apply_launch_at_login(enabled: bool) -> Result<()> {
        if enabled {
            install_run_value()
        } else {
            remove_run_value()
        }
    }

    fn install_run_value() -> Result<()> {
        let exe = std::env::current_exe().context("Failed to resolve current executable")?;
        let command = quoted_command(&exe);
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu
            .create_subkey(RUN_KEY)
            .context("Failed to open Windows Run registry key")?;

        if let Ok(existing) = key.get_value::<String, _>(RUN_VALUE_NAME) {
            if !is_managed_run_value(&existing) {
                return Err(anyhow!(
                    "Run value {RUN_VALUE_NAME} already exists and is not managed by {APP_NAME}"
                ));
            }
        }

        key.set_value(RUN_VALUE_NAME, &command)
            .context("Failed to write Windows Run registry value")?;
        Ok(())
    }

    fn remove_run_value() -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let key = match hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE) {
            Ok(key) => key,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err).context("Failed to open Windows Run registry key"),
        };

        let existing = match key.get_value::<String, _>(RUN_VALUE_NAME) {
            Ok(existing) => existing,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err).context("Failed to read Windows Run registry value"),
        };
        if !is_managed_run_value(&existing) {
            return Err(anyhow!(
                "Run value {RUN_VALUE_NAME} exists and is not managed by {APP_NAME}"
            ));
        }

        key.delete_value(RUN_VALUE_NAME)
            .context("Failed to delete Windows Run registry value")?;
        Ok(())
    }

    fn quoted_command(path: &Path) -> String {
        format!("\"{}\"", path.display())
    }

    fn is_managed_run_value(value: &str) -> bool {
        value.to_ascii_lowercase().contains("boltscribe.exe")
    }

    #[test]
    fn run_value_management_detects_boltscribe_command() {
        assert!(is_managed_run_value(
            r#""C:\Users\me\AppData\Local\BoltScribe\boltscribe.exe""#
        ));
        assert!(!is_managed_run_value(r#""C:\Tools\other.exe""#));
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use anyhow::{anyhow, Result};

    pub(crate) fn apply_launch_at_login(enabled: bool) -> Result<()> {
        if enabled {
            Err(anyhow!(
                "Launch at login is not implemented on this platform"
            ))
        } else {
            Ok(())
        }
    }
}
