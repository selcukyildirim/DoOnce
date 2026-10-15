//! The DoOnce workflow engine. Replays workflows deterministically through a
//! [`Driver`]; no AI is involved in normal runs.

pub mod cdp;
pub mod driver;
pub mod engine;

pub use driver::{DownloadInfo, Driver, DriverError};
pub use engine::{
    ApprovalRequest, ExecEvent, FailureKind, Observer, RunOptions, RunOutcome, RunStatus, StepFailure, StepStatus,
    run,
};

/// The in-page agent shared with the browser extension (built from
/// `extension/src/page-agent`).
pub const PAGE_AGENT_JS: &str = include_str!("../assets/page-agent.js");

/// The recorder shared with the browser extension (built from
/// `extension/src/content/recorder.ts`).
pub const RECORDER_JS: &str = include_str!("../assets/recorder.js");
