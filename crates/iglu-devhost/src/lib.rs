//! A development execution host: hostd with workspaces as local processes
//! instead of Incus containers, so the whole stack runs on a laptop. It
//! isolates nothing, so it only ever serves loopback. See DEVELOPMENT.md.

pub mod config;
pub mod local;
mod processes;
mod store;
