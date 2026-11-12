//! Browser automation primitives for DoOnce.
//!
//! The crate exposes a small CDP client, a Chrome launcher, and a
//! [`BrowserSession`] that injects the page agent into an isolated world. It is
//! deliberately independent of the AI and workflow layers so it can be tested
//! and reviewed in isolation.

#![forbid(unsafe_code)]

pub mod chrome;
pub mod connection;
pub mod error;
pub mod session;

pub use chrome::{ChromeProcess, LaunchOptions, find_chrome, resolve_ws_url};
pub use connection::{CdpEvent, Connection};
pub use error::{BrowserError, Result};
pub use session::{BrowserSession, DownloadInfo};

/// Deterministic locator/action implementation injected into pages.
pub const PAGE_AGENT_JS: &str = include_str!("../assets/page-agent.js");
