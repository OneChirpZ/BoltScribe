use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::PathBuf;

const LAUNCH_AGENT_LABEL: &str = "cn.local.boltscribe";
const APP_NAME: &str = "BoltScribe";

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
    let raw =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
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
