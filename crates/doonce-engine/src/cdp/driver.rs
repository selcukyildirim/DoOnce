//! [`Driver`] implementation on top of the Chrome DevTools Protocol. The page
//! agent runs in an isolated world, so page scripts cannot tamper with it.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::broadcast::{self, error::RecvError};

use super::chrome::{ChromeProcess, LaunchOptions, resolve_ws_url};
use super::connection::{CdpEvent, Connection};
use crate::PAGE_AGENT_JS;
use crate::driver::{DownloadInfo, Driver, DriverError};

/// How long to watch for a navigation after a click or key press.
const NAVIGATION_GRACE: Duration = Duration::from_millis(300);

pub struct CdpDriver {
    conn: Connection,
    session: String,
    target_id: String,
    context_id: Option<i64>,
    download_dir: PathBuf,
    downloads: Option<broadcast::Receiver<CdpEvent>>,
    process: Option<ChromeProcess>,
}

impl CdpDriver {
    /// Starts a dedicated Chrome and drives its first tab.
    pub async fn launch(opts: &LaunchOptions, download_dir: &Path) -> Result<Self, DriverError> {
        let process = ChromeProcess::launch(opts).await?;
        let conn = Connection::connect(&process.ws_url).await?;
        let targets = conn.send(None, "Target.getTargets", json!({})).await?;
        let existing = targets["targetInfos"]
            .as_array()
            .and_then(|t| t.iter().find(|t| t["type"] == "page"))
            .and_then(|t| t["targetId"].as_str())
            .map(str::to_string);
        let target_id = match existing {
            Some(id) => id,
            None => create_tab(&conn).await?,
        };
        let mut driver = Self::attach(conn, target_id, download_dir).await?;
        driver.process = Some(process);
        Ok(driver)
    }

    /// Opens a new tab in a Chrome that is already running with
    /// `--remote-debugging-port`.
    pub async fn connect(endpoint: &str, download_dir: &Path) -> Result<Self, DriverError> {
        let ws = resolve_ws_url(endpoint).await?;
        let conn = Connection::connect(&ws).await?;
        let target_id = create_tab(&conn).await?;
        Self::attach(conn, target_id, download_dir).await
    }

    async fn attach(conn: Connection, target_id: String, download_dir: &Path) -> Result<Self, DriverError> {
        std::fs::create_dir_all(download_dir)
            .map_err(|e| DriverError::Browser(format!("cannot create download directory: {e}")))?;
        conn.send(
            None,
            "Browser.setDownloadBehavior",
            json!({ "behavior": "allowAndName", "downloadPath": download_dir, "eventsEnabled": true }),
        )
        .await?;
        let mut driver = CdpDriver {
            conn,
            session: String::new(),
            target_id: String::new(),
            context_id: None,
            download_dir: download_dir.to_path_buf(),
            downloads: None,
            process: None,
        };
        driver.attach_target(&target_id).await?;
        Ok(driver)
    }

    async fn attach_target(&mut self, target_id: &str) -> Result<(), DriverError> {
        let r = self.conn.send(None, "Target.attachToTarget", json!({ "targetId": target_id, "flatten": true })).await?;
        self.session = r["sessionId"].as_str().ok_or_else(|| DriverError::Browser("no session id".into()))?.into();
        self.target_id = target_id.to_string();
        self.context_id = None;
        self.conn.send(Some(&self.session), "Page.enable", json!({})).await?;
        self.conn.send(None, "Target.activateTarget", json!({ "targetId": target_id })).await?;
        Ok(())
    }

    /// The raw connection, for tests and tooling.
    pub fn connection(&self) -> (&Connection, &str) {
        (&self.conn, &self.session)
    }

    /// Evaluates `expression` in the page's main world (used by tests to
    /// inspect or prepare pages, never by workflow steps).
    pub async fn evaluate_main_world(&self, expression: &str) -> Result<Value, DriverError> {
        let r = self
            .conn
            .send(
                Some(&self.session),
                "Runtime.evaluate",
                json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(DriverError::Browser(exception_text(ex)));
        }
        Ok(r["result"]["value"].clone())
    }

    pub async fn close(self) {
        if let Some(p) = self.process {
            let _ = self.conn.send_timeout(None, "Browser.close", json!({}), Duration::from_secs(3)).await;
            p.close().await;
        } else {
            let _ = self.conn.send(None, "Target.closeTarget", json!({ "targetId": self.target_id })).await;
        }
    }

