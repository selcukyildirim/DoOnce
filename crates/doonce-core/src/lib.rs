//! Core domain types for DoOnce.
//!
//! This crate is deliberately free of I/O, database and async dependencies. It
//! defines the contract shared by the server, worker, CLI and SDK generators:
//! tenants, identities, tasks, workflow blocks, runs, artifacts and the
//! validation rules that every persisted or imported document must pass.

#![forbid(unsafe_code)]

pub mod deployment;
pub mod error;
pub mod ids;
pub mod models;
pub mod risk;
pub mod template;
pub mod validation;
pub mod workflow;

pub use deployment::{DEFAULT_REGION, DeploymentMode, region_from_env};
pub use error::{CoreError, Result};
pub use ids::{is_valid_id, is_valid_name, new_id, now_rfc3339, slugify};
pub use models::*;
pub use workflow::*;
