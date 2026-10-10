//! Tools that run inside workspace guests. They run as the workspace user, so
//! nothing here needs or gets more authority than the user already has.
//!
//! Each module keeps its decisions in pure functions and its I/O in a thin
//! `run`/`apply` layer.

pub mod browser;
pub mod claude;
pub mod credential;
pub mod git;
pub mod listeners;
pub mod pasted;
pub mod paths;
pub mod provision;
pub mod secrets;
pub mod session;
pub mod status;