    async fn ensure_context(&mut self) -> Result<i64, DriverError> {
        if let Some(id) = self.context_id {
            return Ok(id);
        }
        let tree = self.conn.send(Some(&self.session), "Page.getFrameTree", json!({})).await?;
        let frame_id = tree["frameTree"]["frame"]["id"].as_str().unwrap_or(&self.target_id).to_string();
        let world = self
            .conn
            .send(
                Some(&self.session),
                "Page.createIsolatedWorld",
                json!({ "frameId": frame_id, "worldName": "doonce", "grantUniveralAccess": false }),
            )
            .await?;
        let id = world["executionContextId"]
            .as_i64()
            .ok_or_else(|| DriverError::Transient("no isolated world".into()))?;
        let r = self
            .conn
            .send(Some(&self.session), "Runtime.evaluate", json!({ "expression": PAGE_AGENT_JS, "contextId": id }))
            .await?;
        if let Some(ex) = r.get("exceptionDetails") {
            return Err(DriverError::Browser(format!("page agent failed to load: {}", exception_text(ex))));
        }
        self.context_id = Some(id);
        Ok(id)
    }

    async fn call(&mut self, method: &str, args: &Value) -> Result<Value, DriverError> {
        let expression = format!("globalThis.__doonce.call({}, {})", Value::from(method), args);
        for attempt in 0..2 {
            let context = self.ensure_context().await?;
            let r = self
                .conn
                .send(
                    Some(&self.session),
                    "Runtime.evaluate",
                    json!({ "expression": expression, "contextId": context, "returnByValue": true, "awaitPromise": true }),
                )
                .await;
            match r {
                // The page navigated since the world was created: recreate it.
                Err(DriverError::Transient(m)) if m.contains("Cannot find context") && attempt == 0 => {
                    self.context_id = None;
                }
                Err(e) => {
                    if matches!(e, DriverError::Transient(_)) {
                        self.context_id = None;
                    }
                    return Err(e);
                }
                Ok(r) => {
                    if let Some(ex) = r.get("exceptionDetails") {
                        let text = exception_text(ex);
                        if text.contains("__doonce") && attempt == 0 {
                            self.context_id = None;
                            continue;
                        }
                        return Err(DriverError::Browser(text));
                    }
                    return Ok(r["result"]["value"].clone());
                }
            }
        }
        Err(DriverError::Transient("page agent unavailable".into()))
    }

