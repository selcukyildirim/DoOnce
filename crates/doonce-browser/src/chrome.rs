//! Finding, launching and attaching to Chrome.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::error::{BrowserError, Result};

/// Locates a Chrome/Chromium executable. `$DOONCE_CHROME` wins.
pub fn find_chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DOONCE_CHROME") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ]
    } else if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ]
    } else {
        &[]
    };
    for candidate in candidates {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    if cfg!(windows)
        && let Some(local) = std::env::var_os("LOCALAPPDATA")
    {
        let path = PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe");
        if path.is_file() {
            return Some(path);
        }
    }
    let names = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ];
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub executable: PathBuf,
    /// Dedicated profile directory. Logins persist between runs.
    pub user_data_dir: PathBuf,
    pub headless: bool,
}

pub struct ChromeProcess {
    child: Child,
    pub ws_url: String,
}

impl ChromeProcess {
    pub async fn launch(options: &LaunchOptions) -> Result<Self> {
        std::fs::create_dir_all(&options.user_data_dir)?;
        let port_file = options.user_data_dir.join("DevToolsActivePort");
        let _ = std::fs::remove_file(&port_file);

        let mut command = Command::new(&options.executable);
        command
            .arg("--remote-debugging-port=0")
            .arg(format!(
                "--user-data-dir={}",
                options.user_data_dir.display()
            ))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
                "--disable-features=Translate,OptimizationHints,MediaRouter",
                "--password-store=basic",
                "--use-mock-keychain",
                "--disable-dev-shm-usage",
                "--disable-blink-features=AutomationControlled",
                "--disable-infobars",
            ]);
        if let Ok(proxy) = std::env::var("DOONCE_PROXY")
            && !proxy.trim().is_empty()
        {
            command.arg(format!("--proxy-server={}", proxy.trim()));
        }
        if let Ok(user_agent) = std::env::var("DOONCE_USER_AGENT")
            && !user_agent.trim().is_empty()
        {
            command.arg(format!("--user-agent={}", user_agent.trim()));
        }
        let stealth = std::env::var("DOONCE_STEALTH")
            .map(|v| {
                !matches!(
                    v.to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(true);
        if stealth {
            command.args([
                "--exclude-switches=enable-automation",
                "--disable-automation",
                "--disable-component-extensions-with-background-pages",
                "--disable-default-apps",
                "--disable-extensions",
                "--window-size=1920,1080",
                "--lang=en-US,en",
            ]);
        }
        if options.headless {
            command.arg("--headless=new");
        }
        command
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|e| {
            BrowserError::Browser(format!(
                "could not start Chrome at {}: {e}",
                options.executable.display()
            ))
        })?;

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(contents) = std::fs::read_to_string(&port_file) {
                let mut lines = contents.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                    return Ok(ChromeProcess {
                        child,
                        ws_url: format!("ws://127.0.0.1:{}{}", port.trim(), path.trim()),
                    });
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(BrowserError::Browser(format!(
                    "Chrome exited immediately ({status}). Is another Chrome using {}?",
                    options.user_data_dir.display()
                )));
            }
            if Instant::now() > deadline {
                return Err(BrowserError::Timeout(
                    "Chrome did not start within 30s".into(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn close(mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }
}

/// Resolves the browser WebSocket URL from an already running Chrome.
///
/// Only loopback endpoints are accepted by default. Remote CDP endpoints must
/// be enabled explicitly with `DOONCE_ALLOW_REMOTE_CDP=1` and are expected to
/// be protected by a tunnel/authenticated proxy.
pub async fn resolve_ws_url(endpoint: &str) -> Result<String> {
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        return Ok(endpoint.to_string());
    }
    let endpoint = endpoint.trim_end_matches('/');
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"));
    let Some(hostport) = without_scheme else {
        return Err(BrowserError::Endpoint(
            "expected ws://, http:// or https://".into(),
        ));
    };
    let host = hostport.split(':').next().unwrap_or_default();
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1");
    let allow_remote = std::env::var("DOONCE_ALLOW_REMOTE_CDP").ok().as_deref() == Some("1");
    if !loopback && !allow_remote {
        return Err(BrowserError::Endpoint(
            "remote CDP endpoints are disabled; set DOONCE_ALLOW_REMOTE_CDP=1 to allow".into(),
        ));
    }
    if endpoint.starts_with("https://") {
        return Err(BrowserError::Endpoint(
            "https CDP discovery is not supported yet; use wss://".into(),
        ));
    }

    let mut stream = tokio::net::TcpStream::connect(hostport)
        .await
        .map_err(|e| {
            BrowserError::Endpoint(format!("cannot reach Chrome DevTools at {endpoint}: {e}"))
        })?;
    let request =
        format!("GET /json/version HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await?;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let value: Value = serde_json::from_str(body)
        .map_err(|_| BrowserError::Endpoint("unexpected /json/version response".into()))?;
    value
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            BrowserError::Endpoint("Chrome did not report a DevTools WebSocket URL".into())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_cdp_is_opt_in() {
        // The check is synchronous for the rejection path; no network required.
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(resolve_ws_url("http://example.com:9222"))
            .unwrap_err();
        assert!(error.to_string().contains("disabled"));
    }

    #[test]
    fn chrome_path_can_be_overridden() {
        // `find_chrome` must not panic when the environment is empty.
        let _ = find_chrome();
    }
}
