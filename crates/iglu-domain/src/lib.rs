//! iglu's functional core.
//!
//! Everything here is a value or a pure function over values: no I/O, no
//! clocks, no randomness. The binaries gather inputs, call these functions and
//! perform the results.

mod parse;

pub mod attention;
pub mod auth;
pub mod capacity;
pub mod env;
pub mod guest;
pub mod id;
pub mod label;
pub mod lifecycle;
pub mod names;
pub mod network;
pub mod port;
pub mod preview;
pub mod repo;
pub mod secret;
pub mod signin;
pub mod terminal;
pub mod time;

pub use parse::ParseError;
