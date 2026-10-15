//! The small surface the engine needs from a browser. Implemented by the
//! CDP driver (CLI) and by the browser extension over Native Messaging.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, thiserror::Error)]
pub enum DriverError {
    /// The page is navigating or the script context went away; retrying
    /// shortly is expected to work.
    #[error("page is not ready: {0}")]
    Transient(String),
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("browser error: {0}")]
    Browser(String),
    #[error("browser disconnected")]
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadInfo {
    pub filename: String,
    /// Where the file was saved, when the driver knows it.
    pub path: Option<PathBuf>,
    pub bytes: Option<u64>,
}

pub trait Driver: Send {
    /// Opens `url` in the current tab and waits until it has loaded.
    fn navigate(&mut self, url: &str, timeout: Duration) -> impl Future<Output = Result<(), DriverError>> + Send;

    /// Calls a read-only method of the page agent (`locate`, `check`, `ping`).
    fn query(&mut self, method: &str, args: Value) -> impl Future<Output = Result<Value, DriverError>> + Send;

    /// Runs the page agent's `perform` and then waits for any navigation the
    /// action started to finish loading.
    fn perform(&mut self, args: Value, timeout: Duration) -> impl Future<Output = Result<Value, DriverError>> + Send;

    /// Starts listening for downloads. Call before the triggering action.
    fn expect_download(&mut self) -> impl Future<Output = Result<(), DriverError>> + Send;

    /// Waits for the download announced after `expect_download` to finish.
    fn wait_download(&mut self, timeout: Duration) -> impl Future<Output = Result<DownloadInfo, DriverError>> + Send;

    /// Makes the tab whose URL contains `url_contains` the current tab.
    fn switch_tab(&mut self, url_contains: &str, timeout: Duration) -> impl Future<Output = Result<(), DriverError>> + Send;
}
