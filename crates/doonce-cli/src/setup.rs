//! Registers the `doonce` binary as a Chrome Native Messaging host so the
//! browser extension can talk to it. Nothing listens on the network: Chrome
//! starts the host itself and only for the allowed extension id.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::json;

pub const HOST_NAME: &str = "com.doonce.host";
/// Id of the unpacked extension in `extension/`, fixed by the `key` in its
/// manifest.
pub const DEFAULT_EXTENSION_ID: &str = "mmninaegachgmioebddmlccpjdkcdefb";

/// Per-browser directories that hold user-level native messaging manifests.
fn browser_dirs() -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(base) = dirs::home_dir().map(|h| h.join("Library/Application Support")) {
            for (name, rel) in [
                ("Google Chrome", "Google/Chrome"),
                ("Chrome for Testing", "Google/Chrome for Testing"),
                ("Chrome Canary", "Google/Chrome Canary"),
                ("Chromium", "Chromium"),
                ("Microsoft Edge", "Microsoft Edge"),
                ("Brave", "BraveSoftware/Brave-Browser"),
            ] {
                out.push((name, base.join(rel)));
            }
        }
    } else if cfg!(target_os = "linux") {
        if let Some(base) = dirs::config_dir() {
            for (name, rel) in [
                ("Google Chrome", "google-chrome"),
                ("Chrome for Testing", "google-chrome-for-testing"),
                ("Chromium", "chromium"),
                ("Microsoft Edge", "microsoft-edge"),
                ("Brave", "BraveSoftware/Brave-Browser"),
            ] {
                out.push((name, base.join(rel)));
            }
        }
    }
    out
}

pub fn manifest_json(binary: &Path, extension_ids: &[String]) -> serde_json::Value {
    json!({
        "name": HOST_NAME,
        "description": "DoOnce automation engine",
        "path": binary,
        "type": "stdio",
        "allowed_origins": extension_ids.iter().map(|id| format!("chrome-extension://{id}/")).collect::<Vec<_>>(),
    })
}

fn validate_id(id: &str) -> Result<()> {
    if id.len() != 32 || !id.chars().all(|c| ('a'..='p').contains(&c)) {
        bail!("\"{id}\" is not a Chrome extension id (32 letters a-p)");
    }
    Ok(())
}

pub struct SetupReport {
    pub installed: Vec<(String, PathBuf)>,
}

/// Installs the host manifest for every browser found on this machine, or
/// into `profile_dir` only (custom `--user-data-dir` profiles).
pub fn install(extension_ids: &[String], profile_dir: Option<&Path>) -> Result<SetupReport> {
    for id in extension_ids {
        validate_id(id)?;
    }
    let binary = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .context("cannot determine the path of the doonce binary")?;
    let manifest = serde_json::to_string_pretty(&manifest_json(&binary, extension_ids))?;
    let mut installed = Vec::new();

    if cfg!(windows) && profile_dir.is_none() {
        let path = doonce_storage::data_dir().join(format!("{HOST_NAME}.json"));
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, &manifest)?;
        for key in [
            r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
            r"HKCU\Software\Chromium\NativeMessagingHosts",
            r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
        ] {
            let status = std::process::Command::new("reg")
                .args(["add", &format!(r"{key}\{HOST_NAME}"), "/ve", "/t", "REG_SZ", "/d"])
                .arg(&path)
                .arg("/f")
                .status()
                .context("could not run reg.exe")?;
            if status.success() {
                installed.push((key.to_string(), path.clone()));
            }
        }
        return Ok(SetupReport { installed });
    }

    let targets: Vec<(String, PathBuf)> = match profile_dir {
        Some(dir) => vec![("Custom profile".into(), dir.to_path_buf())],
        None => browser_dirs().into_iter().filter(|(_, d)| d.exists()).map(|(n, d)| (n.to_string(), d)).collect(),
    };
    if targets.is_empty() {
        bail!("no Chrome-based browser profile found; start Chrome once, or pass --profile-dir");
    }
    for (name, dir) in targets {
        let hosts = dir.join("NativeMessagingHosts");
        let path = hosts.join(format!("{HOST_NAME}.json"));
        let result = (|| -> Result<()> {
            std::fs::create_dir_all(&hosts).with_context(|| format!("cannot create {}", hosts.display()))?;
            std::fs::write(&path, &manifest).with_context(|| format!("cannot write {}", path.display()))?;
            Ok(())
        })();
        match result {
            Ok(()) => installed.push((name, path)),
            Err(e) => eprintln!("warning: skipped {name}: {e:#}"),
        }
    }
    if installed.is_empty() {
        bail!("could not install the native host in any browser profile");
    }
    Ok(SetupReport { installed })
}

pub fn uninstall() -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for (_, dir) in browser_dirs() {
        let path = dir.join("NativeMessagingHosts").join(format!("{HOST_NAME}.json"));
        if path.exists() {
            std::fs::remove_file(&path)?;
            removed.push(path);
        }
    }
    if cfg!(windows) {
        for key in [
            r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
            r"HKCU\Software\Chromium\NativeMessagingHosts",
            r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
        ] {
            let _ = std::process::Command::new("reg")
                .args(["delete", &format!(r"{key}\{HOST_NAME}"), "/f"])
                .status();
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_exact_origins() {
        let m = manifest_json(Path::new("/usr/local/bin/doonce"), &[DEFAULT_EXTENSION_ID.to_string()]);
        assert_eq!(m["name"], HOST_NAME);
        assert_eq!(m["allowed_origins"][0], format!("chrome-extension://{DEFAULT_EXTENSION_ID}/"));
        assert!(validate_id(DEFAULT_EXTENSION_ID).is_ok());
        assert!(validate_id("*").is_err());
    }

    #[test]
    fn installs_into_custom_profile() {
        let dir = tempfile::tempdir().unwrap();
        let report = install(&[DEFAULT_EXTENSION_ID.to_string()], Some(dir.path())).unwrap();
        let written = std::fs::read_to_string(&report.installed[0].1).unwrap();
        assert!(written.contains(DEFAULT_EXTENSION_ID));
    }
}
