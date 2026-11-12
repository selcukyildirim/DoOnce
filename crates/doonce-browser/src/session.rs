//! A CDP browser session with an isolated copy of the DoOnce page agent.
//!
//! The page agent is evaluated in an isolated world, so page scripts cannot
//! tamper with locator or action logic. Every call re-creates the world after a
//! navigation.

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::PAGE_AGENT_JS;
use crate::chrome::{ChromeProcess, LaunchOptions, resolve_ws_url};
use crate::connection::{CdpEvent, Connection};
use crate::error::{BrowserError, Result};

const NAVIGATION_GRACE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    pub filename: String,
    pub path: Option<PathBuf>,
    pub bytes: Option<u64>,
}

pub struct BrowserSession {
    conn: Connection,
    session_id: String,
    target_id: String,
    context_id: Option<i64>,
    download_dir: PathBuf,
    process: Option<ChromeProcess>,
    downloads: Option<broadcast::Receiver<CdpEvent>>,
}

impl std::fmt::Debug for BrowserSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSession")
            .field("target_id", &self.target_id)
            .field("download_dir", &self.download_dir)
            .finish_non_exhaustive()
    }
}

impl BrowserSession {
    pub async fn launch(options: &LaunchOptions, download_dir: &Path) -> Result<Self> {
        let process = ChromeProcess::launch(options).await?;
        let conn = Connection::connect(&process.ws_url).await?;
        let targets = conn.send(None, "Target.getTargets", json!({})).await?;
        let existing = targets["targetInfos"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["type"] == "page"))
            .and_then(|item| item["targetId"].as_str())
            .map(str::to_string);
        let target_id = match existing {
            Some(id) => id,
            None => create_tab(&conn).await?,
        };
        Self::attach(conn, target_id, download_dir, Some(process)).await
    }

    pub async fn connect(endpoint: &str, download_dir: &Path) -> Result<Self> {
        let ws_url = resolve_ws_url(endpoint).await?;
        let conn = Connection::connect(&ws_url).await?;
        let target_id = create_tab(&conn).await?;
        Self::attach(conn, target_id, download_dir, None).await
    }

    async fn attach(
        conn: Connection,
        target_id: String,
        download_dir: &Path,
        process: Option<ChromeProcess>,
    ) -> Result<Self> {
        std::fs::create_dir_all(download_dir)?;
        let mut session = BrowserSession {
            conn,
            session_id: String::new(),
            target_id: String::new(),
            context_id: None,
            download_dir: download_dir.to_path_buf(),
            process,
            downloads: None,
        };
        session.attach_target(&target_id).await?;
        // Browser-level download handling covers tabs opened later as well.
        session
            .conn
            .send(
                None,
                "Browser.setDownloadBehavior",
                json!({
                    "behavior": "allowAndName",
                    "downloadPath": download_dir.display().to_string(),
                    "eventsEnabled": true,
                }),
            )
            .await?;
        Ok(session)
    }

    async fn attach_target(&mut self, target_id: &str) -> Result<()> {
        let response = self
            .conn
            .send(
                None,
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let session_id = response["sessionId"]
            .as_str()
            .ok_or_else(|| BrowserError::Browser("Chrome did not return a session id".into()))?;
        self.session_id = session_id.to_string();
        self.target_id = target_id.to_string();
        self.context_id = None;
        self.conn
            .send(Some(&self.session_id), "Page.enable", json!({}))
            .await?;
        self.conn
            .send(
                None,
                "Target.activateTarget",
                json!({ "targetId": target_id }),
            )
            .await?;
        Ok(())
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub fn download_dir(&self) -> &Path {
        &self.download_dir
    }

    async fn ensure_context(&mut self) -> Result<i64> {
        if let Some(id) = self.context_id {
            return Ok(id);
        }
        let tree = self
            .conn
            .send(Some(&self.session_id), "Page.getFrameTree", json!({}))
            .await?;
        let frame_id = tree["frameTree"]["frame"]["id"]
            .as_str()
            .unwrap_or(&self.target_id)
            .to_string();
        let world = self
            .conn
            .send(
                Some(&self.session_id),
                "Page.createIsolatedWorld",
                json!({ "frameId": frame_id, "worldName": "doonce", "grantUniveralAccess": false }),
            )
            .await?;
        let context_id = world["executionContextId"].as_i64().ok_or_else(|| {
            BrowserError::Transient("Chrome did not create an isolated world".into())
        })?;
        let loaded = self
            .conn
            .send(
                Some(&self.session_id),
                "Runtime.evaluate",
                json!({ "expression": PAGE_AGENT_JS, "contextId": context_id, "returnByValue": true }),
            )
            .await?;
        if let Some(exception) = loaded.get("exceptionDetails") {
            return Err(BrowserError::Browser(format!(
                "page agent failed to load: {}",
                exception_text(exception)
            )));
        }
        self.context_id = Some(context_id);
        Ok(context_id)
    }

    async fn call(&mut self, method: &str, args: &Value) -> Result<Value> {
        let expression = format!(
            "globalThis.__doonce.call({}, {})",
            serde_json::to_string(method).map_err(|e| BrowserError::Protocol(e.to_string()))?,
            args
        );
        for attempt in 0..2 {
            let context_id = self.ensure_context().await?;
            let response = self
                .conn
                .send(
                    Some(&self.session_id),
                    "Runtime.evaluate",
                    json!({
                        "expression": expression,
                        "contextId": context_id,
                        "returnByValue": true,
                        "awaitPromise": true,
                    }),
                )
                .await;
            match response {
                Err(BrowserError::Transient(message))
                    if message.contains("Cannot find context") && attempt == 0 =>
                {
                    self.context_id = None;
                }
                Err(error) => {
                    if matches!(error, BrowserError::Transient(_)) {
                        self.context_id = None;
                    }
                    return Err(error);
                }
                Ok(response) => {
                    if let Some(exception) = response.get("exceptionDetails") {
                        let text = exception_text(exception);
                        if (text.contains("__doonce") || text.contains("Cannot find context"))
                            && attempt == 0
                        {
                            self.context_id = None;
                            continue;
                        }
                        return Err(BrowserError::Browser(text));
                    }
                    return Ok(response["result"]["value"].clone());
                }
            }
        }
        Err(BrowserError::Transient("page agent is unavailable".into()))
    }

    pub async fn query(&mut self, method: &str, args: Value) -> Result<Value> {
        self.call(method, &args).await
    }

    pub async fn ping(&mut self) -> Result<Value> {
        self.call("ping", &json!({})).await
    }

    pub async fn navigate(&mut self, url: &str, timeout: Duration) -> Result<()> {
        self.context_id = None;
        let mut events = self.conn.subscribe();
        let response = self
            .conn
            .send_timeout(
                Some(&self.session_id),
                "Page.navigate",
                json!({ "url": url }),
                timeout,
            )
            .await?;
        if let Some(error) = response["errorText"]
            .as_str()
            .filter(|value| !value.is_empty())
        {
            return Err(BrowserError::Browser(format!(
                "could not open the page ({error})"
            )));
        }
        if response.get("loaderId").is_none() {
            return Ok(());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) => return self.accept_if_interactive().await,
                Ok(Err(RecvError::Lagged(_))) => continue,
                Ok(Err(RecvError::Closed)) => return Err(BrowserError::Disconnected),
                Ok(Ok(event))
                    if event.method == "Page.loadEventFired"
                        && event.session_id.as_deref() == Some(&self.session_id) =>
                {
                    return Ok(());
                }
                Ok(Ok(_)) => {}
            }
        }
    }

    async fn settle(
        &mut self,
        mut events: broadcast::Receiver<CdpEvent>,
        timeout: Duration,
    ) -> Result<()> {
        let grace_end = tokio::time::Instant::now() + NAVIGATION_GRACE;
        let mut loading = false;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let until = if loading { deadline } else { grace_end };
            let event = match tokio::time::timeout_at(until, events.recv()).await {
                Err(_) if !loading => return Ok(()),
                Err(_) => return self.accept_if_interactive().await,
                Ok(Err(RecvError::Lagged(_))) => continue,
                Ok(Err(RecvError::Closed)) => return Err(BrowserError::Disconnected),
                Ok(Ok(event)) => event,
            };
            if event.session_id.as_deref() != Some(&self.session_id) {
                continue;
            }
            let main_frame = event.params["frameId"].as_str() == Some(&self.target_id);
            match event.method.as_str() {
                "Page.frameStartedLoading" | "Page.frameRequestedNavigation" if main_frame => {
                    loading = true;
                    self.context_id = None;
                }
                "Page.loadEventFired" if loading => return Ok(()),
                "Page.frameStoppedLoading" if loading && main_frame => return Ok(()),
                _ => {}
            }
        }
    }

    async fn accept_if_interactive(&mut self) -> Result<()> {
        let state = self.call("ping", &json!({})).await?;
        match state["readyState"].as_str() {
            Some("interactive" | "complete") => Ok(()),
            _ => Err(BrowserError::Timeout("page did not finish loading".into())),
        }
    }

    pub async fn perform(&mut self, args: Value, timeout: Duration) -> Result<Value> {
        let action = args["action"].as_str().unwrap_or_default().to_string();
        let may_navigate = matches!(action.as_str(), "click" | "press");
        let events = self.conn.subscribe();
        let result = match self.call("perform", &args).await {
            Ok(value) => value,
            Err(BrowserError::Transient(message))
                if message.contains("destroyed") || message.contains("navigated") =>
            {
                json!({ "ok": true, "navigated": true })
            }
            Err(error) => return Err(error),
        };
        if action == "hover"
            && result["ok"] == true
            && let Some((x, y)) = center_of(&result["rect"])
        {
            self.conn
                .send(
                    Some(&self.session_id),
                    "Input.dispatchMouseEvent",
                    json!({ "type": "mouseMoved", "x": x, "y": y, "button": "none", "buttons": 0, "clickCount": 0 }),
                )
                .await?;
        }
        if let Some(url) = result["openUrl"].as_str() {
            if doonce_core::validation::is_allowed_url(url) {
                self.conn
                    .send(None, "Target.createTarget", json!({ "url": url }))
                    .await?;
            }
            return Ok(result);
        }
        if may_navigate && result["ok"] == true {
            self.settle(events, timeout).await?;
        }
        Ok(result)
    }

    pub async fn screenshot(&mut self) -> Result<Vec<u8>> {
        let response = self
            .conn
            .send(
                Some(&self.session_id),
                "Page.captureScreenshot",
                json!({ "format": "png", "fromSurface": true }),
            )
            .await?;
        let data = response["data"]
            .as_str()
            .ok_or_else(|| BrowserError::Browser("Chrome returned no screenshot data".into()))?;
        STANDARD
            .decode(data)
            .map_err(|e| BrowserError::Protocol(format!("invalid screenshot base64: {e}")))
    }

    pub fn expect_download(&mut self) {
        self.downloads = Some(self.conn.subscribe());
    }

    pub async fn wait_download(&mut self, timeout: Duration) -> Result<DownloadInfo> {
        let mut events = self
            .downloads
            .take()
            .ok_or_else(|| BrowserError::Browser("download was not expected".into()))?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut started: Option<(String, String)> = None;
        loop {
            let event = match tokio::time::timeout_at(deadline, events.recv()).await {
                Err(_) => {
                    let what = if started.is_some() {
                        "download did not finish"
                    } else {
                        "no download started"
                    };
                    return Err(BrowserError::Timeout(format!(
                        "{what} within {}s",
                        timeout.as_secs()
                    )));
                }
                Ok(Err(RecvError::Lagged(_))) => continue,
                Ok(Err(RecvError::Closed)) => return Err(BrowserError::Disconnected),
                Ok(Ok(event)) => event,
            };
            match event.method.as_str() {
                "Browser.downloadWillBegin" if started.is_none() => {
                    let guid = event.params["guid"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let filename = event.params["suggestedFilename"]
                        .as_str()
                        .unwrap_or("download")
                        .to_string();
                    started = Some((guid, filename));
                }
                "Browser.downloadProgress" => {
                    let Some((guid, filename)) = &started else {
                        continue;
                    };
                    if event.params["guid"].as_str() != Some(guid) {
                        continue;
                    }
                    match event.params["state"].as_str() {
                        Some("completed") => {
                            let saved = self.download_dir.join(guid);
                            let path = unique_path(&self.download_dir, filename);
                            let final_path = match std::fs::rename(&saved, &path) {
                                Ok(()) => Some(path),
                                Err(_) => saved.exists().then_some(saved),
                            };
                            return Ok(DownloadInfo {
                                filename: filename.clone(),
                                path: final_path,
                                bytes: event.params["receivedBytes"].as_u64(),
                            });
                        }
                        Some("canceled") => {
                            return Err(BrowserError::Browser("the download was cancelled".into()));
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    pub async fn switch_tab(&mut self, url_contains: &str, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let targets = self.conn.send(None, "Target.getTargets", json!({})).await?;
            let found = targets["targetInfos"].as_array().and_then(|items| {
                let pages = items.iter().filter(|item| item["type"] == "page");
                let matching: Vec<&Value> = pages
                    .filter(|item| {
                        item["url"]
                            .as_str()
                            .is_some_and(|url| url.contains(url_contains))
                    })
                    .collect();
                matching
                    .iter()
                    .find(|item| item["targetId"].as_str() != Some(&self.target_id))
                    .or_else(|| matching.first())
                    .and_then(|item| item["targetId"].as_str())
                    .map(str::to_string)
            });
            if let Some(target_id) = found {
                if target_id != self.target_id {
                    self.attach_target(&target_id).await?;
                }
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(BrowserError::Timeout(format!(
                    "no open tab matches \"{url_contains}\""
                )));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    pub async fn close(mut self) {
        let _ = self
            .conn
            .send_timeout(None, "Browser.close", json!({}), Duration::from_secs(3))
            .await;
        if let Some(process) = self.process.take() {
            process.close().await;
        }
    }
}

async fn create_tab(conn: &Connection) -> Result<String> {
    let response = conn
        .send(None, "Target.createTarget", json!({ "url": "about:blank" }))
        .await?;
    response["targetId"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| BrowserError::Browser("could not open a tab".into()))
}

fn exception_text(exception: &Value) -> String {
    let text = exception["exception"]["description"]
        .as_str()
        .or(exception["text"].as_str())
        .unwrap_or("script error");
    text.lines()
        .next()
        .unwrap_or(text)
        .chars()
        .take(300)
        .collect()
}

fn center_of(rect: &Value) -> Option<(f64, f64)> {
    let x = rect.get("x")?.as_f64()?;
    let y = rect.get("y")?.as_f64()?;
    let width = rect.get("width")?.as_f64()?;
    let height = rect.get("height")?.as_f64()?;
    let (cx, cy) = (x + width / 2.0, y + height / 2.0);
    (cx >= 0.0 && cy >= 0.0 && width > 0.0 && height > 0.0).then_some((cx, cy))
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = safe.replace("..", "_");
    let safe = safe.trim().trim_start_matches('.').trim();
    let safe = if safe.is_empty() {
        "download".to_string()
    } else {
        safe.to_string()
    };
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = match safe.rfind('.') {
        Some(index) if index > 0 => (&safe[..index], &safe[index..]),
        _ => (safe.as_str(), ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){extension}")))
        .find(|path| !path.exists())
        .expect("some filename is free")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_paths_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "report.csv");
        std::fs::write(&first, b"x").unwrap();
        let second = unique_path(dir.path(), "report.csv");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().contains("report (1).csv"));
    }

    #[test]
    fn unsafe_download_names_are_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        let path = unique_path(dir.path(), "../etc/passwd");
        assert_eq!(path.parent(), Some(dir.path()));
        assert!(!path.to_string_lossy().contains(".."));
    }
}
