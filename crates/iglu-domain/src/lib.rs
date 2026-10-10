//! iglu's functional core.
//!
//! Everything here is a value or a pure function over values: no I/O, no
//! clocks, no randomness. The binaries gather inputs, call these functions and
//! perform the results.

mod parse;

pub mod agent;
pub mod attention;
pub mod auth;
pub mod capacity;
pub mod column;
pub mod env;
pub mod git;
pub mod guest;
pub mod id;
pub mod idle;
pub mod label;
pub mod lifecycle;
pub mod listener;
pub mod names;
pub mod network;
pub mod port;
pub mod preferences;
pub mod preview;
pub mod project;
pub mod repo;
pub mod secret;
pub mod signin;
pub mod standing;
pub mod terminal;
pub mod time;

pub use parse::ParseError;