    /// Waits for a navigation of our main frame that started within the
    /// grace period to finish loading.
    async fn settle(&mut self, mut rx: broadcast::Receiver<CdpEvent>, timeout: Duration) -> Result<(), DriverError> {
        let grace_end = tokio::time::Instant::now() + NAVIGATION_GRACE;
        let mut loading = false;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let until = if loading { deadline } else { grace_end };
            let ev = match tokio::time::timeout_at(until, rx.recv()).await {
                Err(_) if !loading => return Ok(()),
                Err(_) => return self.accept_if_interactive().await,
                Ok(Err(RecvError::Lagged(_))) => continue,
                Ok(Err(RecvError::Closed)) => return Err(DriverError::Disconnected),
                Ok(Ok(ev)) => ev,
            };
            if ev.session_id.as_deref() != Some(&self.session) {
                continue;
            }
            let main_frame = ev.params["frameId"].as_str() == Some(&self.target_id);
            match ev.method.as_str() {
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

    async fn accept_if_interactive(&mut self) -> Result<(), DriverError> {
        let state = self.call("ping", &json!({})).await?;
        match state["readyState"].as_str() {
            Some("interactive" | "complete") => Ok(()),
            _ => Err(DriverError::Timeout("page did not finish loading".into())),
        }
    }
}

async fn create_tab(conn: &Connection) -> Result<String, DriverError> {
    let r = conn.send(None, "Target.createTarget", json!({ "url": "about:blank" })).await?;
    r["targetId"].as_str().map(str::to_string).ok_or_else(|| DriverError::Browser("could not open a tab".into()))
}

fn exception_text(ex: &Value) -> String {
    let text = ex["exception"]["description"].as_str().or(ex["text"].as_str()).unwrap_or("script error");
    text.lines().next().unwrap_or(text).chars().take(300).collect()
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' || c == ':' || c.is_control() { '_' } else { c })
        .collect();
    let safe = if safe.trim().is_empty() || safe.starts_with('.') { format!("download{safe}") } else { safe };
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match safe.rfind('.') {
        Some(i) if i > 0 => (&safe[..i], &safe[i..]),
        _ => (safe.as_str(), ""),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("some name is free")
}

impl Driver for CdpDriver {
    async fn navigate(&mut self, url: &str, timeout: Duration) -> Result<(), DriverError> {
        let mut rx = self.conn.subscribe();
        self.context_id = None;
        let r = self.conn.send_timeout(Some(&self.session), "Page.navigate", json!({ "url": url }), timeout).await?;
        if let Some(err) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(DriverError::Browser(format!("could not open the page ({err})")));
        }
        if r.get("loaderId").is_none() {
            return Ok(()); // same-document navigation
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Err(_) => return self.accept_if_interactive().await,
                Ok(Err(RecvError::Lagged(_))) => continue,
                Ok(Err(RecvError::Closed)) => return Err(DriverError::Disconnected),
                Ok(Ok(ev)) if ev.method == "Page.loadEventFired" && ev.session_id.as_deref() == Some(&self.session) => {
                    return Ok(());
                }
                Ok(Ok(_)) => {}
            }
        }
    }

    async fn query(&mut self, method: &str, args: Value) -> Result<Value, DriverError> {
        self.call(method, &args).await
    }

    async fn perform(&mut self, args: Value, timeout: Duration) -> Result<Value, DriverError> {
        let may_navigate = matches!(args["action"].as_str(), Some("click" | "press"));
        let rx = self.conn.subscribe();
        let result = match self.call("perform", &args).await {
            Ok(v) => v,
            // The action made the page navigate before it could answer.
            Err(DriverError::Transient(m)) if m.contains("destroyed") || m.contains("navigated") => {
                json!({ "ok": true, "navigated": true })
            }
            Err(e) => return Err(e),
        };
        if let Some(url) = result["openUrl"].as_str() {
            if doonce_core::validate::is_allowed_url(url) {
                self.conn.send(None, "Target.createTarget", json!({ "url": url })).await?;
            }
            return Ok(result);
        }
        if may_navigate && result["ok"] == true {
            self.settle(rx, timeout).await?;
        }
        Ok(result)
    }

    async fn expect_download(&mut self) -> Result<(), DriverError> {
        self.downloads = Some(self.conn.subscribe());
        Ok(())
    }

    async fn wait_download(&mut self, timeout: Duration) -> Result<DownloadInfo, DriverError> {
        let mut rx = self.downloads.take().ok_or_else(|| DriverError::Browser("download was not expected".into()))?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut started: Option<(String, String)> = None;
        loop {
            let ev = match tokio::time::timeout_at(deadline, rx.recv()).await {
                Err(_) => {
                    let what = if started.is_some() { "download did not finish" } else { "no download started" };
                    return Err(DriverError::Timeout(format!("{what} within {}s", timeout.as_secs())));
                }
                Ok(Err(RecvError::Lagged(_))) => continue,
                Ok(Err(RecvError::Closed)) => return Err(DriverError::Disconnected),
                Ok(Ok(ev)) => ev,
            };
            match ev.method.as_str() {
                "Browser.downloadWillBegin" if started.is_none() => {
                    let guid = ev.params["guid"].as_str().unwrap_or_default().to_string();
                    let name = ev.params["suggestedFilename"].as_str().unwrap_or("download").to_string();
                    started = Some((guid, name));
                }
                "Browser.downloadProgress" => {
                    let Some((guid, name)) = &started else { continue };
                    if ev.params["guid"].as_str() != Some(guid) {
                        continue;
                    }
                    match ev.params["state"].as_str() {
                        Some("completed") => {
                            let bytes = ev.params["receivedBytes"].as_u64();
                            let saved = self.download_dir.join(guid);
                            let path = unique_path(&self.download_dir, name);
                            let final_path = match std::fs::rename(&saved, &path) {
                                Ok(()) => Some(path),
                                Err(_) => saved.exists().then_some(saved),
                            };
                            return Ok(DownloadInfo { filename: name.clone(), path: final_path, bytes });
                        }
                        Some("canceled") => return Err(DriverError::Browser("the download was cancelled".into())),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    async fn switch_tab(&mut self, url_contains: &str, timeout: Duration) -> Result<(), DriverError> {
        let deadline = Instant::now() + timeout;
        loop {
            let targets = self.conn.send(None, "Target.getTargets", json!({})).await?;
            let found = targets["targetInfos"].as_array().and_then(|list| {
                let pages = list.iter().filter(|t| t["type"] == "page");
                let matching: Vec<&Value> =
                    pages.filter(|t| t["url"].as_str().is_some_and(|u| u.contains(url_contains))).collect();
                // Prefer a tab other than the current one (newly opened).
                matching
                    .iter()
                    .find(|t| t["targetId"].as_str() != Some(&self.target_id))
                    .or(matching.first())
                    .and_then(|t| t["targetId"].as_str())
                    .map(str::to_string)
            });
            if let Some(id) = found {
                if id != self.target_id {
                    self.attach_target(&id).await?;
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(DriverError::Timeout(format!("no open tab matches \"{url_contains}\"")));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
