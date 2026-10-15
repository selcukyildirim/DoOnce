//! Finding, launching and attaching to Chrome for CLI runs.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::driver::DriverError;

/// Locates a Chrome or Chromium executable. `$DOONCE_CHROME` wins.
pub fn find_chrome() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("DOONCE_CHROME") {
        return Some(PathBuf::from(p));
    }
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        ]
    } else if cfg!(windows) {
        &[
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ]
    } else {
        &[]
    };
    for c in candidates {
        if Path::new(c).exists() {
            return Some(PathBuf::from(c));
        }
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let p = PathBuf::from(local).join(r"Google\Chrome\Application\chrome.exe");
            if p.exists() {
                return Some(p);
            }
        }
    }
    let names = ["google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "chrome"];
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

pub struct LaunchOptions {
    pub executable: PathBuf,
    /// Dedicated profile directory. Logins made here persist between runs.
    pub user_data_dir: PathBuf,
    pub headless: bool,
}

/// A Chrome process started by DoOnce. Killed on drop.
pub struct ChromeProcess {
    child: Child,
    pub ws_url: String,
}

impl ChromeProcess {
    pub async fn launch(opts: &LaunchOptions) -> Result<Self, DriverError> {
        std::fs::create_dir_all(&opts.user_data_dir)
            .map_err(|e| DriverError::Browser(format!("cannot create Chrome profile directory: {e}")))?;
        let port_file = opts.user_data_dir.join("DevToolsActivePort");
        let _ = std::fs::remove_file(&port_file);

        let mut cmd = Command::new(&opts.executable);
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", opts.user_data_dir.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--disable-sync",
                "--disable-features=Translate,OptimizationHints,MediaRouter",
                "--password-store=basic",
                "--use-mock-keychain",
            ]);
        if opts.headless {
            cmd.arg("--headless=new");
        }
        cmd.arg("about:blank").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| DriverError::Browser(format!("could not start Chrome at {}: {e}", opts.executable.display())))?;

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(contents) = std::fs::read_to_string(&port_file) {
                let mut lines = contents.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                    let ws_url = format!("ws://127.0.0.1:{}{}", port.trim(), path.trim());
                    return Ok(ChromeProcess { child, ws_url });
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(DriverError::Browser(format!(
                    "Chrome exited immediately ({status}). Is another Chrome already using the profile {}?",
                    opts.user_data_dir.display()
                )));
            }
            if Instant::now() > deadline {
                return Err(DriverError::Timeout("Chrome did not start within 30s".into()));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn close(mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
    }
}

/// Resolves the browser WebSocket URL of an already running Chrome started
/// with `--remote-debugging-port`. Accepts `ws://…` or `http://host:port`.
pub async fn resolve_ws_url(endpoint: &str) -> Result<String, DriverError> {
    if endpoint.starts_with("ws://") || endpoint.starts_with("wss://") {
        return Ok(endpoint.to_string());
    }
    let hostport = endpoint.trim_start_matches("http://").trim_end_matches('/');
    let host = hostport.split(':').next().unwrap_or_default();
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return Err(DriverError::Browser("only local DevTools endpoints are supported".into()));
    }
    let mut stream = tokio::net::TcpStream::connect(hostport)
        .await
        .map_err(|e| DriverError::Browser(format!("cannot reach Chrome DevTools at {endpoint}: {e}")))?;
    let request = format!("GET /json/version HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.map_err(|e| DriverError::Browser(e.to_string()))?;
    let mut response = String::new();
    stream.read_to_string(&mut response).await.map_err(|e| DriverError::Browser(e.to_string()))?;
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default();
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|_| DriverError::Browser("unexpected /json/version response".into()))?;
    json.get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| DriverError::Browser("Chrome did not report a DevTools WebSocket URL".into()))
}
